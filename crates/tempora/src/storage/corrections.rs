//! Corrections: renames, category changes, grouping and their undo, plus carrying an
//! executable's identity over to the Steam game it turns out to be.

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{AppIdentity, Category, Settings};

use super::*;

impl Store {
    /// Keeps an earlier executable identity attached to the Steam install it now matches.
    pub fn migrate_steam_alias(&self, source: &str, steam: &AppIdentity) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut settings = load_settings(&tx)?;
        if copy_capture_alias(&mut settings, source, &steam.key) {
            write_settings(&tx, &settings)?;
        }

        if let Some(source_root) = effective_key(&tx, source)?
            && source_root != steam.key
        {
            let source_protected = group_has_correction(&tx, &source_root)?;
            let move_edit =
                source_root == source && source_protected && can_move_alias_edit(&tx, source)?;
            if source_root == source && (!source_protected || move_edit) {
                tx.execute(
                    "INSERT INTO apps(key, name, category, qualified) VALUES (?1, ?2, ?3, 1)
                     ON CONFLICT(key) DO UPDATE SET qualified = 1 WHERE qualified = 0",
                    params![
                        steam.key,
                        steam.name,
                        serde_json::to_string(&steam.category)?
                    ],
                )?;
                let target =
                    effective_key(&tx, &steam.key)?.ok_or(StorageError::NotFound("Steam app"))?;
                if target != source
                    && is_group_root(&tx, &target)?
                    && (!move_edit || !group_has_correction(&tx, &target)?)
                {
                    if move_edit {
                        tx.execute(
                            "UPDATE apps SET (edited_name, edited_category) = (
                                 SELECT edited_name, edited_category FROM apps WHERE key = ?2
                             ) WHERE key = ?1",
                            params![target, source],
                        )?;
                        tx.execute(
                            "UPDATE correction_undo SET app_key = ?1 WHERE app_key = ?2",
                            params![target, source],
                        )?;
                        tx.execute(
                            "UPDATE apps SET edited_name = NULL, edited_category = NULL WHERE key = ?1",
                            [source],
                        )?;
                    }
                    tx.execute(
                        "UPDATE apps SET group_key = ?1 WHERE key = ?2 OR group_key = ?2",
                        params![target, source],
                    )?;
                }
            } else if is_group_root(&tx, &source_root)? {
                // An existing user group keeps its chosen representative and undo history.
                tx.execute(
                    "INSERT INTO apps(key, name, category, qualified, group_key)
                     VALUES (?1, ?2, ?3, 1, ?4) ON CONFLICT(key) DO NOTHING",
                    params![
                        steam.key,
                        steam.name,
                        serde_json::to_string(&steam.category)?,
                        source_root
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Saves only what differs from the automatic name and category. A field set back to its
    /// automatic value loses its edit, so later recognition updates reach it again. Saving with
    /// nothing changed records no correction. Returns whether a correction was recorded, so the
    /// caller issues an undo token only for one.
    pub fn edit_app(&self, key: &str, name: &str, category: Category) -> Result<bool> {
        let name = name.trim();
        ensure(!name.is_empty(), "App name cannot be empty")?;
        let category = serde_json::to_string(&category)?;
        let tx = self.conn.unchecked_transaction()?;
        let key = effective_key(&tx, key)?.ok_or(StorageError::NotFound("App"))?;
        let (automatic_name, automatic_category, edited_name, edited_category): (
            String,
            String,
            Option<String>,
            Option<String>,
        ) = tx.query_row(
            "SELECT name, category, edited_name, edited_category FROM apps WHERE key = ?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let new_name = (name != automatic_name).then(|| name.to_owned());
        let new_category = (category != automatic_category).then_some(category);
        if new_name == edited_name && new_category == edited_category {
            return Ok(false);
        }
        new_correction(&tx, &key, false, None)?;
        tx.execute(
            "UPDATE apps SET edited_name = ?1, edited_category = ?2 WHERE key = ?3",
            params![new_name, new_category, key],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Clears the name and category edits of an app in one undoable correction.
    pub fn reset_app(&self, key: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let key = effective_key(&tx, key)?.ok_or(StorageError::NotFound("App"))?;
        let edited: bool = tx.query_row(
            "SELECT edited_name IS NOT NULL OR edited_category IS NOT NULL FROM apps WHERE key = ?1",
            [&key],
            |row| row.get(0),
        )?;
        ensure(
            edited,
            "This app already uses its automatic name and category",
        )?;
        new_correction(&tx, &key, false, None)?;
        tx.execute(
            "UPDATE apps SET edited_name = NULL, edited_category = NULL WHERE key = ?1",
            [&key],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The recognized name and category an app returns to on Reset to automatic.
    pub fn automatic_identity(&self, key: &str) -> Result<Option<(String, Category)>> {
        let Some(key) = effective_key(&self.conn, key)? else {
            return Ok(None);
        };
        let (name, category): (String, String) = self.conn.query_row(
            "SELECT name, category FROM apps WHERE key = ?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(Some((name, serde_json::from_str(&category)?)))
    }

    /// The automatic name and category stored on this key's own row. Unlike `automatic_identity`,
    /// a group member gets its own values here, so refreshing a member cannot write the
    /// representative's name onto it.
    pub fn own_automatic_identity(&self, key: &str) -> Result<Option<(String, Category)>> {
        let stored: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT name, category FROM apps WHERE key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((name, category)) = stored else {
            return Ok(None);
        };
        Ok(Some((name, serde_json::from_str(&category)?)))
    }

    pub fn group_apps(&self, source: &str, target: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let source = effective_key(&tx, source)?.ok_or(StorageError::NotFound("Source app"))?;
        let target = effective_key(&tx, target)?.ok_or(StorageError::NotFound("Target app"))?;
        ensure(
            source != target,
            "These apps already belong to the same group",
        )?;
        new_correction(&tx, &source, true, Some(&target))?;
        // A visible target can still have an unqualified legacy parent. Make the user's
        // explicit target the representative so a later qualification cannot create a chain.
        tx.execute(
            "UPDATE apps SET group_key = NULL WHERE key = ?1 AND group_key IS NOT NULL",
            [&target],
        )?;
        tx.execute(
            "UPDATE apps SET group_key = ?1 WHERE key = ?2 OR group_key = ?2",
            params![target, source],
        )?;
        // The source's members now point at the target, so this covers them too.
        tx.execute(
            "UPDATE apps SET grouping = 'user' WHERE key = ?1 OR key = ?2 OR group_key = ?1",
            params![target, source],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Groups an app with an older app that has the same fingerprint, when the evidence is
    /// strong and the user hasn't decided anything about it. Stores the fingerprint either way.
    /// Returns whether the app was grouped. No correction event is recorded: this is automatic.
    pub fn auto_group(&self, key: &str, fingerprint: Option<&str>) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE apps SET fingerprint = ?2 WHERE key = ?1",
            params![key, fingerprint],
        )?;
        let Some(fingerprint) = fingerprint else {
            tx.commit()?;
            return Ok(false);
        };
        let Some(target) = automatic_target(&tx, key, fingerprint)? else {
            tx.commit()?;
            return Ok(false);
        };
        let changed = tx.execute(
            "UPDATE apps SET group_key = ?2 WHERE key = ?1",
            params![key, target],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Checks the key and its effective group's sources against capture exclusions. When a
    /// fingerprint resolves to a prospective automatic group, that group supplies the sources.
    pub fn group_is_excluded(
        &self,
        key: &str,
        fingerprint: Option<&str>,
        settings: &Settings,
    ) -> Result<bool> {
        if settings.excluded.contains(key) {
            return Ok(true);
        }
        let target = match fingerprint {
            Some(fingerprint) => automatic_target(&self.conn, key, fingerprint)?,
            None => None,
        };
        Ok(self
            .source_keys(target.as_deref().unwrap_or(key))?
            .iter()
            .any(|source| settings.excluded.contains(source)))
    }

    /// Removes a source app from its group. The key is the source itself, not the group's
    /// representative. The user's decision sticks, and Undo restores the group.
    pub fn ungroup_app(&self, key: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let grouped: bool = tx
            .query_row(
                "SELECT group_key IS NOT NULL FROM apps WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        ensure(grouped, "This app isn't part of a group")?;
        new_correction(&tx, key, false, None)?;
        tx.execute(
            "UPDATE apps SET group_key = NULL, grouping = 'user' WHERE key = ?1",
            [key],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn undo_correction(&self) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let Some(id) = tx
            .query_row(
                "SELECT id FROM correction_events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        // Carry members added since the snapshot to the restored root. Snapshotted members
        // get their own original links back below.
        tx.execute(
            "UPDATE apps AS member SET group_key = saved.group_key
             FROM correction_undo AS saved
             WHERE saved.event_id = ?1 AND saved.group_key IS NOT NULL
               AND member.group_key = saved.app_key
               AND NOT EXISTS (SELECT 1 FROM correction_undo u
                               WHERE u.event_id = ?1 AND u.app_key = member.key)",
            [id],
        )?;
        tx.execute(
            "UPDATE apps SET (group_key, edited_name, edited_category, grouping) = (
                 SELECT group_key, edited_name, edited_category, grouping FROM correction_undo
                 WHERE event_id = ?1 AND app_key = apps.key
             ) WHERE key IN (SELECT app_key FROM correction_undo WHERE event_id = ?1)",
            [id],
        )?;
        tx.execute("DELETE FROM correction_events WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Undoable corrections, newest first. Only the first can be undone.
    pub fn corrections(&self) -> Result<Vec<CorrectionEntry>> {
        let mut query = self.conn.prepare(
            "SELECT u.event_id, COALESCE(a.edited_name, a.name) FROM correction_undo u
             JOIN apps a ON a.key = u.app_key
             WHERE u.show_in_history = 1
             ORDER BY u.event_id DESC, 2 COLLATE NOCASE",
        )?;
        let mut rows = query.query([])?;
        let mut entries: Vec<CorrectionEntry> = Vec::new();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            match entries.last_mut() {
                Some(entry) if entry.id == id => entry.apps.push(name),
                _ => entries.push(CorrectionEntry {
                    id,
                    apps: vec![name],
                }),
            }
        }
        Ok(entries)
    }
}

pub(super) fn new_correction(
    conn: &Connection,
    key: &str,
    include_members: bool,
    target: Option<&str>,
) -> Result<()> {
    conn.execute("INSERT INTO correction_events DEFAULT VALUES", [])?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO correction_undo(event_id, app_key, group_key, edited_name, edited_category, grouping, show_in_history)
         SELECT ?1, key, group_key, edited_name, edited_category, grouping,
                key = ?2 OR (?3 AND group_key IS ?2) OR (key IS ?4 AND group_key IS NOT NULL)
         FROM apps
          WHERE key = ?2 OR (?3 AND group_key = ?2) OR key = ?4 OR group_key = ?4",
        params![id, key, include_members, target],
    )?;
    Ok(())
}

pub(super) fn effective_key(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT effective_key FROM qualified_apps WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Resolves a prospective group before the source has to be written to history.
pub(super) fn automatic_target(
    conn: &Connection,
    key: &str,
    fingerprint: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT q.effective_key FROM apps a JOIN qualified_apps q ON q.key = a.key
         JOIN apps leader ON leader.key = q.effective_key
         LEFT JOIN apps source ON source.key = ?1
         WHERE a.fingerprint = ?2 AND a.key <> ?1 AND q.effective_key <> ?1
           AND leader.group_key IS NULL
           AND (source.rowid IS NULL OR a.rowid < source.rowid)
           AND NOT EXISTS (
             SELECT 1 FROM apps s WHERE s.key = ?1
               AND (s.grouping IS 'user' OR s.group_key IS NOT NULL
                 OR s.edited_name IS NOT NULL OR s.edited_category IS NOT NULL
                 OR EXISTS (SELECT 1 FROM apps m WHERE m.group_key = s.key))
           )
         ORDER BY a.rowid LIMIT 1",
            params![key, fingerprint],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn group_has_correction(conn: &Connection, root: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM apps a WHERE (a.key = ?1 OR a.group_key = ?1)
               AND (a.grouping = 'user' OR a.edited_name IS NOT NULL OR a.edited_category IS NOT NULL
                 OR EXISTS (SELECT 1 FROM correction_undo u WHERE u.app_key = a.key OR u.group_key = a.key))
         )",
        [root],
        |row| row.get(0),
    )?)
}

pub(super) fn can_move_alias_edit(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT group_key IS NULL AND grouping IS NULL
             AND NOT EXISTS (SELECT 1 FROM apps m WHERE m.group_key = apps.key)
             AND NOT EXISTS (SELECT 1 FROM correction_undo u WHERE u.group_key = apps.key
                  OR (u.app_key = apps.key AND (u.group_key IS NOT NULL OR u.grouping IS NOT NULL)))
         FROM apps WHERE key = ?1",
        [key],
        |row| row.get(0),
    )?)
}

pub(super) fn is_group_root(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT group_key IS NULL FROM apps WHERE key = ?1",
        [key],
        |row| row.get(0),
    )?)
}

pub(super) fn copy_capture_alias(settings: &mut Settings, source: &str, target: &str) -> bool {
    let mut changed = false;
    for keys in [&mut settings.excluded, &mut settings.selected] {
        if keys.contains(source) {
            changed |= keys.insert(target.to_owned());
        }
    }
    changed
}

/// One undoable correction, newest first in `Store::corrections`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrectionEntry {
    pub id: i64,
    /// Display names of the entries the correction changed.
    pub apps: Vec<String>,
}

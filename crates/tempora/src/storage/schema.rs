//! Schema migrations and the settings row.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{AppIdentity, Settings};
use crate::recognition::known_product_for_key;

use super::*;

pub(super) fn migrate_qualification(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        conn.execute_batch("ALTER TABLE apps ADD COLUMN qualified INTEGER NOT NULL DEFAULT 0")?;
        // A legacy foreground sighting is evidence of a window. Preserve other observations,
        // but keep their identities hidden until the recorder subsequently qualifies them.
        conn.execute(
            "UPDATE apps SET qualified = 1 WHERE EXISTS
                (SELECT 1 FROM intervals WHERE app_key = apps.key AND kind = ?1)",
            [FOCUS],
        )?;
        conn.execute_batch(
            "CREATE VIEW qualified_apps AS
             SELECT s.key, COALESCE(g.key, s.key) AS effective_key
             FROM apps s LEFT JOIN apps g ON g.key = s.group_key AND g.qualified = 1
             WHERE s.qualified = 1;
             PRAGMA user_version = 1;",
        )?;
    }
    Ok(())
}

pub(super) fn load_settings(conn: &Connection) -> Result<Settings> {
    let json: Option<String> = conn
        .query_row("SELECT settings FROM config WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(json
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default())
}

pub(super) fn write_settings(conn: &Connection, settings: &Settings) -> Result<()> {
    conn.execute(
        "INSERT INTO config(id, settings) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET settings = excluded.settings",
        [serde_json::to_string(settings)?],
    )?;
    Ok(())
}

pub(super) fn migrate_product_aliases(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 1 {
        return Ok(());
    }

    let mut settings = load_settings(conn)?;
    let previous = settings.clone();
    for keys in [&mut settings.excluded, &mut settings.selected] {
        let canonical: Vec<_> = keys
            .iter()
            .filter_map(|key| known_product_for_key(key).map(|app| app.key))
            .collect();
        keys.extend(canonical);
    }
    if settings != previous {
        write_settings(conn, &settings)?;
    }

    // Keep user edits, explicit groups, and every identity referenced by an undo step.
    let mut blocked = HashSet::new();
    let mut aliases = BTreeMap::<String, (AppIdentity, Vec<String>)>::new();
    {
        let mut query = conn.prepare(
            "SELECT a.key, a.qualified, (a.edited_name IS NOT NULL OR a.edited_category IS NOT NULL
                        OR a.group_key IS NOT NULL OR EXISTS (SELECT 1 FROM apps WHERE group_key = a.key)
                        OR EXISTS (SELECT 1 FROM correction_undo WHERE app_key = a.key OR group_key = a.key))
             FROM apps a",
        )?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            if row.get::<_, bool>(2)? {
                blocked.insert(key);
            } else if row.get::<_, bool>(1)?
                && let Some(product) = known_product_for_key(&key)
            {
                aliases
                    .entry(product.key.clone())
                    .or_insert_with(|| (product, Vec::new()))
                    .1
                    .push(key);
            }
        }
    }
    for (key, (product, sources)) in aliases {
        if blocked.contains(&key) {
            continue;
        }
        conn.execute(
            "INSERT INTO apps(key, name, category, qualified) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(key) DO UPDATE SET qualified = 1 WHERE qualified = 0",
            params![key, product.name, serde_json::to_string(&product.category)?],
        )?;
        for source in sources {
            conn.execute(
                "UPDATE apps SET group_key = ?1 WHERE key = ?2",
                params![key, source],
            )?;
        }
    }
    conn.pragma_update(None, "user_version", 2)?;
    Ok(())
}

/// Adds fullscreen evidence, fingerprints and the grouping decision. Existing user groups are
/// marked as the user's decisions so automatic grouping leaves them alone.
pub(super) fn migrate_v3(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 2 {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE apps ADD COLUMN fullscreen_seen INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE apps ADD COLUMN fingerprint TEXT;
         ALTER TABLE apps ADD COLUMN grouping TEXT;
         ALTER TABLE correction_undo ADD COLUMN grouping TEXT;
         CREATE INDEX apps_fingerprint ON apps(fingerprint);
         UPDATE apps SET grouping = 'user' WHERE group_key IS NOT NULL;
         PRAGMA user_version = 3;",
    )?;
    Ok(())
}

pub(super) fn migrate_v4(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 3 {
        conn.execute_batch(
            "ALTER TABLE correction_undo ADD COLUMN show_in_history INTEGER NOT NULL DEFAULT 1;
             PRAGMA user_version = 4;",
        )?;
    }
    Ok(())
}

/// Edits used to save the name and category together. Drops the half that only repeats the
/// automatic value, so recognition updates reach it again. Saved undo states get the same
/// treatment, or undoing an earlier edit would bring a repeated value back.
///
/// A saved undo state holds no record of the automatic values when it was taken, so it is
/// compared with the current ones: a repeat whose automatic value has changed since stays.
pub(super) fn migrate_v5(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 4 {
        conn.execute_batch(
            "UPDATE apps SET edited_name = NULL WHERE edited_name = name;
             UPDATE apps SET edited_category = NULL WHERE edited_category = category;
             UPDATE correction_undo SET edited_name = NULL
              WHERE edited_name = (SELECT name FROM apps WHERE key = correction_undo.app_key);
             UPDATE correction_undo SET edited_category = NULL
              WHERE edited_category = (SELECT category FROM apps WHERE key = correction_undo.app_key);",
        )?;
        conn.pragma_update(None, "user_version", 5)?;
    }
    Ok(())
}

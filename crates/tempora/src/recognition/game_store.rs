//! Lookup in Windows' own game list (`HKCU\System\GameConfigStore`).
//!
//! Windows keeps one `Parents\<hash>` key per exe file name, where `<hash>` is the SHA-1 of the
//! lowercased file name in UTF-16LE. Its `Children` value names the entries that Windows
//! recognized for that file name. Only the current user's hive is read, and every failure means
//! "no entry".

use std::fmt::Write as _;
use std::path::Path;

use sha1::{Digest, Sha1};

/// One child entry of the game list, with the values this module reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEntry {
    /// `Type`: 1 is a game, 2 is a packaged non-game app.
    pub kind: u32,
    pub title_id: Option<String>,
    /// `ExeParentDirectory`: a folder hint that may be only part of the folder's name.
    pub parent_directory: Option<String>,
    /// `MatchedExeFullPath`: the exact path Windows saw the game run from.
    pub matched_path: Option<String>,
}

pub trait GameEntries {
    /// All child entries for a lowercased exe file name; empty when none or unreadable.
    fn entries(&self, file_name_lower: &str) -> Vec<GameEntry>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameMatch {
    pub title_id: Option<String>,
    /// The real ancestor folder name the hint matched, e.g. "War Thunder" for hint "Thunder".
    pub folder_name: Option<String>,
}

/// Lowercase hex SHA-1 of the UTF-16LE bytes of the lowercased file name.
pub fn parent_hash(file_name_lower: &str) -> String {
    let bytes: Vec<u8> = file_name_lower
        .to_lowercase()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    Sha1::digest(bytes)
        .iter()
        .fold(String::with_capacity(40), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Returns the first entry that accepts `path`.
pub fn match_game(path: &Path, entries: &[GameEntry]) -> Option<GameMatch> {
    entries.iter().find_map(|entry| match_entry(path, entry))
}

fn match_entry(path: &Path, entry: &GameEntry) -> Option<GameMatch> {
    if entry.kind != 1 {
        return None;
    }
    let title_id = present(entry.title_id.as_deref()).map(str::to_owned);
    let hint = present(entry.parent_directory.as_deref());
    let matched = present(entry.matched_path.as_deref());
    let folder_name = hint.and_then(|hint| folder_for_hint(path, hint));
    let accepted = match (matched, hint) {
        (Some(matched), _) if same_path(matched, path) => true,
        (_, Some(_)) => folder_name.is_some(),
        (None, None) => title_id.is_some(),
        (Some(_), None) => false,
    };
    accepted.then_some(GameMatch {
        title_id,
        folder_name,
    })
}

/// An empty or blank registry string means the value is absent.
fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Names of every folder above the exe, nearest first. The file name and the drive or root
/// prefix are not folders.
fn ancestor_folders(path: &Path) -> impl Iterator<Item = &str> {
    path.ancestors()
        .skip(1)
        .filter_map(|parent| parent.file_name()?.to_str())
}

fn folder_for_hint(path: &Path, hint: &str) -> Option<String> {
    let hint = hint.to_lowercase();
    ancestor_folders(path)
        .find(|folder| folder.to_lowercase().contains(&hint))
        .map(str::to_owned)
}

fn same_path(registered: &str, path: &Path) -> bool {
    normalized_path(registered) == normalized_path(&path.to_string_lossy())
}

fn normalized_path(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

#[cfg(windows)]
pub struct RegistryGameEntries;

#[cfg(windows)]
impl GameEntries for RegistryGameEntries {
    fn entries(&self, file_name_lower: &str) -> Vec<GameEntry> {
        let hash = parent_hash(file_name_lower);
        let Some(parents) =
            super::registry::Key::open(&format!(r"System\GameConfigStore\Parents\{hash}"))
        else {
            return Vec::new();
        };
        let children = parents.multi_sz("Children").unwrap_or_default();
        children
            .iter()
            .filter(|guid| !guid.contains('\\'))
            .filter_map(|guid| {
                let child = super::registry::Key::open(&format!(
                    r"System\GameConfigStore\Children\{guid}"
                ))?;
                Some(GameEntry {
                    kind: child.dword("Type")?,
                    title_id: child.string("TitleId"),
                    parent_directory: child.string("ExeParentDirectory"),
                    matched_path: child.string("MatchedExeFullPath"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct FakeEntries(HashMap<String, Vec<GameEntry>>);

    impl GameEntries for FakeEntries {
        fn entries(&self, file_name_lower: &str) -> Vec<GameEntry> {
            self.0.get(file_name_lower).cloned().unwrap_or_default()
        }
    }

    fn game(title_id: Option<&str>, parent: Option<&str>, matched: Option<&str>) -> GameEntry {
        GameEntry {
            kind: 1,
            title_id: title_id.map(str::to_owned),
            parent_directory: parent.map(str::to_owned),
            matched_path: matched.map(str::to_owned),
        }
    }

    fn accepted(path: &str, entries: &[GameEntry]) -> Option<GameMatch> {
        match_game(Path::new(path), entries)
    }

    #[test]
    fn parent_hash_is_sha1_of_the_lowercased_utf16le_name() {
        let expected = {
            let bytes: Vec<u8> = "rdr2.exe"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            Sha1::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        assert_eq!(parent_hash("rdr2.exe"), expected);
        assert_eq!(
            parent_hash("rdr2.exe"),
            "eaf981df7239cf1f71d28fd46313a388d83f6d1d"
        );
        assert_eq!(parent_hash("RDR2.EXE"), parent_hash("rdr2.exe"));
        assert_eq!(parent_hash("rdr2.exe").len(), 40);
    }

    #[test]
    fn a_type_two_entry_is_rejected() {
        let mut entry = game(Some("1234"), Some("Some App"), None);
        entry.kind = 2;
        assert_eq!(accepted(r"D:/Apps/Some App/app.exe", &[entry]), None);
    }

    #[test]
    fn folder_hint_matches_a_distant_ancestor() {
        let entry = game(None, Some("Deep Rock Galactic"), None);
        let found = accepted(
            "D:/Games/Deep Rock Galactic/FSD/Binaries/Win64/FSD-Win64-Shipping.exe",
            &[entry],
        );
        assert_eq!(
            found,
            Some(GameMatch {
                title_id: None,
                folder_name: Some("Deep Rock Galactic".into()),
            })
        );
    }

    #[test]
    fn folder_hint_matches_a_partial_folder_name_and_reports_the_real_name() {
        let entry = game(None, Some("Thunder"), None);
        let found = accepted("E:/Games/War Thunder/aces.exe", &[entry]).unwrap();
        assert_eq!(found.folder_name.as_deref(), Some("War Thunder"));
    }

    #[test]
    fn folder_hint_is_case_insensitive() {
        let entry = game(None, Some("counter-strike global offensive"), None);
        assert!(
            accepted(
                "F:/Steam/Counter-Strike Global Offensive/csgo.exe",
                &[entry]
            )
            .is_some()
        );
    }

    #[test]
    fn non_ascii_folder_hint_matches_with_unicode_lowercase() {
        let entry = game(None, Some("DRAGON QUEST HEROES™ II"), None);
        let found = accepted(
            "D:/Games/Dragon Quest Heroes™ II/DQH2.exe",
            std::slice::from_ref(&entry),
        )
        .unwrap();
        assert_eq!(
            found.folder_name.as_deref(),
            Some("Dragon Quest Heroes™ II")
        );
        assert_eq!(
            accepted("D:/Games/Dragon Quest Heroes II/DQH2.exe", &[entry]),
            None
        );
    }

    #[test]
    fn filename_only_entry_needs_a_title_id() {
        let without = game(None, None, None);
        assert_eq!(accepted("C:/Tools/engine.exe", &[without]), None);
        let with = game(Some("1234567"), None, None);
        assert_eq!(
            accepted("C:/Tools/engine.exe", std::slice::from_ref(&with)),
            Some(GameMatch {
                title_id: Some("1234567".into()),
                folder_name: None,
            })
        );
    }

    #[test]
    fn matched_path_at_another_location_needs_a_matching_folder_hint() {
        let elsewhere = r"D:\Installs\Other\Game.exe";
        let no_hint = game(Some("42"), None, Some(elsewhere));
        assert_eq!(accepted("C:/Games/Game/Game.exe", &[no_hint]), None);

        let with_hint = game(Some("42"), Some("Game"), Some(elsewhere));
        let found = accepted("C:/Games/Game/Game.exe", &[with_hint]).unwrap();
        assert_eq!(found.folder_name.as_deref(), Some("Game"));
    }

    #[test]
    fn matched_path_equal_to_the_observed_path_is_accepted_case_insensitively() {
        let entry = game(None, None, Some(r"C:\Games\Pad Tool\PadTool.exe"));
        assert!(accepted("c:/games/pad tool/padtool.EXE", &[entry]).is_some());
    }

    #[test]
    fn the_first_accepting_entry_wins_and_blank_hints_count_as_absent() {
        let rejected = game(None, Some("   "), Some(r"D:\Elsewhere\x.exe"));
        let accepted_entry = game(Some("99"), Some(""), None);
        let found = accepted("C:/Games/Thing/thing.exe", &[rejected, accepted_entry]).unwrap();
        assert_eq!(found.title_id.as_deref(), Some("99"));
    }

    #[test]
    fn fake_source_feeds_the_matcher_by_hashed_name_key() {
        let mut map = HashMap::new();
        map.insert("hades.exe".to_owned(), vec![game(Some("7"), None, None)]);
        let source = FakeEntries(map);
        let entries = source.entries("hades.exe");
        assert!(accepted("D:/Epic/Hades/Hades.exe", &entries).is_some());
        assert!(source.entries("unknown.exe").is_empty());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "reads the real per-user game list"]
    fn registry_lookup_returns_without_panicking() {
        let _ = RegistryGameEntries.entries("rdr2.exe");
    }
}

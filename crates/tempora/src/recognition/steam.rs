#[cfg(windows)]
use crate::local_fs as fs;
use std::collections::HashMap;
#[cfg(not(windows))]
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::model::{AppIdentity, AppKey, Category};

#[derive(Default)]
pub(super) struct Libraries {
    by_path: HashMap<PathBuf, Library>,
}

struct Library {
    scanned: Instant,
    modified: Option<SystemTime>,
    games: Vec<Game>,
}

struct Game {
    id: u32,
    name: String,
    directory: String,
}

impl Libraries {
    pub(super) fn identify(&mut self, path: &Path) -> Option<AppIdentity> {
        let stem = path.file_stem()?.to_str()?;
        let extension = path.extension()?.to_str()?;
        if !extension.eq_ignore_ascii_case("exe") {
            return None;
        }
        let (steamapps, install_dir) = library_for(path)?;
        let modified = fs::metadata(steamapps)
            .and_then(|metadata| metadata.modified())
            .ok();
        let library = self
            .by_path
            .entry(steamapps.to_owned())
            .or_insert_with(|| scan(steamapps, modified));
        if modified != library.modified || library.scanned.elapsed() >= Duration::from_secs(30) {
            *library = scan(steamapps, modified);
        }
        let mut matches = library
            .games
            .iter()
            .filter(|game| game.directory.eq_ignore_ascii_case(install_dir));
        let game = matches.next()?;
        if matches.next().is_some() || !is_game_executable(stem) || in_support_folder(path) {
            return None;
        }
        Some(AppIdentity {
            key: AppKey::Steam(&game.id.to_string()).to_string(),
            name: game.name.clone(),
            category: Category::Games,
        })
    }
}

fn library_for(path: &Path) -> Option<(&Path, &str)> {
    // Every configured Steam library, including other drives, keeps its own
    // manifests next to common. Start with the observed path to follow moves.
    for install in path.ancestors().skip(1) {
        let common = install.parent()?;
        let steamapps = common.parent()?;
        let common_name = common.file_name().and_then(|name| name.to_str());
        let steamapps_name = steamapps.file_name().and_then(|name| name.to_str());
        if common_name.is_some_and(|name| name.eq_ignore_ascii_case("common"))
            && steamapps_name.is_some_and(|name| name.eq_ignore_ascii_case("steamapps"))
        {
            return Some((steamapps, install.file_name()?.to_str()?));
        }
    }
    None
}

fn scan(steamapps: &Path, modified: Option<SystemTime>) -> Library {
    let games = fs::read_dir(steamapps)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let id = file_name
                .to_str()?
                .strip_prefix("appmanifest_")?
                .strip_suffix(".acf")?
                .parse::<u32>()
                .ok()?;
            let contents = fs::read_to_string(entry.path()).ok()?;
            read_manifest(id, &contents)
        })
        .collect();
    Library {
        scanned: Instant::now(),
        modified,
        games,
    }
}

/// Any executable inside a Steam game's install folder belongs to that game (`cs2.exe` is
/// Counter-Strike 2, `TslGame.exe` is PUBG), except the tools around it: launchers,
/// installers, updaters, crash reporters and uninstallers never extend a game's session.
fn is_game_executable(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    let tool_word = lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| {
            matches!(
                word,
                "update" | "helper" | "server" | "config" | "settings" | "protected"
            )
        });
    // Fragments that also appear glued to other words: AGameLauncher, DXSETUP, vcredist_x64,
    // unins000, UnityCrashHandler64, CrashReportClient.
    let tool_fragment = [
        "launcher",
        "bootstrap",
        "updater",
        "setup",
        "install",
        "redist",
        "unins",
        "crash",
        "service",
    ]
    .iter()
    .any(|fragment| lower.contains(fragment));
    !lower.is_empty() && !tool_word && !tool_fragment
}

/// Redistributables and anti-cheat services ship inside a game's folder in subfolders with
/// these structural names.
fn in_support_folder(path: &Path) -> bool {
    path.ancestors().skip(1).any(|folder| {
        folder
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "_commonredist"
                        | "commonredist"
                        | "redist"
                        | "redistributables"
                        | "__installer"
                        | "installer"
                        | "installers"
                        | "directx"
                        | "vcredist"
                        | "easyanticheat"
                        | "battleye"
                )
            })
    })
}

fn read_manifest(id: u32, contents: &str) -> Option<Game> {
    let mut tokens = Tokens(contents.trim_start_matches('\u{feff}'));
    let Token::Text(heading) = tokens.next()? else {
        return None;
    };
    if !heading.eq_ignore_ascii_case("AppState") || tokens.next()? != Token::Open {
        return None;
    }
    let (mut app_id, mut name, mut directory) = (None, None, None);
    loop {
        let key = match tokens.next()? {
            Token::Close => break,
            Token::Text(key) => key,
            Token::Open => return None,
        };
        let value = match tokens.next()? {
            Token::Text(value) => value,
            Token::Open => {
                let mut depth = 1usize;
                while depth != 0 {
                    match tokens.next()? {
                        Token::Open => depth += 1,
                        Token::Close => depth -= 1,
                        Token::Text(_) => (),
                    }
                }
                continue;
            }
            Token::Close => return None,
        };
        match key.to_ascii_lowercase().as_str() {
            "appid" => app_id = value.parse::<u32>().ok(),
            "name" => name = Some(value),
            "installdir" => directory = Some(value),
            _ => (),
        }
    }
    let (name, directory) = (name?, directory?);
    (id != 0 && app_id == Some(id) && !name.trim().is_empty() && !directory.trim().is_empty())
        .then_some(Game {
            id,
            name,
            directory,
        })
}

#[derive(PartialEq)]
enum Token {
    Open,
    Close,
    Text(String),
}

struct Tokens<'a>(&'a str);

impl Iterator for Tokens<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        loop {
            self.0 = self.0.trim_start();
            if !self.0.starts_with("//") {
                break;
            }
            self.0 = self.0.find('\n').map_or("", |index| &self.0[index..]);
        }
        let mut chars = self.0.chars();
        match chars.next()? {
            '{' => {
                self.0 = chars.as_str();
                Some(Token::Open)
            }
            '}' => {
                self.0 = chars.as_str();
                Some(Token::Close)
            }
            '"' => {
                let mut value = String::new();
                let mut chars = chars.as_str().char_indices().peekable();
                while let Some((index, character)) = chars.next() {
                    match character {
                        '"' => {
                            self.0 = &self.0[index + 2..];
                            return Some(Token::Text(value));
                        }
                        '\\' if matches!(chars.peek(), Some((_, '"' | '\\'))) => {
                            value.push(chars.next()?.1);
                        }
                        _ => value.push(character),
                    }
                }
                None
            }
            _ => {
                let end = self
                    .0
                    .find(|character: char| {
                        character.is_whitespace() || matches!(character, '{' | '}' | '"')
                    })
                    .unwrap_or(self.0.len());
                let (value, rest) = self.0.split_at(end);
                self.0 = rest;
                Some(Token::Text(value.to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn manifest_reads_app_state_and_skips_nested_data() {
        let contents = r#"
            // Steam metadata
            "AppState"
            {
                "appid" "123"
                "name" "A Game: \"Special\""
                "installdir" "A Game"
                "InstalledDepots" { "1" { "name" "Not the game" } }
            }
        "#;
        let game = read_manifest(123, contents).unwrap();
        assert_eq!(game.name, "A Game: \"Special\"");
        assert_eq!(game.directory, "A Game");
        assert!(read_manifest(999, contents).is_none());
        assert!(read_manifest(123, contents.trim_end().trim_end_matches('}')).is_none());
    }

    #[test]
    fn recognizes_multiple_libraries_and_keeps_the_id_after_a_move() {
        let directory = TestDirectory::new();
        let library_a = directory.0.join("primary/steamapps");
        let library_b = directory.0.join("secondary/steamapps");
        fs::create_dir_all(&library_a).unwrap();
        fs::create_dir_all(&library_b).unwrap();
        let first_manifest = library_a.join("appmanifest_123.acf");
        let second_manifest = library_b.join("appmanifest_123.acf");
        fs::write(
            &first_manifest,
            r#""AppState" { "appid" "123" "name" "A Game" "installdir" "A Game" }"#,
        )
        .unwrap();
        fs::write(
            library_b.join("appmanifest_456.acf"),
            r#""AppState" { "appid" "456" "name" "Other Game" "installdir" "Other Game" }"#,
        )
        .unwrap();
        let first_path = library_a.join("common/A Game/Binaries/Win64/AGame-Win64-Shipping.exe");
        let mut libraries = Libraries::default();
        let game = libraries.identify(&first_path).unwrap();
        assert_eq!(game.key, "steam:123");
        assert_eq!(game.name, "A Game");
        assert_eq!(
            libraries
                .identify(&library_b.join("common/Other Game/OtherGame.exe"))
                .unwrap()
                .key,
            "steam:456"
        );
        assert!(
            libraries
                .identify(&first_path.with_file_name("AGameLauncher.exe"))
                .is_none()
        );
        // An executable named unlike its game (cs2.exe for Counter-Strike 2) is still it.
        assert_eq!(
            libraries
                .identify(&first_path.with_file_name("cs2.exe"))
                .map(|app| app.key),
            Some("steam:123".into())
        );
        // Tools and support files in the game's folder never count as the game.
        for tool in [
            "common/A Game/unins000.exe",
            "common/A Game/UnityCrashHandler64.exe",
            "common/A Game/start_protected_game.exe",
            "common/A Game/_CommonRedist/DirectX/DXSETUP.exe",
            "common/A Game/EasyAntiCheat/EasyAntiCheat_EOS_Setup.exe",
            "common/A Game/Binaries/Win64/BEService.exe",
        ] {
            assert!(
                libraries.identify(&library_a.join(tool)).is_none(),
                "{tool}"
            );
        }

        // Library B was just scanned. Windows file times advance in clock ticks of about
        // 15 ms, so wait past one tick for the move to change its modified time.
        std::thread::sleep(Duration::from_millis(40));
        fs::rename(first_manifest, second_manifest).unwrap();
        let moved_path = library_b.join(first_path.strip_prefix(library_a).unwrap());
        assert_eq!(libraries.identify(&moved_path), Some(game));
        assert!(libraries.identify(&first_path).is_none());
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("gta-steam-{}-{stamp}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

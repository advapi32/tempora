//! Display names for ordinary executables, Store apps and games.

use super::{GameMatch, VersionInfo};
use std::path::{Component, Path};

/// A friendly name for an ordinary executable from its version resource. The file description
/// comes first, then the product name, the opposite of games: Windows tools carry a generic
/// product name such as "Microsoft Windows Operating System". Names that say nothing are
/// skipped. A name that repeats the executable stem can still supply its capitalization.
pub(super) fn executable_name(version: Option<&VersionInfo>) -> Option<String> {
    [
        version.and_then(|version| version.description.as_deref()),
        version.and_then(|version| version.product.as_deref()),
    ]
    .into_iter()
    .flatten()
    .filter_map(clean_name)
    .find(|name| {
        let lower = name.to_lowercase();
        !(lower.contains("operating system")
            || lower == "electron"
            || lower.starts_with("java(tm)")
            || lower.starts_with("java (")
            || name.chars().count() > MAX_NAME_CHARS)
    })
}

/// A shell display name that can stand in for an app's name, or `None` when it is empty or too
/// long to be a name.
pub(super) fn package_display_name(name: &str) -> Option<String> {
    clean_name(name).filter(|name| name.chars().count() <= MAX_NAME_CHARS)
}

/// Maximum accepted length for version and shell display names.
const MAX_NAME_CHARS: usize = 60;

/// The best name for a game: the first usable of the folder hint, the Unreal project layout,
/// the version product, the version description, the parent folder, and the exe stem.
pub(super) fn game_name(
    path: &Path,
    game: &GameMatch,
    version: Option<&VersionInfo>,
    exe_name: &str,
) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_lowercase();
    // A version name that only repeats the exe stem says nothing new.
    let usable = |name: Option<&str>| {
        name.and_then(clean_name)
            .filter(|name| name.to_lowercase() != stem)
    };

    game.folder_name
        .as_deref()
        .and_then(clean_name)
        .or_else(|| unreal_project_name(path))
        .or_else(|| usable(version.and_then(|v| v.product.as_deref())))
        .or_else(|| usable(version.and_then(|v| v.description.as_deref())))
        .or_else(|| {
            path.parent()
                .and_then(|folder| folder.file_name())
                .and_then(|name| name.to_str())
                .filter(|name| !is_generic_folder(name))
                .and_then(clean_name)
        })
        .unwrap_or_else(|| exe_name.to_owned())
}

/// For `...\<Project>\<Sub>\Binaries\Win64\x.exe`, the name of the folder above the project
/// folder `<Sub>`. Unreal Engine games use this layout.
fn unreal_project_name(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    parts.windows(2).enumerate().find_map(|(index, pair)| {
        let is_binaries = pair[0].to_lowercase() == "binaries";
        let is_platform = matches!(pair[1].to_lowercase().as_str(), "win64" | "win32");
        if !(is_binaries && is_platform) || index < 2 {
            return None;
        }
        let name = parts[index - 2];
        if is_generic_folder(name) {
            return None;
        }
        clean_name(name)
    })
}

/// Structural folder names that say nothing about the game, including build-variant folders
/// such as `x64_dx12` or `Win64-Shipping`.
fn is_generic_folder(name: &str) -> bool {
    let name = name.to_lowercase();
    if ["x64", "x86", "win64", "win32"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return true;
    }
    matches!(
        name.as_str(),
        "bin"
            | "binaries"
            | "win64"
            | "win32"
            | "x64"
            | "x86"
            | "game"
            | "client"
            | "retail"
            | "live"
            | "common"
            | "steamapps"
            | "games"
            | "program files"
            | "program files (x86)"
    )
}

/// Trims whitespace and trailing trademark signs, however they're interleaved. Empty is `None`.
fn clean_name(name: &str) -> Option<String> {
    let cleaned = name
        .trim()
        .trim_end_matches(|c: char| matches!(c, '™' | '®' | '©') || c.is_whitespace());
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

#[cfg(test)]
mod tests {
    use super::clean_name;

    #[test]
    fn trailing_trademarks_can_be_interleaved_with_spaces() {
        assert_eq!(clean_name("Game ™ ®").as_deref(), Some("Game"));
    }
}

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub enum Category {
    Games,
    Browsers,
    Communication,
    Media,
    Productivity,
    #[default]
    Other,
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::Games,
        Self::Browsers,
        Self::Communication,
        Self::Media,
        Self::Productivity,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Games => "Games",
            Self::Browsers => "Browsers",
            Self::Communication => "Communication",
            Self::Media => "Media",
            Self::Productivity => "Productivity",
            Self::Other => "Other",
        }
    }
}

/// The parsed form of an app key. Keys are stored as text, `kind:value`:
///
/// - `exe:<lowercased full path>` for an executable;
/// - `steam:<app ID>` for a Steam game;
/// - `riot:<id>` for a Riot game or client;
/// - `product:<name>` for a product recognized from its install location;
/// - `package:<AUMID or package family>` for a Store app. A process inside a package is
///   tracked as `package:<…>:<file.exe>` before it is resolved to the app.
///
/// Parsing borrows from the key, and `Display` writes it back, so `AppKey::Exe(path).to_string()`
/// builds a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKey<'a> {
    Exe(&'a str),
    Steam(&'a str),
    Riot(&'a str),
    Product(&'a str),
    Package(&'a str),
    /// A key of a kind this version doesn't know.
    Unknown(&'a str),
}

impl<'a> AppKey<'a> {
    pub fn parse(key: &'a str) -> Self {
        match key.split_once(':') {
            Some(("exe", path)) => Self::Exe(path),
            Some(("steam", id)) => Self::Steam(id),
            Some(("riot", id)) => Self::Riot(id),
            Some(("product", name)) => Self::Product(name),
            Some(("package", package)) => Self::Package(package),
            _ => Self::Unknown(key),
        }
    }

    /// The executable's lowercased path, for an `exe:` key.
    pub fn executable(self) -> Option<&'a str> {
        match self {
            Self::Exe(path) => Some(path),
            _ => None,
        }
    }

    /// The AUMID or package family, for a `package:` key.
    pub fn package(self) -> Option<&'a str> {
        match self {
            Self::Package(package) => Some(package),
            _ => None,
        }
    }

    pub fn steam_id(self) -> Option<&'a str> {
        match self {
            Self::Steam(id) => Some(id),
            _ => None,
        }
    }

    pub fn riot_id(self) -> Option<&'a str> {
        match self {
            Self::Riot(id) => Some(id),
            _ => None,
        }
    }

    pub fn product(self) -> Option<&'a str> {
        match self {
            Self::Product(name) => Some(name),
            _ => None,
        }
    }

    pub fn is_executable(self) -> bool {
        matches!(self, Self::Exe(_))
    }

    pub fn is_steam(self) -> bool {
        matches!(self, Self::Steam(_))
    }

    /// Whether the key names exactly one app. A product, or a Store app by its AUMID
    /// (`family!app`), does; a package family can hold several apps.
    pub fn is_single_app(self) -> bool {
        match self {
            Self::Product(_) => true,
            Self::Package(package) => package.contains('!'),
            _ => false,
        }
    }
}

impl std::fmt::Display for AppKey<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exe(path) => write!(f, "exe:{path}"),
            Self::Steam(id) => write!(f, "steam:{id}"),
            Self::Riot(id) => write!(f, "riot:{id}"),
            Self::Product(name) => write!(f, "product:{name}"),
            Self::Package(package) => write!(f, "package:{package}"),
            Self::Unknown(key) => f.write_str(key),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppIdentity {
    pub key: String,
    pub name: String,
    pub category: Category,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IntervalKind {
    Presence(AppIdentity),
    Focus(AppIdentity),
    Desktop,
    Unknown,
    Idle,
    Paused,
    Gap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Interval {
    pub start_ms: i64,
    pub end_ms: i64,
    pub offset_seconds: i32,
    pub kind: IntervalKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CaptureMode {
    #[default]
    All,
    Selected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub paused: bool,
    pub launch_at_sign_in: bool,
    pub mode: CaptureMode,
    pub selected: BTreeSet<String>,
    pub excluded: BTreeSet<String>,
    pub idle_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            paused: false,
            launch_at_sign_in: true,
            mode: CaptureMode::All,
            selected: BTreeSet::new(),
            excluded: BTreeSet::new(),
            idle_minutes: 5,
        }
    }
}

impl Settings {
    pub fn is_recording(&self) -> bool {
        self.enabled && !self.paused
    }

    pub fn allows(&self, key: &str) -> bool {
        self.is_recording()
            && !self.excluded.contains(key)
            && (self.mode == CaptureMode::All || self.selected.contains(key))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ControlRequest {
    Status,
    SetSettings(Settings),
    SetPaused(bool),
    EditApp {
        key: String,
        name: String,
        category: Category,
    },
    GroupApps {
        source: String,
        target: String,
    },
    UndoCorrection,
    DeleteHistory {
        key: Option<String>,
    },
    Backup {
        path: PathBuf,
    },
    Quit,
    /// Clears the name and category edits of an app, as one undoable correction.
    /// Only sent to recorders advertising `CAPABILITY_RESET_APP`.
    ResetApp {
        key: String,
    },
    /// Removes a source app from its group, as one undoable correction. The key is the source,
    /// not the group's representative. Only sent to recorders advertising `CAPABILITY_UNGROUP_APP`.
    UngroupApp {
        key: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ControlResponse {
    Ok,
    Status(RecorderStatus),
    Error(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecorderStatus {
    pub settings: Settings,
    pub candidates: Vec<AppIdentity>,
    pub problem: Option<String>,
    pub recording: bool,
    /// Commands this recorder understands beyond the original set. Older recorders omit
    /// the field, so the dashboard hides features they can't perform.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

pub const CAPABILITY_RESET_APP: &str = "reset-app";
pub const CAPABILITY_UNGROUP_APP: &str = "ungroup-app";

pub fn database_path() -> io::Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))?;
    Ok(PathBuf::from(local).join("Tempora").join("history.sqlite3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_keys_parse_and_write_back_unchanged() {
        for (key, parsed) in [
            (r"exe:c:\games\game.exe", AppKey::Exe(r"c:\games\game.exe")),
            ("steam:730", AppKey::Steam("730")),
            ("riot:valorant", AppKey::Riot("valorant")),
            ("product:spotify", AppKey::Product("spotify")),
            (
                "package:contoso.app_8wekyb3d8bbwe!app",
                AppKey::Package("contoso.app_8wekyb3d8bbwe!app"),
            ),
            (
                "package:contoso.app_8wekyb3d8bbwe:tool.exe",
                AppKey::Package("contoso.app_8wekyb3d8bbwe:tool.exe"),
            ),
            ("legacy", AppKey::Unknown("legacy")),
            ("future:thing", AppKey::Unknown("future:thing")),
        ] {
            assert_eq!(AppKey::parse(key), parsed);
            assert_eq!(parsed.to_string(), key);
        }
        assert!(AppKey::parse("package:family!app").is_single_app());
        assert!(!AppKey::parse("package:family").is_single_app());
        assert!(AppKey::parse("product:discord").is_single_app());
        assert!(!AppKey::parse("steam:1").is_single_app());
    }

    #[test]
    fn status_from_an_older_recorder_has_no_capabilities() {
        let older = r#"{"settings":{},"candidates":[],"problem":null,"recording":true}"#;
        let status: RecorderStatus = serde_json::from_str(older).unwrap();
        assert!(status.capabilities.is_empty());
        let request = serde_json::to_string(&ControlRequest::ResetApp {
            key: "steam:1".into(),
        })
        .unwrap();
        assert!(serde_json::from_str::<ControlRequest>(&request).is_ok());
        let request = serde_json::to_string(&ControlRequest::UngroupApp {
            key: r"exe:c:\games\game.exe".into(),
        })
        .unwrap();
        assert!(matches!(
            serde_json::from_str::<ControlRequest>(&request),
            Ok(ControlRequest::UngroupApp { key }) if key == r"exe:c:\games\game.exe"
        ));
    }
}

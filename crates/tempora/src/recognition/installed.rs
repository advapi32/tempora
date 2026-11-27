//! Installed apps from the Uninstall entries in the registry, and which one owns an executable.
//!
//! An entry names its folder through `InstallLocation`, or indirectly through the icon or
//! uninstaller it points at. The entry whose folder is the deepest ancestor of an executable
//! owns it. Folders that hold many unrelated apps, such as `Program Files`, never count.

use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledApp {
    /// The entry's key name: an MSI product code, an Inno Setup app ID or the installer's name.
    pub entry: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    /// Lowercased folders with backslashes and no trailing separator.
    pub folders: Vec<String>,
}

pub trait InstalledApps {
    /// Changes whenever an app is installed or removed; `None` when it can't be read.
    fn stamp(&self) -> Option<u64>;
    fn read(&self) -> Vec<InstalledApp>;
}

/// The installed apps, reread only when the source's stamp changes.
pub(super) struct Installed {
    source: Box<dyn InstalledApps>,
    stamp: Option<Option<u64>>,
    apps: Vec<InstalledApp>,
}

impl Installed {
    pub(super) fn new(source: Box<dyn InstalledApps>) -> Self {
        Self {
            source,
            stamp: None,
            apps: Vec::new(),
        }
    }

    /// The installed app whose folder most closely contains `path`. Two different entries
    /// sharing that folder make the owner ambiguous, so neither is chosen.
    pub(super) fn owner(&mut self, path: &Path) -> Option<&InstalledApp> {
        let stamp = self.source.stamp();
        if self.stamp != Some(stamp) {
            self.apps = self.source.read();
            self.stamp = Some(stamp);
        }
        let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
        let mut best: Option<(usize, &InstalledApp)> = None;
        let mut tied = false;
        for app in &self.apps {
            for folder in &app.folders {
                let contains = path
                    .strip_prefix(folder.as_str())
                    .is_some_and(|rest| rest.starts_with('\\'));
                if !contains {
                    continue;
                }
                match best {
                    Some((depth, _)) if folder.len() < depth => {}
                    Some((depth, owner)) if folder.len() == depth => {
                        tied |= !std::ptr::eq(owner, app);
                    }
                    _ => {
                        best = Some((folder.len(), app));
                        tied = false;
                    }
                }
            }
        }
        best.filter(|_| !tied).map(|(_, app)| app)
    }
}

/// The folders an entry names, from its `InstallLocation`, `DisplayIcon` and `UninstallString`.
pub fn folders(
    install_location: Option<&str>,
    display_icon: Option<&str>,
    uninstall_string: Option<&str>,
) -> Vec<String> {
    let mut folders = Vec::new();
    let candidates = [
        install_location.map(|location| unquote(location).to_owned()),
        display_icon.and_then(icon_file).and_then(parent),
        uninstall_string.and_then(command_file).and_then(parent),
    ];
    for folder in candidates.into_iter().flatten() {
        let folder = folder.replace('/', "\\").to_lowercase();
        let folder = folder.trim_end_matches('\\').to_owned();
        if is_specific(&folder) && !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

fn unquote(text: &str) -> &str {
    text.trim().trim_matches('"').trim()
}

/// `"C:\App\app.exe",0` → `C:\App\app.exe`.
fn icon_file(icon: &str) -> Option<String> {
    let icon = unquote(icon);
    let file = match icon.rsplit_once(',') {
        Some((file, index)) if index.trim().trim_start_matches('-').parse::<u32>().is_ok() => file,
        _ => icon,
    };
    Some(unquote(file).to_owned())
}

/// The program a command line runs: the quoted part, or everything up to `.exe`.
fn command_file(command: &str) -> Option<String> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return rest.split_once('"').map(|(file, _)| file.to_owned());
    }
    let end = command.to_ascii_lowercase().find(".exe")? + ".exe".len();
    Some(command[..end].to_owned())
}

fn parent(file: String) -> Option<String> {
    let file = file.replace('/', "\\");
    let (folder, _) = file.rsplit_once('\\')?;
    Some(folder.to_owned())
}

/// Whether a lowercased folder belongs to one app rather than holding many. It must be an
/// absolute local path below the drive root, outside Windows, and not a well-known shared
/// folder such as `Program Files`, `AppData\Local` or a user profile.
fn is_specific(folder: &str) -> bool {
    let bytes = folder.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
        return false;
    }
    let parts: Vec<&str> = folder[3..].split('\\').collect();
    if parts.iter().any(|part| part.is_empty()) {
        return false;
    }
    const SHARED: [&str; 14] = [
        "program files",
        "program files (x86)",
        "programdata",
        "common files",
        "appdata",
        "local",
        "locallow",
        "roaming",
        "programs",
        "users",
        "windowsapps",
        "steamapps",
        "common",
        "games",
    ];
    let last = parts[parts.len() - 1];
    !(parts[0] == "windows" || SHARED.contains(&last) || (parts[0] == "users" && parts.len() <= 2))
}

#[cfg(windows)]
pub use registry_source::RegistryInstalled;

#[cfg(windows)]
mod registry_source {
    use super::super::registry::{Hive, Key};
    use super::{InstalledApp, InstalledApps, folders};

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    const UNINSTALL_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    const ROOTS: [(Hive, &str); 3] = [
        (Hive::LocalMachine, UNINSTALL),
        (Hive::LocalMachine, UNINSTALL_32),
        (Hive::CurrentUser, UNINSTALL),
    ];

    /// Reads the machine's and the current user's Uninstall entries.
    pub struct RegistryInstalled;

    impl InstalledApps for RegistryInstalled {
        fn stamp(&self) -> Option<u64> {
            let times: Vec<u64> = ROOTS
                .iter()
                .map(|(hive, path)| {
                    Key::open_in(*hive, path)
                        .and_then(|key| key.last_write())
                        .unwrap_or(0)
                })
                .collect();
            times.iter().any(|time| *time != 0).then(|| {
                times
                    .iter()
                    .fold(0u64, |stamp, time| stamp.rotate_left(21) ^ time)
            })
        }

        fn read(&self) -> Vec<InstalledApp> {
            let mut apps = Vec::new();
            for (hive, path) in ROOTS {
                let Some(root) = Key::open_in(hive, path) else {
                    continue;
                };
                for name in root.subkey_names() {
                    let Some(entry) = root.subkey(&name) else {
                        continue;
                    };
                    // Updates and patches point at their product through ParentKeyName.
                    if entry.string("ParentKeyName").is_some() {
                        continue;
                    }
                    let folders = folders(
                        entry.string("InstallLocation").as_deref(),
                        entry.string("DisplayIcon").as_deref(),
                        entry.string("UninstallString").as_deref(),
                    );
                    if folders.is_empty() {
                        continue;
                    }
                    apps.push(InstalledApp {
                        entry: name,
                        display_name: entry.string("DisplayName"),
                        publisher: entry.string("Publisher"),
                        folders,
                    });
                }
            }
            apps
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn folders_come_from_the_location_icon_and_uninstaller() {
        assert_eq!(
            folders(
                Some(r"C:\Program Files\7-Zip\"),
                Some(r"C:\Program Files\7-Zip\7zFM.exe,0"),
                Some(r#""C:\Program Files\7-Zip\Uninstall.exe""#),
            ),
            vec![r"c:\program files\7-zip".to_owned()]
        );
        assert_eq!(
            folders(
                None,
                Some(r#""D:\Tools\Obsidian\Obsidian.exe",-101"#),
                Some(r"D:\Tools\Obsidian\Uninstall Obsidian.exe /currentuser"),
            ),
            vec![r"d:\tools\obsidian".to_owned()]
        );
    }

    #[test]
    fn shared_folders_never_own_apps() {
        for (location, icon, uninstall) in [
            (Some(r"C:\Program Files"), None, None),
            (Some(r"C:\Program Files (x86)\"), None, None),
            (None, Some(r"C:\Windows\System32\msiexec.exe"), None),
            (
                None,
                None,
                Some("MsiExec.exe /X{23170F69-40C1-2702-2301-000001000000}"),
            ),
            (Some(r"C:\Users\Example"), None, None),
            (Some(r"C:\Users\Example\AppData\Local"), None, None),
            (Some(r"C:\Users\Example\AppData\Local\Programs"), None, None),
            (Some(r"D:\SteamLibrary\steamapps\common"), None, None),
            (Some(r"E:\"), None, None),
            (Some(r"\\server\share\app"), None, None),
            (Some("relative\\app"), None, None),
        ] {
            assert!(
                folders(location, icon, uninstall).is_empty(),
                "{location:?} {icon:?} {uninstall:?}"
            );
        }
    }

    struct Fake {
        apps: Vec<InstalledApp>,
        stamp: Rc<Cell<u64>>,
        reads: Rc<Cell<usize>>,
    }

    impl InstalledApps for Fake {
        fn stamp(&self) -> Option<u64> {
            Some(self.stamp.get())
        }

        fn read(&self) -> Vec<InstalledApp> {
            self.reads.set(self.reads.get() + 1);
            self.apps.clone()
        }
    }

    fn app(entry: &str, folders: &[&str]) -> InstalledApp {
        InstalledApp {
            entry: entry.into(),
            folders: folders.iter().map(|folder| (*folder).to_owned()).collect(),
            ..InstalledApp::default()
        }
    }

    #[test]
    fn the_deepest_containing_folder_owns_an_executable() {
        let stamp = Rc::new(Cell::new(1));
        let reads = Rc::new(Cell::new(0));
        let mut installed = Installed::new(Box::new(Fake {
            apps: vec![
                app("Suite", &[r"d:\tools\suite"]),
                app("Editor", &[r"d:\tools\suite\editor"]),
                app("Other", &[r"d:\tools\suiteextra"]),
                app("Shared A", &[r"d:\tools\shared"]),
                app("Shared B", &[r"d:\tools\shared", r"d:\tools\shared"]),
                app("Mine", &[r"d:\tools\shared\mine"]),
            ],
            stamp: Rc::clone(&stamp),
            reads: Rc::clone(&reads),
        }));
        let owner = |installed: &mut Installed, path: &str| {
            installed
                .owner(Path::new(path))
                .map(|app| app.entry.clone())
        };
        assert_eq!(
            owner(&mut installed, r"D:\Tools\Suite\Editor\bin\editor.exe").as_deref(),
            Some("Editor")
        );
        assert_eq!(
            owner(&mut installed, r"D:/Tools/Suite/suite.exe").as_deref(),
            Some("Suite")
        );
        assert_eq!(owner(&mut installed, r"D:\Tools\SuiteExtra.exe"), None);
        assert_eq!(
            owner(&mut installed, r"D:\Tools\Shared\one.exe"),
            None,
            "two entries sharing the deepest folder are ambiguous"
        );
        assert_eq!(
            owner(&mut installed, r"D:\Tools\Shared\Mine\mine.exe").as_deref(),
            Some("Mine"),
            "a deeper folder still decides"
        );
        assert_eq!(reads.get(), 1, "an unchanged stamp reuses the list");
        stamp.set(2);
        owner(&mut installed, r"D:\Tools\Suite\suite.exe");
        assert_eq!(reads.get(), 2, "a new stamp rereads it");
    }
}

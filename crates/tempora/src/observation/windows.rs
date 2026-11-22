use super::{Focus, Observation, capture_clock};
use crate::{
    model::{AppIdentity, AppKey, Category},
    recognition::Recognizer,
    win::{self, Window},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::{
        WS_CAPTION, WS_CHILD, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    },
};

struct Process {
    created: u64,
    identity: Option<Owner>,
}

#[derive(Clone)]
struct Owner {
    key: String,
    app: AppIdentity,
    packaged: bool,
}

pub struct WindowsObserver {
    origin: Instant,
    pid: u32,
    session: u32,
    tracker_directory: Option<String>,
    windows_directory: Option<String>,
    cache: HashMap<u32, Process>,
    qualified: BTreeMap<String, String>,
    steam_aliases: BTreeMap<String, AppIdentity>,
    recognizer: Recognizer,
    path_buffer: Vec<u16>,
    /// The `exe:` key of the foreground app when its window covers a whole monitor, as of the
    /// latest sample.
    last_fullscreen: Option<String>,
}

impl WindowsObserver {
    pub fn new() -> io::Result<Self> {
        let pid = win::current_process_id();
        let session = win::process_session(pid).ok_or_else(io::Error::last_os_error)?;
        let tracker_directory = std::env::current_exe()?.parent().map(path_key);
        let windows_directory = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .as_deref()
            .map(path_key);
        Ok(Self {
            origin: Instant::now(),
            pid,
            session,
            tracker_directory,
            windows_directory,
            cache: HashMap::new(),
            qualified: BTreeMap::new(),
            steam_aliases: BTreeMap::new(),
            recognizer: Recognizer::default(),
            path_buffer: vec![0; 260],
            last_fullscreen: None,
        })
    }

    pub fn remember(&mut self, keys: impl IntoIterator<Item = String>) {
        for key in keys {
            self.qualified.entry(key).or_default();
        }
    }

    /// Trusts stored executables whose windows were seen covering a monitor in earlier sessions.
    pub fn remember_fullscreen(&mut self, keys: impl IntoIterator<Item = String>) {
        self.recognizer.remember_fullscreen(keys);
    }

    /// Records fullscreen evidence for an executable, then forgets its cached identities so the
    /// next sample recognizes it again.
    pub fn confirm_fullscreen(&mut self, key: &str) {
        self.recognizer.confirm_fullscreen(key);
        // Otherwise the cached identity would be written back over the refreshed one.
        self.cache.retain(|_, process| {
            !process
                .identity
                .as_ref()
                .is_some_and(|owner| owner.app.key == key)
        });
    }

    /// The `exe:` key of the foreground app if its window covers a whole monitor in the latest
    /// sample.
    pub fn last_fullscreen(&self) -> Option<&str> {
        self.last_fullscreen.as_deref()
    }

    /// Executables newly recognized as a Steam game, keyed by their `exe:` key. The recorder
    /// migrates each one's history and settings to the Steam key, then clears the map.
    pub fn steam_aliases(&mut self) -> &mut BTreeMap<String, AppIdentity> {
        &mut self.steam_aliases
    }

    /// Re-runs recognition for a stored `exe:` key, including a newly matched Steam identity.
    pub fn reidentify(&mut self, key: &str) -> Option<AppIdentity> {
        let path = self.executable_on_disk(key)?;
        let fallback = AppIdentity {
            key: key.to_owned(),
            name: path.file_stem()?.to_string_lossy().into_owned(),
            category: Category::Other,
        };
        let identity = self.recognizer.identify_executable(&path, fallback.clone());
        changed_identity(key, &fallback, identity)
    }

    /// Re-runs recognition for a stored `package:` key from its AUMID or family. Returns the
    /// identity only when it is the same app and changes the stored name or category.
    pub fn reidentify_package(&mut self, key: &str) -> Option<AppIdentity> {
        let package = AppKey::parse(key).package()?;
        // The stored name was the executable's stem or the package itself; neither is known
        // here, so an unchanged lookup compares equal to this fallback and returns nothing.
        let fallback = AppIdentity {
            key: key.to_owned(),
            name: package.to_owned(),
            category: Category::Other,
        };
        let identity = self.recognizer.identify_package(package, fallback.clone());
        changed_identity(key, &fallback, Some(identity))
    }

    /// The grouping fingerprint of a stored `exe:` key, from the executable on disk. Gone files
    /// and non-executable keys give `None`.
    pub fn fingerprint(&mut self, key: &str) -> Option<String> {
        let path = self.executable_on_disk(key)?;
        self.recognizer.fingerprint_executable(&path)
    }

    /// The executable behind a stored `exe:` key, with its casing on disk. History stores paths
    /// lowercased, so the real casing comes from the file system. Files that are gone give `None`.
    fn executable_on_disk(&self, key: &str) -> Option<PathBuf> {
        let stored = AppKey::parse(key).executable()?;
        crate::local_fs::metadata(stored).ok()?;
        let path = strip_verbatim(&std::fs::canonicalize(stored).ok()?);
        (!self.is_helper(&path)).then_some(path)
    }

    pub fn session_eligible(&self) -> bool {
        win::session_is_active_and_unlocked(self.session)
    }

    pub fn sample(&mut self, include_processes: bool) -> Observation {
        self.last_fullscreen = None;
        let foreground = Window::foreground();
        let monotonic = self.origin.elapsed();
        let (utc_ms, local_offset_seconds) = capture_clock();
        let idle_age = idle_age();
        let focus = match foreground {
            None => Focus::Unknown,
            Some(window) if is_desktop(window) => Focus::Desktop,
            Some(window) => match self.window_owner(window, true) {
                Some(owner) => {
                    let app = self.qualify(owner);
                    if AppKey::parse(&app.key).is_executable() && is_fullscreen(window) {
                        self.last_fullscreen = Some(app.key.clone());
                    }
                    Focus::App(app)
                }
                None => Focus::Unknown,
            },
        };
        let processes = include_processes.then(|| self.processes());
        Observation {
            monotonic,
            utc_ms,
            local_offset_seconds,
            focus,
            idle_age,
            processes,
        }
    }

    /// Apps with a window, plus background processes of apps that had one before. When the
    /// process list can't be read, only the apps with windows are returned.
    fn processes(&mut self) -> Vec<AppIdentity> {
        let mut applications = BTreeMap::new();
        for window in Window::top_level() {
            if let Some(owner) = self.window_owner(window, false) {
                let app = self.qualify(owner);
                applications.insert(app.key.clone(), app);
            }
        }
        if self.qualified.is_empty() {
            return Vec::new();
        }
        let Ok(pids) = win::process_ids() else {
            self.cache.clear();
            return applications.into_values().collect();
        };
        let seen: HashSet<u32> = pids.iter().copied().collect();
        for pid in pids {
            if let Some(owner) = self.identity(pid)
                && has_window_evidence(&self.qualified, &owner)
            {
                let app = self.qualify(owner);
                applications.insert(app.key.clone(), app);
            }
        }
        self.cache.retain(|pid, _| seen.contains(pid));
        applications.into_values().collect()
    }

    fn window_owner(&mut self, window: Window, foreground: bool) -> Option<Owner> {
        if is_desktop(window) || !is_app_window(window, foreground) {
            return None;
        }
        if window.class_is("ApplicationFrameWindow") {
            let mut owner = None::<Owner>;
            for child in window.children() {
                if !child.class_is("Windows.UI.Core.CoreWindow") {
                    continue;
                }
                let Some(candidate) = child.process_id().and_then(|pid| self.identity(pid)) else {
                    continue;
                };
                if !candidate.packaged {
                    continue;
                }
                if owner
                    .as_ref()
                    .is_some_and(|first| first.key != candidate.key)
                {
                    return None;
                }
                owner = Some(candidate);
            }
            return owner;
        }
        window.process_id().and_then(|pid| self.identity(pid))
    }

    fn qualify(&mut self, owner: Owner) -> AppIdentity {
        if AppKey::parse(&owner.key).is_executable()
            && AppKey::parse(&owner.app.key).is_steam()
            && self.qualified.get(&owner.key) != Some(&owner.app.key)
        {
            self.steam_aliases
                .insert(owner.key.clone(), owner.app.clone());
        }
        self.qualified.insert(owner.key, owner.app.key.clone());
        owner.app
    }

    fn identity(&mut self, pid: u32) -> Option<Owner> {
        if pid == self.pid || pid == 0 {
            return None;
        }
        if win::process_session(pid) != Some(self.session) {
            self.cache.remove(&pid);
            return None;
        }
        let Some(process) = win::Process::open(pid) else {
            self.cache.remove(&pid);
            return None;
        };
        let created = process.created()?;
        // The PID can be reused between snapshots, while the process creation time identifies its lifetime.
        if let Some(cached) = self
            .cache
            .get(&pid)
            .filter(|cached| cached.created == created)
        {
            return cached.identity.clone();
        }
        let executable = process.executable(&mut self.path_buffer);
        let identity = self.resolve(&process, executable.as_deref());
        self.cache.insert(
            pid,
            Process {
                created,
                identity: identity.clone(),
            },
        );
        identity
    }

    fn resolve(&mut self, process: &win::Process, path: Option<&Path>) -> Option<Owner> {
        if path.is_some_and(|path| self.is_helper(path)) {
            return None;
        }
        let stem = path
            .and_then(Path::file_stem)
            .map(|stem| stem.to_string_lossy().into_owned());
        if let Some(package) = process.package_identity() {
            let fallback = AppIdentity {
                key: AppKey::Package(&package.to_lowercase()).to_string(),
                name: stem.unwrap_or_else(|| package.clone()),
                category: Category::Other,
            };
            let key = path.and_then(Path::file_name).map_or_else(
                || fallback.key.clone(),
                |file| format!("{}:{}", fallback.key, file.to_string_lossy().to_lowercase()),
            );
            return Some(Owner {
                key,
                app: self.recognizer.identify_package(&package, fallback),
                packaged: true,
            });
        }
        let path = path?;
        let fallback = AppIdentity {
            key: AppKey::Exe(&path_key(path)).to_string(),
            name: stem?,
            category: Category::Other,
        };
        let key = fallback.key.clone();
        self.recognizer
            .identify_executable(path, fallback)
            .map(|app| Owner {
                key,
                app,
                packaged: false,
            })
    }

    fn is_helper(&self, path: &Path) -> bool {
        let Some(file) = path.file_name().and_then(|file| file.to_str()) else {
            return false;
        };
        let lowercase_file = file.to_ascii_lowercase();
        if matches!(
            lowercase_file.as_str(),
            "tempora-recorder.exe" | "tempora.exe"
        ) && path.parent().map(path_key) == self.tracker_directory
        {
            return true;
        }
        is_system_helper(&lowercase_file)
            && self.windows_directory.as_ref().is_some_and(|directory| {
                path_key(path)
                    .strip_prefix(directory)
                    .is_some_and(|rest| rest.starts_with('\\'))
            })
    }
}

fn has_window_evidence(qualified: &BTreeMap<String, String>, owner: &Owner) -> bool {
    if qualified.contains_key(&owner.key) {
        return true;
    }
    let key = &owner.app.key;
    // Recognition only assigns product keys from known installation evidence, and an AUMID
    // identifies a particular Windows app; a package family can contain several apps.
    AppKey::parse(key).is_single_app() && qualified.get(key).is_some_and(String::is_empty)
}

fn is_desktop(window: Window) -> bool {
    let shell = Window::shell();
    if Some(window) == Window::desktop() || Some(window) == shell {
        return true;
    }
    let shell_pid = shell.and_then(Window::process_id);
    shell_pid.is_some()
        && window.process_id() == shell_pid
        && [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ]
        .iter()
        .any(|class| window.class_is(class))
}

fn is_app_window(window: Window, foreground: bool) -> bool {
    if !window.is_visible() {
        return false;
    }
    let style = window.style();
    let extended = window.extended_style();
    if style & WS_CHILD != 0 || extended & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
        return false;
    }
    if !foreground && extended & WS_EX_APPWINDOW == 0 && window.has_owner() {
        return false;
    }
    if window.is_cloaked() {
        return false;
    }
    if window.is_minimized() {
        return true;
    }
    window
        .rect()
        .is_some_and(|bounds| bounds.right > bounds.left && bounds.bottom > bounds.top)
}

/// Pixels each edge may differ by. A DPI-unaware process sees rescaled coordinates at 125 % or
/// 150 % scaling, and rounding can move an edge by one. A maximized window misses by the whole
/// taskbar height, far more than this.
const FULLSCREEN_TOLERANCE: i32 = 2;

/// Whether a window covers its whole monitor without a caption. A maximized window stops at the
/// work area and keeps its caption, so it doesn't count.
fn covers_monitor(window: RECT, monitor: RECT, style: u32) -> bool {
    let near = |a: i32, b: i32| (a - b).abs() <= FULLSCREEN_TOLERANCE;
    let whole_monitor = near(window.left, monitor.left)
        && near(window.top, monitor.top)
        && near(window.right, monitor.right)
        && near(window.bottom, monitor.bottom);
    // WS_CAPTION is two bits (WS_BORDER | WS_DLGFRAME): a caption needs both.
    whole_monitor && (style & WS_CAPTION) != WS_CAPTION
}

fn is_fullscreen(window: Window) -> bool {
    match (window.rect(), window.monitor_rect()) {
        (Some(bounds), Some(monitor)) => covers_monitor(bounds, monitor, window.style()),
        _ => false,
    }
}

/// `std::fs::canonicalize` returns verbatim paths (`\\?\C:\...`, `\\?\UNC\...`). Strip the prefix
/// so the path reads like the one Windows shows.
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path.to_path_buf()
    }
}

/// The refreshed identity when its automatic details changed or Steam recognized its install.
fn changed_identity(
    key: &str,
    fallback: &AppIdentity,
    identity: Option<AppIdentity>,
) -> Option<AppIdentity> {
    identity.filter(|identity| {
        (identity.key == key && identity != fallback)
            || (AppKey::parse(key).is_executable() && AppKey::parse(&identity.key).is_steam())
    })
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn is_system_helper(executable: &str) -> bool {
    matches!(
        executable,
        "applicationframehost.exe"
            | "backgroundtaskhost.exe"
            | "conhost.exe"
            | "csrss.exe"
            | "ctfmon.exe"
            | "dllhost.exe"
            | "dwm.exe"
            | "fontdrvhost.exe"
            | "lockapp.exe"
            | "runtimebroker.exe"
            | "searchapp.exe"
            | "searchhost.exe"
            | "shellexperiencehost.exe"
            | "sihost.exe"
            | "startmenuexperiencehost.exe"
            | "svchost.exe"
            | "taskhostw.exe"
            | "textinputhost.exe"
            | "winlogon.exe"
            | "wudfhost.exe"
    )
}

fn idle_age() -> Option<Duration> {
    let (now, last_input) = win::input_ticks()?;
    input_age(now, last_input)
}

/// Time since the last input, from the 64-bit tick count and the input time's low 32 bits.
fn input_age(now: u64, last_input: u32) -> Option<Duration> {
    let age = u64::from((now as u32).wrapping_sub(last_input));
    if age > now {
        return None;
    }
    // A timestamp just ahead of the tick counter is not a 49-day idle period.
    if age > u64::from(u32::MAX) - 1_000 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_millis(age))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_qualifies_its_owner_without_qualifying_a_different_executable() {
        let mut observer = WindowsObserver::new().unwrap();
        let game = AppIdentity {
            key: "steam:123".into(),
            name: "Game".into(),
            category: Category::Games,
        };
        let primary = Owner {
            key: "exe:c:\\games\\game.exe".into(),
            app: game.clone(),
            packaged: false,
        };
        let helper = Owner {
            key: "exe:c:\\games\\helper.exe".into(),
            app: game.clone(),
            packaged: false,
        };
        observer.remember([game.key.clone()]);
        assert!(!has_window_evidence(&observer.qualified, &helper));
        assert!(!has_window_evidence(&observer.qualified, &primary));
        observer.qualify(primary.clone());
        assert!(has_window_evidence(&observer.qualified, &primary));
        assert!(!has_window_evidence(&observer.qualified, &helper));
    }

    #[test]
    fn a_recognized_tray_app_can_reuse_persisted_window_evidence() {
        let mut observer = WindowsObserver::new().unwrap();
        let spotify = Owner {
            key: "exe:c:\\users\\test\\appdata\\roaming\\spotify\\spotify.exe".into(),
            app: AppIdentity {
                key: "product:spotify".into(),
                name: "Spotify".into(),
                category: Category::Media,
            },
            packaged: false,
        };
        assert!(!has_window_evidence(&observer.qualified, &spotify));
        observer.remember([spotify.app.key.clone()]);
        assert!(has_window_evidence(&observer.qualified, &spotify));
    }

    #[test]
    fn input_age_handles_the_32_bit_wrap_of_the_input_time() {
        assert_eq!(input_age(5_000, 4_000), Some(Duration::from_secs(1)));
        // The input time is the tick count's low 32 bits; it wraps after 49.7 days.
        let now = u64::from(u32::MAX) + 1_500;
        assert_eq!(input_age(now, u32::MAX - 500), Some(Duration::from_secs(2)));
        // An input stamp just ahead of the tick count is no input age at all.
        let now = u64::from(u32::MAX) * 2;
        assert_eq!(
            input_age(now, (now as u32).wrapping_add(100)),
            Some(Duration::ZERO)
        );
        assert_eq!(input_age(100, 5_000), None);
    }

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn only_an_exact_captionless_monitor_rect_is_fullscreen() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WS_BORDER, WS_POPUP};

        let monitor = rect(0, 0, 2560, 1440);
        assert!(covers_monitor(monitor, monitor, WS_POPUP));
        assert!(covers_monitor(monitor, monitor, 0));
        // One border bit is not a caption: WS_CAPTION needs both WS_BORDER and WS_DLGFRAME.
        assert!(covers_monitor(monitor, monitor, WS_POPUP | WS_BORDER));
        assert!(!covers_monitor(monitor, monitor, WS_POPUP | WS_CAPTION));
        assert!(!covers_monitor(monitor, monitor, WS_CAPTION));
        // Maximized: the work area (taskbar removed) with its caption.
        let work_area = rect(0, 0, 2560, 1400);
        assert!(!covers_monitor(work_area, monitor, WS_POPUP | WS_CAPTION));
        assert!(!covers_monitor(work_area, monitor, WS_POPUP));
        // Rounding from DPI scaling (one or two pixels) still counts.
        assert!(covers_monitor(rect(0, 0, 2559, 1441), monitor, WS_POPUP));
        assert!(covers_monitor(rect(-2, 1, 2560, 1440), monitor, WS_POPUP));
        // Three pixels short on any edge doesn't.
        assert!(!covers_monitor(rect(0, 0, 2557, 1440), monitor, WS_POPUP));
        assert!(!covers_monitor(rect(3, 0, 2560, 1440), monitor, WS_POPUP));
        assert!(!covers_monitor(rect(0, 3, 2560, 1440), monitor, WS_POPUP));
        assert!(!covers_monitor(rect(0, 0, 2560, 1437), monitor, WS_POPUP));
    }

    #[test]
    fn verbatim_prefixes_are_removed_before_recognition() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Games\RDR2.exe")),
            PathBuf::from(r"C:\Games\RDR2.exe")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\server\share\RDR2.exe")),
            PathBuf::from(r"\\server\share\RDR2.exe")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"C:\Games\RDR2.exe")),
            PathBuf::from(r"C:\Games\RDR2.exe")
        );
    }

    #[test]
    fn reidentification_returns_changed_details_and_steam_aliases() {
        let key = r"exe:c:\games\rdr2.exe";
        let fallback = AppIdentity {
            key: key.into(),
            name: "rdr2".into(),
            category: Category::Other,
        };
        let game = AppIdentity {
            key: key.into(),
            name: "Red Dead Redemption 2".into(),
            category: Category::Games,
        };
        assert_eq!(
            changed_identity(key, &fallback, Some(game.clone())),
            Some(game.clone())
        );
        assert_eq!(
            changed_identity(key, &fallback, Some(fallback.clone())),
            None
        );
        assert_eq!(changed_identity(key, &fallback, None), None);
        let other_app = AppIdentity {
            key: "steam:1".into(),
            ..game.clone()
        };
        assert_eq!(
            changed_identity(key, &fallback, Some(other_app.clone())),
            Some(other_app)
        );
        let unrelated = AppIdentity {
            key: "riot:other".into(),
            ..fallback.clone()
        };
        assert_eq!(changed_identity(key, &fallback, Some(unrelated)), None);
        let recategorized = AppIdentity {
            category: Category::Games,
            ..fallback.clone()
        };
        assert_eq!(
            changed_identity(key, &fallback, Some(recategorized.clone())),
            Some(recategorized)
        );
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("gta-observer-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_stored_key_resolves_to_the_executable_with_its_casing_on_disk() {
        let directory = TempDirectory::new();
        let game = directory.0.join("Red Dead Redemption 2");
        std::fs::create_dir(&game).unwrap();
        let file = game.join("RDR2.exe");
        std::fs::write(&file, b"").unwrap();
        let observer = WindowsObserver::new().unwrap();

        // History stores the path lowercased.
        let stored = format!("exe:{}", path_key(&file));
        assert!(stored.contains("red dead redemption 2"));
        let resolved = observer.executable_on_disk(&stored).unwrap();
        assert!(
            resolved
                .to_string_lossy()
                .ends_with(r"Red Dead Redemption 2\RDR2.exe"),
            "{}",
            resolved.display()
        );

        // An uninstalled game is not refreshed, and other key kinds are not paths.
        std::fs::remove_file(&file).unwrap();
        assert!(observer.executable_on_disk(&stored).is_none());
        assert!(observer.executable_on_disk("steam:123").is_none());
    }

    #[test]
    fn confirming_fullscreen_forgets_only_that_apps_cached_identities() {
        let mut observer = WindowsObserver::new().unwrap();
        let cached = |key: &str| Process {
            created: 1,
            identity: Some(Owner {
                key: key.into(),
                app: AppIdentity {
                    key: key.into(),
                    name: key.into(),
                    category: Category::Other,
                },
                packaged: false,
            }),
        };
        observer.cache.insert(10, cached(r"exe:c:\games\rdr2.exe"));
        observer.cache.insert(11, cached(r"exe:c:\tools\tool.exe"));
        observer.cache.insert(
            12,
            Process {
                created: 1,
                identity: None,
            },
        );
        observer.confirm_fullscreen(r"exe:c:\games\rdr2.exe");
        let mut pids: Vec<u32> = observer.cache.keys().copied().collect();
        pids.sort_unstable();
        assert_eq!(pids, [11, 12]);
    }
}

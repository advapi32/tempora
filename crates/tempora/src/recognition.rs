pub mod catalog;
mod game_store;
mod installed;
mod names;
mod package_names;
mod products;
#[cfg(windows)]
mod registry;
mod steam;
mod version_info;

use std::collections::HashSet;
use std::path::Path;

use crate::model::{AppIdentity, AppKey, Category};

pub use catalog::Catalog;
pub use installed::{InstalledApp, InstalledApps};

#[cfg(windows)]
use game_store::RegistryGameEntries;
use game_store::{GameEntries, GameMatch, match_game};
use installed::Installed;
use package_names::PackageNames;
#[cfg(windows)]
use package_names::ShellPackageNames;
pub(crate) use products::known_product_for_key;
#[cfg(windows)]
use version_info::FileVersionSource;
use version_info::{VersionInfo, VersionSource};

pub struct Recognizer {
    steam: steam::Libraries,
    games: Box<dyn GameEntries>,
    versions: Box<dyn VersionSource>,
    packages: Box<dyn PackageNames>,
    installed: Installed,
    catalog: Catalog,
    /// Keys of executables whose window has been seen covering a whole monitor.
    fullscreen_seen: HashSet<String>,
}

impl Default for Recognizer {
    /// Uses Windows' game list, the file's version resource, the shell's Apps folder, the
    /// Uninstall entries and the built-in WinGet catalog on Windows. Elsewhere all are empty.
    fn default() -> Self {
        #[cfg(windows)]
        let (games, versions, packages): (
            Box<dyn GameEntries>,
            Box<dyn VersionSource>,
            Box<dyn PackageNames>,
        ) = (
            Box::new(RegistryGameEntries),
            Box::new(FileVersionSource),
            Box::new(ShellPackageNames),
        );
        #[cfg(windows)]
        let installed: Box<dyn InstalledApps> = Box::new(installed::RegistryInstalled);
        #[cfg(not(windows))]
        let installed: Box<dyn InstalledApps> = Box::new(NoInstalledApps);
        #[cfg(not(windows))]
        let (games, versions, packages): (
            Box<dyn GameEntries>,
            Box<dyn VersionSource>,
            Box<dyn PackageNames>,
        ) = (
            Box::new(NoGameEntries),
            Box::new(NoVersions),
            Box::new(NoPackageNames),
        );
        Self::with_sources(games, versions, packages)
            .with_installed(installed)
            .with_catalog(Catalog::embedded())
    }
}

impl Recognizer {
    pub fn with_sources(
        games: Box<dyn GameEntries>,
        versions: Box<dyn VersionSource>,
        packages: Box<dyn PackageNames>,
    ) -> Self {
        Self {
            steam: steam::Libraries::default(),
            games,
            versions,
            packages,
            installed: Installed::new(Box::new(NoInstalledApps)),
            catalog: Catalog::empty(),
            fullscreen_seen: HashSet::new(),
        }
    }

    pub fn with_installed(mut self, installed: Box<dyn InstalledApps>) -> Self {
        self.installed = Installed::new(installed);
        self
    }

    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// Records that the executable with this key was seen covering a whole monitor.
    pub fn confirm_fullscreen(&mut self, key: &str) {
        self.fullscreen_seen.insert(key.to_owned());
    }

    pub fn remember_fullscreen(&mut self, keys: impl IntoIterator<Item = String>) {
        self.fullscreen_seen.extend(keys);
    }

    /// Recognizes an executable from installed-product, Steam and Windows game-list evidence,
    /// then categorizes what is left from the WinGet catalog. Returns `None` for filtered helper
    /// processes. Ordinary apps retain their fallback key.
    pub fn identify_executable(
        &mut self,
        path: &Path,
        mut fallback: AppIdentity,
    ) -> Option<AppIdentity> {
        if let Some(product) = products::for_executable(&path.to_string_lossy()) {
            return Some(product);
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Some(fallback);
        };
        let ascii_name = file_name.to_ascii_lowercase();
        if let Some(identity) = riot_identity(path, &ascii_name) {
            return Some(identity);
        }
        if is_helper(&ascii_name) {
            return None;
        }
        if let Some(identity) = self.steam.identify(path) {
            return Some(identity);
        }
        // The Windows game list uses Unicode lowercase, while the built-in names above are ASCII.
        let entries = self.games.entries(&file_name.to_lowercase());
        let game = match_game(path, &entries)
            .filter(|game| game.title_id.is_some() || self.fullscreen_seen.contains(&fallback.key));
        let version = self.versions.read(path);
        if let Some(game) = game {
            fallback.name = names::game_name(path, &game, version.as_ref(), &fallback.name);
            fallback.category = Category::Games;
            return Some(fallback);
        }

        if fallback.category == Category::Other {
            fallback.category = match ascii_name.as_str() {
                "chrome.exe" | "msedge.exe" | "firefox.exe" | "brave.exe" | "vivaldi.exe"
                | "opera.exe" => Category::Browsers,
                "discord.exe" | "discordptb.exe" | "discordcanary.exe" => Category::Communication,
                "spotify.exe" => Category::Media,
                _ => Category::Other,
            };
        }
        if fallback.category == Category::Other
            && let Some(category) = self.catalog_category(path, file_name, version.as_ref())
        {
            fallback.category = category;
        }
        if let Some(name) = names::executable_name(version.as_ref()) {
            fallback.name = name;
        }
        Some(fallback)
    }

    /// Identifies a known product by its canonical key. For other packages, preserves the key,
    /// takes the catalog's category for the package family when the fallback has none, and uses
    /// the shell display name if available. A family alone skips shell lookup.
    pub fn identify_package(&self, package: &str, mut fallback: AppIdentity) -> AppIdentity {
        if let Some(known) = products::for_package(package) {
            return known;
        }
        if fallback.category == Category::Other {
            let family = package.split(['!', ':']).next().unwrap_or(package);
            if let Some(category) = self.catalog.package_family(family) {
                fallback.category = category;
            }
        }
        if !package.contains('!') {
            return fallback;
        }
        match self
            .packages
            .display_name(package)
            .and_then(|name| names::package_display_name(&name))
        {
            Some(name) => AppIdentity { name, ..fallback },
            None => fallback,
        }
    }

    /// The catalog's category for an executable: through the installed app that owns its folder,
    /// or for a portable app through its file name and the publisher in its version resource.
    fn catalog_category(
        &mut self,
        path: &Path,
        file_name: &str,
        version: Option<&VersionInfo>,
    ) -> Option<Category> {
        if self.catalog.is_empty() {
            return None;
        }
        let catalog = self.catalog;
        self.installed
            .owner(path)
            .and_then(|app| {
                catalog.installed(
                    &app.entry,
                    app.display_name.as_deref(),
                    app.publisher.as_deref(),
                )
            })
            .or_else(|| catalog.executable(file_name, version?.company.as_deref()?))
    }

    /// The grouping fingerprint of an executable. The version resource is read only when the
    /// game list has no title ID to use.
    pub fn fingerprint_executable(&mut self, path: &Path) -> Option<String> {
        let file_lower = path.file_name()?.to_str()?.to_lowercase();
        let game = match_game(path, &self.games.entries(&file_lower));
        let version = if title_id(game.as_ref()).is_some() {
            None
        } else {
            self.versions.read(path)
        };
        fingerprint(&file_lower, game.as_ref(), version.as_ref())
    }
}

/// The title ID of a game match, when it is non-empty.
fn title_id(game: Option<&GameMatch>) -> Option<&str> {
    game.and_then(|game| game.title_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Identifies the same program across install locations, for automatic grouping. A title ID is
/// the strongest evidence. Otherwise the company and product must both be present, and the file
/// name is part of the key, so one publisher's different executables never match. A file name
/// alone is never enough.
pub fn fingerprint(
    file_lower: &str,
    game: Option<&GameMatch>,
    version: Option<&VersionInfo>,
) -> Option<String> {
    if let Some(id) = title_id(game) {
        return Some(format!("title:{id}"));
    }
    let present = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase)
    };
    let company = present(version.and_then(|v| v.company.as_deref()))?;
    let product = present(version.and_then(|v| v.product.as_deref()))?;
    Some(format!(
        "product:{company}|{product}|{}",
        file_lower.to_lowercase()
    ))
}

fn riot_identity(path: &Path, file_name: &str) -> Option<AppIdentity> {
    let in_directory = |directory: &str| {
        path.ancestors().skip(1).any(|parent| {
            parent
                .file_name()
                .and_then(|part| part.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(directory))
        })
    };
    let (id, name, category) = match file_name {
        "league of legends.exe" if in_directory("League of Legends") && in_directory("Game") => {
            ("league-of-legends", "League of Legends", Category::Games)
        }
        "leagueclient.exe" | "leagueclientux.exe" if in_directory("League of Legends") => {
            ("league-client", "League of Legends Client", Category::Other)
        }
        "valorant-win64-shipping.exe"
            if in_directory("VALORANT") && in_directory("ShooterGame") =>
        {
            ("valorant", "VALORANT", Category::Games)
        }
        "valorant.exe" if in_directory("VALORANT") => {
            ("valorant-launcher", "VALORANT Launcher", Category::Other)
        }
        "riotclientservices.exe" | "riotclientux.exe" if in_directory("Riot Client") => {
            ("client", "Riot Client", Category::Other)
        }
        _ => return None,
    };
    Some(AppIdentity {
        key: AppKey::Riot(id).to_string(),
        name: name.into(),
        category,
    })
}

fn is_helper(file_name: &str) -> bool {
    matches!(
        file_name,
        "steamwebhelper.exe"
            | "steamservice.exe"
            | "gameoverlayui.exe"
            | "riotclientcrashhandler.exe"
            | "riotclientuxrender.exe"
            | "leagueclientuxrender.exe"
            | "unitycrashhandler32.exe"
            | "unitycrashhandler64.exe"
            | "crashreportclient.exe"
            | "crashpad_handler.exe"
    )
}

/// No installed apps: the default outside Windows and in tests.
struct NoInstalledApps;

impl InstalledApps for NoInstalledApps {
    fn stamp(&self) -> Option<u64> {
        None
    }

    fn read(&self) -> Vec<InstalledApp> {
        Vec::new()
    }
}

#[cfg(not(windows))]
struct NoGameEntries;

#[cfg(not(windows))]
impl GameEntries for NoGameEntries {
    fn entries(&self, _file_name_lower: &str) -> Vec<game_store::GameEntry> {
        Vec::new()
    }
}

#[cfg(not(windows))]
struct NoVersions;

#[cfg(not(windows))]
impl VersionSource for NoVersions {
    fn read(&self, _path: &Path) -> Option<VersionInfo> {
        None
    }
}

#[cfg(not(windows))]
struct NoPackageNames;

#[cfg(not(windows))]
impl PackageNames for NoPackageNames {
    fn display_name(&self, _aumid: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::game_store::GameEntry;
    use super::*;

    fn fallback() -> AppIdentity {
        AppIdentity {
            key: "observed".into(),
            name: "Executable".into(),
            category: Category::Other,
        }
    }

    /// A recognizer that sees no game list entries and no version resources.
    fn empty_recognizer() -> Recognizer {
        Recognizer::with_sources(
            Box::new(FakeGames::default()),
            Box::new(FakeVersions::default()),
            Box::new(FakePackages::default()),
        )
    }

    /// Shell display names keyed by AUMID, counting every lookup.
    #[derive(Default)]
    struct FakePackages {
        names: HashMap<String, String>,
        lookups: Rc<Cell<usize>>,
    }

    impl PackageNames for FakePackages {
        fn display_name(&self, aumid: &str) -> Option<String> {
            self.lookups.set(self.lookups.get() + 1);
            self.names.get(aumid).cloned()
        }
    }

    #[derive(Default)]
    struct FakeGames(HashMap<String, Vec<GameEntry>>);

    impl GameEntries for FakeGames {
        fn entries(&self, file_name_lower: &str) -> Vec<GameEntry> {
            self.0.get(file_name_lower).cloned().unwrap_or_default()
        }
    }

    /// Version resources keyed by lowercased file name, counting every read.
    #[derive(Default)]
    struct FakeVersions {
        by_file: HashMap<String, VersionInfo>,
        reads: Rc<Cell<usize>>,
    }

    impl VersionSource for FakeVersions {
        fn read(&self, path: &Path) -> Option<VersionInfo> {
            self.reads.set(self.reads.get() + 1);
            let name = path.file_name()?.to_str()?.to_lowercase();
            self.by_file.get(&name).cloned()
        }
    }

    fn game(title_id: Option<&str>, folder: Option<&str>, matched: Option<&str>) -> GameEntry {
        GameEntry {
            kind: 1,
            title_id: title_id.map(str::to_owned),
            parent_directory: folder.map(str::to_owned),
            matched_path: matched.map(str::to_owned),
        }
    }

    /// A recognizer with the given game list entries and version resources, keyed by lowercased
    /// file name. The returned counter is the number of version reads.
    fn with_fakes(
        games: &[(&str, GameEntry)],
        versions: &[(&str, VersionInfo)],
    ) -> (Recognizer, Rc<Cell<usize>>) {
        let mut by_file: HashMap<String, Vec<GameEntry>> = HashMap::new();
        for (file, entry) in games {
            by_file
                .entry((*file).to_owned())
                .or_default()
                .push(entry.clone());
        }
        let reads = Rc::new(Cell::new(0));
        let fake_versions = FakeVersions {
            by_file: versions
                .iter()
                .map(|(file, info)| ((*file).to_owned(), info.clone()))
                .collect(),
            reads: Rc::clone(&reads),
        };
        let recognizer = Recognizer::with_sources(
            Box::new(FakeGames(by_file)),
            Box::new(fake_versions),
            Box::new(FakePackages::default()),
        );
        (recognizer, reads)
    }

    /// The fallback that `resolve` builds for an executable.
    fn exe_fallback(path: &str) -> AppIdentity {
        AppIdentity {
            key: format!("exe:{path}"),
            name: Path::new(path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            category: Category::Other,
        }
    }

    fn version(product: Option<&str>, description: Option<&str>) -> VersionInfo {
        VersionInfo {
            company: None,
            product: product.map(str::to_owned),
            description: description.map(str::to_owned),
        }
    }

    #[test]
    fn spotify_process_and_package_share_a_library_entry() {
        let mut recognizer = empty_recognizer();
        let executable = recognizer
            .identify_executable(
                Path::new("C:/Users/Example/AppData/Roaming/Spotify/Spotify.exe"),
                fallback(),
            )
            .unwrap();
        let package =
            recognizer.identify_package("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", fallback());
        assert_eq!(executable, package);
        assert_eq!(
            recognizer.identify_package("Unknown_Publisher!Spotify", fallback()),
            fallback()
        );
    }

    #[test]
    fn riot_games_clients_and_helpers_have_distinct_roles() {
        let mut recognizer = empty_recognizer();
        let cases = [
            (
                "Riot Games/League of Legends/Game/League of Legends.exe",
                "riot:league-of-legends",
                Category::Games,
            ),
            (
                "Riot Games/League of Legends/LeagueClientUx.exe",
                "riot:league-client",
                Category::Other,
            ),
            (
                "Riot Games/VALORANT/live/ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe",
                "riot:valorant",
                Category::Games,
            ),
            (
                "Riot Games/VALORANT/live/VALORANT.exe",
                "riot:valorant-launcher",
                Category::Other,
            ),
            (
                "Riot Games/Riot Client/RiotClientServices.exe",
                "riot:client",
                Category::Other,
            ),
            (
                "Other/VALORANT-Win64-Shipping.exe",
                "observed",
                Category::Other,
            ),
        ];
        for (path, key, category) in cases {
            let app = recognizer
                .identify_executable(Path::new(path), fallback())
                .unwrap();
            assert_eq!(app.key, key);
            assert_eq!(app.category, category);
        }
        assert!(
            recognizer
                .identify_executable(
                    Path::new("Riot Games/League of Legends/LeagueClientUxRender.exe"),
                    fallback(),
                )
                .is_none()
        );
    }

    fn identify(recognizer: &mut Recognizer, path: &str) -> AppIdentity {
        recognizer
            .identify_executable(Path::new(path), exe_fallback(path))
            .unwrap()
    }

    #[test]
    fn a_catalogued_game_takes_its_name_and_the_games_category() {
        let (mut recognizer, _) =
            with_fakes(&[("hades.exe", game(Some("1145360"), None, None))], &[]);
        let path = r"D:\Epic\Hades\Hades.exe";
        let app = identify(&mut recognizer, path);
        assert_eq!(app.key, exe_fallback(path).key);
        assert_eq!(app.name, "Hades");
        assert_eq!(app.category, Category::Games);
    }

    #[test]
    fn a_folder_hint_names_the_game_ahead_of_the_version_product() {
        let (mut recognizer, _) = with_fakes(
            &[(
                "rdr2.exe",
                game(Some("1174180"), Some("Red Dead Redemption 2"), None),
            )],
            &[("rdr2.exe", version(Some("Rockstar Launcher"), None))],
        );
        let app = identify(&mut recognizer, r"C:\Games\Red Dead Redemption 2\RDR2.exe");
        assert_eq!(app.name, "Red Dead Redemption 2");
        assert_eq!(app.category, Category::Games);
    }

    #[test]
    fn a_matched_path_without_a_title_stays_other_until_fullscreen_is_confirmed() {
        let path = r"C:\Program Files\PadTool\PadTool.exe";
        let (mut recognizer, _) = with_fakes(&[("padtool.exe", game(None, None, Some(path)))], &[]);
        let before = identify(&mut recognizer, path);
        assert_eq!(before, exe_fallback(path));
        assert_eq!(before.category, Category::Other);

        recognizer.confirm_fullscreen(&exe_fallback(path).key);
        let after = identify(&mut recognizer, path);
        assert_eq!(after.key, exe_fallback(path).key);
        assert_eq!(after.name, "PadTool");
        assert_eq!(after.category, Category::Games);
    }

    #[test]
    fn remember_fullscreen_trusts_a_candidate_like_confirm_does() {
        let path = r"C:\Program Files\PadTool\PadTool.exe";
        let (mut recognizer, _) = with_fakes(&[("padtool.exe", game(None, None, Some(path)))], &[]);
        recognizer.remember_fullscreen([exe_fallback(path).key]);
        assert_eq!(identify(&mut recognizer, path).category, Category::Games);
    }

    #[test]
    fn a_filename_only_entry_stays_other_even_after_fullscreen_is_confirmed() {
        let path = r"C:\Tools\engine.exe";
        let (mut recognizer, _) = with_fakes(&[("engine.exe", game(None, None, None))], &[]);
        recognizer.confirm_fullscreen(&exe_fallback(path).key);
        assert_eq!(identify(&mut recognizer, path), exe_fallback(path));
    }

    #[test]
    fn league_client_keeps_its_riot_identity_even_when_the_game_list_has_it() {
        let path = r"C:\Riot Games\League of Legends\LeagueClientUx.exe";
        let (mut recognizer, _) =
            with_fakes(&[("leagueclientux.exe", game(Some("1"), None, None))], &[]);
        recognizer.confirm_fullscreen(&exe_fallback(path).key);
        let app = identify(&mut recognizer, path);
        assert_eq!(app.key, "riot:league-client");
        assert_eq!(app.category, Category::Other);
    }

    #[test]
    fn a_steam_library_game_keeps_its_steam_identity_even_when_the_game_list_has_it() {
        let directory = TempDirectory::new();
        let steamapps = directory.0.join("steamapps");
        fs::create_dir_all(steamapps.join("common/A Game")).unwrap();
        fs::write(
            steamapps.join("appmanifest_123.acf"),
            r#""AppState" { "appid" "123" "name" "A Game" "installdir" "A Game" }"#,
        )
        .unwrap();
        let path = steamapps.join("common/A Game/AGame.exe");
        let path_text = path.to_string_lossy().into_owned();
        let (mut recognizer, _) = with_fakes(&[("agame.exe", game(Some("9999"), None, None))], &[]);
        let app = recognizer
            .identify_executable(&path, exe_fallback(&path_text))
            .unwrap();
        assert_eq!(app.key, "steam:123");
        assert_eq!(app.name, "A Game");
        assert_eq!(app.category, Category::Games);
    }

    #[test]
    fn an_executable_without_an_entry_is_unchanged() {
        let mut recognizer = empty_recognizer();
        let found = recognizer
            .identify_executable(Path::new(r"C:\Tools\Something\thing.exe"), fallback())
            .unwrap();
        assert_eq!(found, fallback());
    }

    #[test]
    fn unreal_layout_names_the_project_folder_above_the_binaries() {
        let file = "fsd-win64-shipping.exe";
        let (mut recognizer, _) = with_fakes(
            &[(file, game(Some("1"), None, None))],
            &[(file, version(Some("FSD"), None))],
        );
        let forward = "D:/Games/Deep Rock Galactic/FSD/Binaries/Win64/FSD-Win64-Shipping.exe";
        assert_eq!(
            identify(&mut recognizer, forward).name,
            "Deep Rock Galactic"
        );
        let backslashes = r"D:\Games\Deep Rock Galactic\FSD\Binaries\Win64\FSD-Win64-Shipping.exe";
        assert_eq!(
            identify(&mut recognizer, backslashes).name,
            "Deep Rock Galactic"
        );
    }

    #[test]
    fn trademark_signs_are_removed_from_names_and_empty_names_are_skipped() {
        let (mut recognizer, _) = with_fakes(
            &[("example.exe", game(Some("1"), Some("Example Game"), None))],
            &[],
        );
        let app = identify(&mut recognizer, r"D:\Games\Example Game®\run\Example.exe");
        assert_eq!(app.name, "Example Game");

        let (mut recognizer, _) = with_fakes(
            &[("thing.exe", game(Some("1"), None, None))],
            &[(
                "thing.exe",
                version(Some("Real Name ©"), Some("Described Game")),
            )],
        );
        assert_eq!(
            identify(&mut recognizer, r"D:\Games\Folder\Thing.exe").name,
            "Real Name"
        );

        let (mut recognizer, _) = with_fakes(
            &[("thing.exe", game(Some("1"), None, None))],
            &[("thing.exe", version(Some("  ™ "), Some("Described Game")))],
        );
        assert_eq!(
            identify(&mut recognizer, r"D:\Games\Folder\Thing.exe").name,
            "Described Game"
        );
    }

    #[test]
    fn a_build_variant_folder_is_not_a_name() {
        let (mut recognizer, _) = with_fakes(&[("witcher.exe", game(Some("1"), None, None))], &[]);
        assert_eq!(
            identify(
                &mut recognizer,
                r"D:\Games\Witcher\bin\x64_dx12\witcher.exe"
            )
            .name,
            "witcher"
        );
    }

    #[test]
    fn a_version_name_equal_to_the_exe_stem_is_skipped_for_the_next_rule() {
        let path = r"D:\Games\Installed\Tool.exe";
        let (mut recognizer, _) = with_fakes(
            &[("tool.exe", game(Some("1"), None, None))],
            &[("tool.exe", version(Some("TOOL"), Some("Real Name")))],
        );
        assert_eq!(identify(&mut recognizer, path).name, "Real Name");

        let (mut recognizer, _) = with_fakes(
            &[("tool.exe", game(Some("1"), None, None))],
            &[("tool.exe", version(Some("tool"), None))],
        );
        assert_eq!(identify(&mut recognizer, path).name, "Installed");
    }

    #[test]
    fn version_info_is_read_once_per_identification_that_needs_a_name() {
        let (mut recognizer, reads) = with_fakes(
            &[
                ("hades.exe", game(Some("1145360"), None, None)),
                (
                    "padtool.exe",
                    game(None, None, Some(r"C:\Tools\PadTool\PadTool.exe")),
                ),
            ],
            &[],
        );
        // An ordinary executable takes its name from the version resource.
        identify(&mut recognizer, r"C:\Tools\plain.exe");
        assert_eq!(reads.get(), 1);

        // An untrusted candidate falls through to the same ordinary path.
        identify(&mut recognizer, r"C:\Tools\PadTool\PadTool.exe");
        assert_eq!(reads.get(), 2);

        // A trusted game reads the version resource once, for its fallback name.
        identify(&mut recognizer, r"D:\Epic\Hades\Hades.exe");
        assert_eq!(reads.get(), 3);
    }

    fn publisher(company: Option<&str>, product: Option<&str>) -> VersionInfo {
        VersionInfo {
            company: company.map(str::to_owned),
            product: product.map(str::to_owned),
            description: None,
        }
    }

    #[test]
    fn a_title_id_wins_over_the_version_fields() {
        let game = GameMatch {
            title_id: Some("1145360".into()),
            folder_name: None,
        };
        let version = publisher(Some("Acme"), Some("Widget"));
        assert_eq!(
            fingerprint("hades.exe", Some(&game), Some(&version)).as_deref(),
            Some("title:1145360")
        );
    }

    #[test]
    fn a_product_fingerprint_needs_both_company_and_product() {
        assert_eq!(
            fingerprint("tool.exe", None, Some(&publisher(Some("Acme"), None))),
            None
        );
        assert_eq!(
            fingerprint("tool.exe", None, Some(&publisher(None, Some("Widget")))),
            None
        );
        assert_eq!(fingerprint("tool.exe", None, None), None);
    }

    #[test]
    fn blank_fields_do_not_count_as_evidence() {
        let blank = publisher(Some("  "), Some("Widget"));
        assert_eq!(fingerprint("tool.exe", None, Some(&blank)), None);
        let blank_title = GameMatch {
            title_id: Some("  ".into()),
            folder_name: None,
        };
        assert_eq!(fingerprint("tool.exe", Some(&blank_title), None), None);
    }

    #[test]
    fn a_product_fingerprint_is_case_insensitive_and_includes_the_file_name() {
        let version = publisher(Some(" Acme Corp "), Some("Widget"));
        let first = fingerprint("widget.exe", None, Some(&version));
        assert_eq!(
            first.as_deref(),
            Some("product:acme corp|widget|widget.exe")
        );
        let other_case = publisher(Some("ACME CORP"), Some("widget"));
        assert_eq!(fingerprint("widget.exe", None, Some(&other_case)), first);
        assert_ne!(
            fingerprint("widget-helper.exe", None, Some(&version)),
            first,
            "a different executable of the same product must not match"
        );
    }

    #[test]
    fn the_version_resource_is_not_read_when_a_title_matches() {
        let path = r"D:\Epic\Hades\Hades.exe";
        let (mut recognizer, reads) = with_fakes(
            &[("hades.exe", game(Some("1145360"), None, None))],
            &[("hades.exe", publisher(Some("Acme"), Some("Hades")))],
        );
        assert_eq!(
            recognizer
                .fingerprint_executable(Path::new(path))
                .as_deref(),
            Some("title:1145360")
        );
        assert_eq!(reads.get(), 0);

        let (mut recognizer, reads) = with_fakes(
            &[],
            &[("tool.exe", publisher(Some("Acme"), Some("Widget")))],
        );
        assert_eq!(
            recognizer
                .fingerprint_executable(Path::new(r"C:\Tools\Tool.exe"))
                .as_deref(),
            Some("product:acme|widget|tool.exe")
        );
        assert_eq!(reads.get(), 1);
    }

    fn with_packages(names: &[(&str, &str)]) -> (Recognizer, Rc<Cell<usize>>) {
        let lookups = Rc::new(Cell::new(0));
        let packages = FakePackages {
            names: names
                .iter()
                .map(|(aumid, name)| ((*aumid).to_owned(), (*name).to_owned()))
                .collect(),
            lookups: Rc::clone(&lookups),
        };
        let recognizer = Recognizer::with_sources(
            Box::new(FakeGames::default()),
            Box::new(FakeVersions::default()),
            Box::new(packages),
        );
        (recognizer, lookups)
    }

    #[test]
    fn an_ordinary_executable_takes_its_description_as_its_name() {
        let path = r"C:\Program Files\Microsoft VS Code\Code.exe";
        let (mut recognizer, _) = with_fakes(
            &[],
            &[("code.exe", version(None, Some("Visual Studio Code")))],
        );
        let app = identify(&mut recognizer, path);
        assert_eq!(app.key, exe_fallback(path).key);
        assert_eq!(app.name, "Visual Studio Code");
        assert_eq!(app.category, Category::Other);
    }

    #[test]
    fn filename_categories_only_fill_an_other_fallback() {
        let path = Path::new(r"C:\Tools\Firefox\firefox.exe");
        let (mut recognizer, _) = with_fakes(&[], &[]);
        let fallback = exe_fallback(path.to_str().unwrap());
        let browser = recognizer
            .identify_executable(path, fallback.clone())
            .unwrap();
        assert_eq!(browser.category, Category::Browsers);

        let categorized = AppIdentity {
            category: Category::Productivity,
            ..fallback
        };
        assert_eq!(
            recognizer.identify_executable(path, categorized.clone()),
            Some(categorized)
        );
    }

    #[test]
    fn windows_tools_skip_the_generic_product_name() {
        let generic = "Microsoft® Windows® Operating System";
        let (mut recognizer, _) = with_fakes(
            &[],
            &[(
                "explorer.exe",
                version(Some(generic), Some("Windows Explorer")),
            )],
        );
        assert_eq!(
            identify(&mut recognizer, r"C:\Windows\explorer.exe").name,
            "Windows Explorer"
        );

        let (mut recognizer, _) =
            with_fakes(&[], &[("explorer.exe", version(Some(generic), None))]);
        assert_eq!(
            identify(&mut recognizer, r"C:\Windows\explorer.exe").name,
            "explorer",
            "a generic product with no description keeps the stem"
        );
    }

    #[test]
    fn electron_and_java_descriptions_are_not_names() {
        let (mut recognizer, _) = with_fakes(
            &[],
            &[("notes.exe", version(Some("Acme Notes"), Some("Electron")))],
        );
        assert_eq!(
            identify(&mut recognizer, r"C:\Apps\Notes\notes.exe").name,
            "Acme Notes"
        );

        let (mut recognizer, _) = with_fakes(
            &[],
            &[(
                "javaw.exe",
                version(None, Some("Java(TM) Platform SE binary")),
            )],
        );
        assert_eq!(
            identify(&mut recognizer, r"C:\Java\bin\javaw.exe").name,
            "javaw"
        );
    }

    #[test]
    fn an_executable_without_a_version_keeps_its_stem() {
        let (mut recognizer, _) = with_fakes(&[], &[]);
        assert_eq!(
            identify(&mut recognizer, r"C:\Tools\Thing.exe").name,
            "Thing"
        );
    }

    #[test]
    fn app_name_rejects_what_says_nothing_and_falls_through_to_the_product() {
        let named = |product: Option<&str>, description: Option<&str>| {
            names::executable_name(Some(&version(product, description)))
        };
        assert_eq!(
            named(Some("Acme Tool"), Some("ELECTRON")),
            Some("Acme Tool".into())
        );
        assert_eq!(named(None, Some("MICROSOFT OPERATING SYSTEM")), None);
        assert_eq!(named(None, Some("Java (TM) Runtime")), None);
        assert_eq!(
            named(None, Some("Vesktop")),
            Some("Vesktop".into()),
            "a name that repeats the file name still gives its capitalization"
        );
        assert_eq!(named(None, Some("Acme Tool")), Some("Acme Tool".into()));
        assert_eq!(named(None, Some(&"A".repeat(61))), None);
        assert_eq!(named(None, Some(&"B".repeat(60))), Some("B".repeat(60)));
        assert_eq!(names::executable_name(None), None);
    }

    #[test]
    fn a_store_app_takes_the_shell_display_name_and_keeps_its_key() {
        let aumid = "contoso.tool_8wekyb3d8bbwe!tool";
        let (recognizer, _) = with_packages(&[(aumid, "Contoso Tool")]);
        let fallback = AppIdentity {
            key: format!("package:{aumid}"),
            name: "tool".into(),
            category: Category::Other,
        };
        let app = recognizer.identify_package(aumid, fallback.clone());
        assert_eq!(app.key, fallback.key);
        assert_eq!(app.name, "Contoso Tool");
        assert_eq!(app.category, Category::Other);
    }

    #[test]
    fn a_store_app_without_a_usable_display_name_keeps_the_fallback() {
        let aumid = "contoso.tool_8wekyb3d8bbwe!tool";
        let (recognizer, _) = with_packages(&[(aumid, "   ")]);
        assert_eq!(recognizer.identify_package(aumid, fallback()), fallback());

        let long = "C".repeat(61);
        let (recognizer, _) = with_packages(&[(aumid, long.as_str())]);
        assert_eq!(recognizer.identify_package(aumid, fallback()), fallback());

        let (recognizer, lookups) = with_packages(&[]);
        assert_eq!(recognizer.identify_package(aumid, fallback()), fallback());
        assert_eq!(lookups.get(), 1);
    }

    #[test]
    fn a_family_name_without_an_app_part_skips_the_lookup() {
        let family = "contoso.tool_8wekyb3d8bbwe";
        let (recognizer, lookups) = with_packages(&[(family, "Contoso Tool")]);
        assert_eq!(recognizer.identify_package(family, fallback()), fallback());
        assert_eq!(lookups.get(), 0);
    }

    #[test]
    fn a_known_product_package_keeps_its_product_identity_without_a_lookup() {
        let (recognizer, lookups) = with_packages(&[]);
        let app =
            recognizer.identify_package("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", fallback());
        assert_eq!(app.category, Category::Media);
        assert_eq!(lookups.get(), 0);
    }

    struct FixedInstalled(Vec<InstalledApp>);

    impl InstalledApps for FixedInstalled {
        fn stamp(&self) -> Option<u64> {
            Some(1)
        }

        fn read(&self) -> Vec<InstalledApp> {
            self.0.clone()
        }
    }

    /// A recognizer with a small catalog, one installed app under `D:\Tools\Obsidian`, and the
    /// given game list entries and version resources.
    fn with_catalog(games: &[(&str, GameEntry)], versions: &[(&str, VersionInfo)]) -> Recognizer {
        let catalog = catalog::build(&[
            ("n:obsidian|obsidian", Category::Productivity),
            ("p:hades", Category::Development),
            ("f:example.notes_abc123", Category::Productivity),
            ("e:hwinfo64.exe|martin malik realix", Category::Utilities),
            ("e:chrome.exe|google", Category::Development),
        ]);
        let installed = FixedInstalled(vec![InstalledApp {
            entry: "bd400747-f0c1-5638-a859-982036102edf".into(),
            display_name: Some("Obsidian 1.7.7".into()),
            publisher: Some("Obsidian".into()),
            folders: vec![r"d:\tools\obsidian".into()],
        }]);
        with_fakes(games, versions)
            .0
            .with_installed(Box::new(installed))
            .with_catalog(catalog)
    }

    #[test]
    fn the_catalog_categorizes_an_executable_through_its_installed_app() {
        let mut recognizer = with_catalog(&[], &[]);
        let app = identify(&mut recognizer, r"D:\Tools\Obsidian\Obsidian.exe");
        assert_eq!(app.category, Category::Productivity);
        let helper = identify(&mut recognizer, r"D:\Tools\Obsidian\resources\helper.exe");
        assert_eq!(
            helper.category,
            Category::Productivity,
            "the whole folder belongs to it"
        );
        let elsewhere = identify(&mut recognizer, r"D:\Tools\Other\Obsidian.exe");
        assert_eq!(elsewhere.category, Category::Other);
    }

    #[test]
    fn the_catalog_categorizes_a_portable_executable_only_with_its_publisher() {
        let company = |name: &str| VersionInfo {
            company: Some(name.into()),
            ..VersionInfo::default()
        };
        let mut recognizer =
            with_catalog(&[], &[("hwinfo64.exe", company("Martin Malik - REALiX"))]);
        let app = identify(&mut recognizer, r"C:\Portable\HWiNFO\HWiNFO64.exe");
        assert_eq!(app.category, Category::Utilities);

        let mut recognizer = with_catalog(&[], &[("hwinfo64.exe", company("Someone Else"))]);
        let app = identify(&mut recognizer, r"C:\Portable\HWiNFO\HWiNFO64.exe");
        assert_eq!(app.category, Category::Other);
    }

    #[test]
    fn stronger_evidence_wins_over_the_catalog() {
        let google = VersionInfo {
            company: Some("Google LLC".into()),
            ..VersionInfo::default()
        };
        let mut recognizer = with_catalog(
            &[("hades.exe", game(Some("1145360"), None, None))],
            &[("chrome.exe", google)],
        );
        let game = identify(&mut recognizer, r"D:\Games\Hades\Hades.exe");
        assert_eq!(game.category, Category::Games, "the game list wins");
        let browser = identify(
            &mut recognizer,
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        );
        assert_eq!(
            browser.category,
            Category::Browsers,
            "the built-in names win"
        );
    }

    #[test]
    fn the_catalog_categorizes_a_store_app_by_its_family() {
        let recognizer = with_catalog(&[], &[]);
        for package in ["Example.Notes_abc123!App", "example.notes_abc123"] {
            assert_eq!(
                recognizer.identify_package(package, fallback()).category,
                Category::Productivity,
                "{package}"
            );
        }
        let edited = AppIdentity {
            category: Category::Media,
            ..fallback()
        };
        assert_eq!(
            recognizer
                .identify_package("Example.Notes_abc123!App", edited)
                .category,
            Category::Media,
            "a category already set stays"
        );
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gta-recognition-{}-{stamp}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

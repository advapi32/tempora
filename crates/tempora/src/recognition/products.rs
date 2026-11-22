use crate::model::{AppIdentity, AppKey, Category};

const SPOTIFY_FAMILY: &str = "spotifyab.spotifymusic_zpdnekdrzrea0";

pub(crate) fn known_product_for_key(key: &str) -> Option<AppIdentity> {
    match AppKey::parse(key) {
        AppKey::Package(package) => for_package(package),
        AppKey::Exe(path) => for_executable(path),
        _ => None,
    }
}

pub(super) fn for_package(package: &str) -> Option<AppIdentity> {
    let family = match package.split_once('!') {
        Some((family, app)) if app.eq_ignore_ascii_case("Spotify") => family,
        Some(_) => return None,
        None => package,
    };
    family.eq_ignore_ascii_case(SPOTIFY_FAMILY).then(spotify)
}

pub(super) fn for_executable(path: &str) -> Option<AppIdentity> {
    let path = path.to_ascii_lowercase();
    let mut parts = path.rsplit(['\\', '/']);
    let executable = parts.next()?;
    let directory = parts.next()?;
    match executable {
        "spotify.exe" => {
            let parent = parts.next()?;
            let is_desktop_install = directory == "spotify"
                && (matches!(parent, "program files" | "program files (x86)")
                    || (matches!(parent, "roaming" | "local") && parts.next() == Some("appdata")));
            let is_store_install =
                parent == "windowsapps" && is_spotify_package_directory(directory);
            (is_desktop_install || is_store_install).then(spotify)
        }
        // The Steam client: its bootstrapper and the CEF helper that owns its window.
        // Recognized before the helper filter, as its own app, so it never extends a game.
        "steam.exe" | "steamwebhelper.exe"
            if path.split(['\\', '/']).any(|part| part == "steam") =>
        {
            Some(app("product:steam", "Steam", Category::Other))
        }
        "discord.exe" | "discordptb.exe" | "discordcanary.exe" => {
            let version = directory.strip_prefix("app-")?;
            if !is_version(version) {
                return None;
            }
            let product_directory = parts.next()?;
            if parts.next() != Some("local") || parts.next() != Some("appdata") {
                return None;
            }
            let (key, name) = match (executable, product_directory) {
                ("discord.exe", "discord") => ("product:discord", "Discord"),
                ("discordptb.exe", "discordptb") => ("product:discord-ptb", "Discord PTB"),
                ("discordcanary.exe", "discordcanary") => {
                    ("product:discord-canary", "Discord Canary")
                }
                _ => return None,
            };
            Some(app(key, name, Category::Communication))
        }
        _ => None,
    }
}

fn is_version(version: &str) -> bool {
    version.contains('.')
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_spotify_package_directory(directory: &str) -> bool {
    let mut fields = directory.split('_');
    matches!(
        (fields.next(), fields.next(), fields.next(), fields.next(), fields.next(), fields.next()),
        (Some("spotifyab.spotifymusic"), Some(version), Some("x86" | "x64" | "arm64" | "neutral"), Some(""), Some("zpdnekdrzrea0"), None)
            if is_version(version)
    )
}

fn spotify() -> AppIdentity {
    app("product:spotify", "Spotify", Category::Media)
}

fn app(key: &str, name: &str, category: Category) -> AppIdentity {
    AppIdentity {
        key: key.into(),
        name: name.into(),
        category,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_store_and_desktop_observations_resolve_to_one_product() {
        for key in [
            "package:SpotifyAB.SpotifyMusic_zpdnekdrzrea0",
            "package:spotifyab.spotifymusic_zpdnekdrzrea0!spotify",
            r"exe:C:\Program Files\WindowsApps\SpotifyAB.SpotifyMusic_1.309.2055.0_x64__zpdnekdrzrea0\Spotify.exe",
            r"exe:C:\Users\Example\AppData\Roaming\Spotify\Spotify.exe",
        ] {
            assert_eq!(known_product_for_key(key), Some(spotify()));
        }
        for key in [
            "package:SpotifyAB.SpotifyMusic_anotherpublisher!Spotify",
            "package:SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Other",
            r"exe:C:\Downloads\Spotify\Spotify.exe",
            r"exe:C:\Users\Example\AppData\Roaming\Spotify\SpotifyLauncher.exe",
        ] {
            assert_eq!(known_product_for_key(key), None);
        }
    }

    #[test]
    fn discord_updates_share_a_key_within_each_channel() {
        for (directory, executable, key) in [
            ("Discord", "Discord.exe", "product:discord"),
            ("DiscordPTB", "DiscordPTB.exe", "product:discord-ptb"),
            (
                "DiscordCanary",
                "DiscordCanary.exe",
                "product:discord-canary",
            ),
        ] {
            for version in ["1.0.100", "1.0.101"] {
                let path = format!(
                    r"exe:C:\Users\Example\AppData\Local\{directory}\app-{version}\{executable}"
                );
                assert_eq!(known_product_for_key(&path).unwrap().key, key);
            }
        }
        for path in [
            r"exe:C:\Downloads\Discord\app-1.0.101\Discord.exe",
            r"exe:C:\Users\Example\AppData\Local\DiscordCanary\app-1.0.101\Discord.exe",
            r"exe:C:\Users\Example\AppData\Local\Discord\Update.exe",
        ] {
            assert_eq!(known_product_for_key(path), None);
        }
    }

    #[test]
    fn the_steam_client_is_its_own_app() {
        for path in [
            r"C:\Program Files (x86)\Steam\steam.exe",
            r"C:\Program Files (x86)\Steam\bin\cef\cef.win7x64\steamwebhelper.exe",
            r"D:\Steam\steam.exe",
        ] {
            assert_eq!(
                for_executable(path).map(|app| app.key),
                Some("product:steam".into()),
                "{path}"
            );
        }
        assert_eq!(
            for_executable(r"C:\Games\steam.exe"),
            None,
            "only inside a Steam folder"
        );
    }
}

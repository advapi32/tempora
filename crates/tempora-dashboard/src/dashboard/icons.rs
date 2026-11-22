//! App icons extracted from recorded executables. Extraction runs on the worker thread;
//! textures are created on the UI thread and shared with `widgets::icon_tile`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, ColorImage, Context, TextureHandle, TextureOptions};

/// Pixel size icons are extracted at: sharp for a 56 pt tile at 200%.
use tempora::local_fs;

pub(super) const ICON_PIXELS: i32 = 96;

/// Icons by app key. `None` means the app has no usable icon; draw the letter tile.
#[derive(Default)]
pub(super) struct IconCache {
    pub textures: HashMap<String, Option<TextureHandle>>,
    /// Keys a tile asked for that have no entry yet.
    pub wanted: HashSet<String>,
}

pub(super) type SharedIcons = Arc<Mutex<IconCache>>;

fn cache_id() -> egui::Id {
    egui::Id::new("app-icon-cache")
}

pub(super) fn shared(ctx: &Context) -> SharedIcons {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(cache_id(), SharedIcons::default)
            .clone()
    })
}

/// The texture for `key`, or `None` while unknown (recorded as wanted) or unavailable.
pub(super) fn lookup(ctx: &Context, key: &str) -> Option<TextureHandle> {
    let icons = shared(ctx);
    let mut cache = icons.lock().ok()?;
    match cache.textures.get(key) {
        Some(texture) => texture.clone(),
        None => {
            cache.wanted.insert(key.to_owned());
            None
        }
    }
}

/// Takes the keys tiles asked for since the last call.
pub(super) fn take_wanted(ctx: &Context) -> Vec<String> {
    let icons = shared(ctx);
    let Ok(mut cache) = icons.lock() else {
        return Vec::new();
    };
    let wanted: Vec<String> = cache.wanted.drain().collect();
    for key in &wanted {
        // Mark as pending so tiles stop asking; the result replaces it.
        cache.textures.insert(key.clone(), None);
    }
    wanted
}

pub(super) fn store(ctx: &Context, key: String, image: Option<ColorImage>) {
    let texture = image
        .map(|image| ctx.load_texture(format!("app-icon:{key}"), image, TextureOptions::LINEAR));
    if let Ok(mut cache) = shared(ctx).lock() {
        cache.textures.insert(key, texture);
    }
    ctx.request_repaint();
}

/// Forgets every icon, for example after corrections changed which executables an app has.
pub(super) fn clear(ctx: &Context) {
    if let Ok(mut cache) = shared(ctx).lock() {
        cache.textures.clear();
    }
}

/// The executable's icon as RGBA, or `None` when it has none.
pub(super) fn extract(path: &Path) -> Option<ColorImage> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::WindowsAndMessaging::{HICON, PrivateExtractIconsW};

    local_fs::metadata(path).ok()?;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut icon: HICON = null_mut();
    let mut id = 0u32;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the out pointers are valid.
    let count = unsafe {
        PrivateExtractIconsW(
            wide.as_ptr(),
            0,
            ICON_PIXELS,
            ICON_PIXELS,
            &mut icon,
            &mut id,
            1,
            0,
        )
    };
    if count == 0 || count == u32::MAX || icon.is_null() {
        return None;
    }
    icon_image(icon)
}

/// Converts an icon handle to an image at its own size, then destroys the handle.
fn icon_image(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<ColorImage> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, GetObjectW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    let mut info = ICONINFO::default();
    // SAFETY: `icon` is a valid icon handle owned here; destroyed below.
    let result = unsafe {
        if GetIconInfo(icon, &mut info) == 0 {
            DestroyIcon(icon);
            return None;
        }
        // Shell icons come at the system's sizes (48, 256), extracted ones at ICON_PIXELS.
        let mut bitmap = BITMAP::default();
        let measured = !info.hbmColor.is_null()
            && GetObjectW(
                info.hbmColor,
                size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            ) != 0;
        let pixels_wide = if measured { bitmap.bmWidth } else { 0 };
        let size = usize::try_from(pixels_wide).unwrap_or(0);
        let dc = CreateCompatibleDC(null_mut());
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pixels_wide,
                biHeight: -pixels_wide,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; size * size * 4];
        let read = if size == 0 || size > 1024 {
            0
        } else {
            GetDIBits(
                dc,
                info.hbmColor,
                0,
                pixels_wide as u32,
                pixels.as_mut_ptr().cast(),
                &mut header,
                DIB_RGB_COLORS,
            )
        };
        let mut mask = vec![0u8; size * size * 4];
        let has_mask = read != 0
            && !info.hbmMask.is_null()
            && GetDIBits(
                dc,
                info.hbmMask,
                0,
                pixels_wide as u32,
                mask.as_mut_ptr().cast(),
                &mut header,
                DIB_RGB_COLORS,
            ) != 0;
        DeleteDC(dc);
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        DestroyIcon(icon);
        (read != 0).then(|| {
            let has_alpha = pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0);
            let rgba: Vec<u8> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(mask.as_chunks::<4>().0)
                .flat_map(|(bgra, mask)| {
                    // Older icons carry no alpha: an AND-mask pixel that is set is transparent.
                    let alpha = if has_alpha {
                        bgra[3]
                    } else if has_mask && mask[0] != 0 {
                        0
                    } else {
                        255
                    };
                    [bgra[2], bgra[1], bgra[0], alpha]
                })
                .collect();
            (size, rgba)
        })
    };
    let (size, rgba) = result?;
    rgba.as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] != 0)
        .then(|| ColorImage::from_rgba_unmultiplied([size, size], &rgba))
}

/// A Store app's icon as the Start menu shows it, from the shell's Apps folder. Many packaged
/// apps (Photos, Calculator) carry no usable icon in their executables.
pub(super) fn package_icon(package: &str) -> Option<ColorImage> {
    use std::ptr::null_mut;
    use tempora::windows_shell::AppsFolderItem;
    use windows_sys::Win32::UI::Controls::{ILD_TRANSPARENT, ImageList_GetIcon};
    use windows_sys::core::GUID;
    // IID_IImageList from commoncontrols.h; windows-sys declares no COM interface IDs.
    const IID_IIMAGELIST: GUID = GUID::from_u128(0x46eb5926_582e_4017_9fdf_e8998daa0950);
    use windows_sys::Win32::UI::Shell::{
        IUnknown_AtomicRelease, SHGFI_SYSICONINDEX, SHGetImageList, SHIL_EXTRALARGE,
    };

    let aumid = match package.split_once('!') {
        Some(_) => package.to_owned(),
        None => {
            let (root, _) = package_root(package)?;
            let manifest = local_fs::read_to_string(root.join("AppxManifest.xml")).ok()?;
            format!("{package}!{}", manifest_app_id(&manifest)?)
        }
    };
    let item = AppsFolderItem::open(&aumid)?;
    let info = item.file_info(SHGFI_SYSICONINDEX)?;
    // SAFETY: the item keeps COM initialized. The shell's IImageList is usable as a
    // HIMAGELIST, and ImageList_GetIcon returns a separate icon owned by the caller.
    unsafe {
        let mut list = null_mut();
        if SHGetImageList(SHIL_EXTRALARGE as i32, &IID_IIMAGELIST, &mut list) < 0 || list.is_null()
        {
            return None;
        }
        let icon = ImageList_GetIcon(list as _, info.iIcon, ILD_TRANSPARENT);
        IUnknown_AtomicRelease(&mut list);
        if icon.is_null() {
            return None;
        }
        icon_image(icon)
    }
}

/// The Id of the manifest's first `<Application>`.
fn manifest_app_id(manifest: &str) -> Option<String> {
    let tag = manifest.split("<Application ").nth(1)?.split('>').next()?;
    let start = tag.find("Id=\"")? + 4;
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

/// Executables of an installed Store package that may carry its icon, from its key
/// `family[!app]`: the manifest's entry point first (often a launcher stub), then an
/// `.exe` named after the app, then the other executables in the package folder.
pub(super) fn package_executables(package: &str) -> Vec<std::path::PathBuf> {
    package_root(package)
        .map(|(root, app)| {
            let manifest =
                local_fs::read_to_string(root.join("AppxManifest.xml")).unwrap_or_default();
            let mut candidates: Vec<std::path::PathBuf> =
                manifest_executable(&manifest, app.as_deref())
                    .map(|executable| root.join(executable))
                    .into_iter()
                    .collect();
            if let Some(app) = &app {
                candidates.push(root.join(format!("{app}.exe")));
            }
            let mut others: Vec<_> = local_fs::read_dir(&root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                })
                .collect();
            others.sort();
            candidates.extend(others);
            candidates.dedup();
            candidates.retain(|path| local_fs::metadata(path).is_ok());
            candidates
        })
        .unwrap_or_default()
}

/// A Store package's install folder and the app id from its key.
fn package_root(package: &str) -> Option<(std::path::PathBuf, Option<String>)> {
    use std::os::windows::ffi::OsStringExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Storage::Packaging::Appx::{
        GetPackagePathByFullName, GetPackagesByPackageFamily,
    };

    let (family, app) = match package.split_once('!') {
        Some((family, app)) => (family, Some(app)),
        None => (package, None),
    };
    let family: Vec<u16> = family.encode_utf16().chain(std::iter::once(0)).collect();
    let (mut count, mut length) = (0u32, 0u32);
    // SAFETY: sizing call with null buffers, as documented.
    unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            null_mut(),
            &mut length,
            null_mut(),
        )
    };
    if count == 0 || length == 0 {
        return None;
    }
    let mut names = vec![null_mut::<u16>(); count as usize];
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: buffers sized by the previous call; `names` point into `buffer`.
    let result = unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            names.as_mut_ptr(),
            &mut length,
            buffer.as_mut_ptr(),
        )
    };
    if result != 0 || count == 0 {
        return None;
    }
    let full_name = names[0];
    let mut path_length = 0u32;
    // SAFETY: `full_name` is NUL-terminated inside `buffer`, which is still alive.
    unsafe { GetPackagePathByFullName(full_name, &mut path_length, null_mut()) };
    if path_length == 0 {
        return None;
    }
    let mut path = vec![0u16; path_length as usize];
    // SAFETY: `path` has the length the sizing call reported.
    if unsafe { GetPackagePathByFullName(full_name, &mut path_length, path.as_mut_ptr()) } != 0 {
        return None;
    }
    let end = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    let root = std::path::PathBuf::from(std::ffi::OsString::from_wide(&path[..end]));
    Some((root, app.map(str::to_owned)))
}

/// The `Executable` of the manifest's `<Application>` with the given Id, else the first.
fn manifest_executable(manifest: &str, app: Option<&str>) -> Option<String> {
    let mut first = None;
    for part in manifest.split("<Application ").skip(1) {
        let Some(tag) = part.split('>').next() else {
            continue;
        };
        let attribute = |name: &str| {
            let key = format!("{name}=\"");
            let start = tag.find(&key)? + key.len();
            let end = tag[start..].find('"')? + start;
            Some(tag[start..end].to_owned())
        };
        let Some(executable) = attribute("Executable") else {
            continue;
        };
        if app.is_some_and(|app| attribute("Id").is_some_and(|id| id.eq_ignore_ascii_case(app))) {
            return Some(executable);
        }
        first.get_or_insert(executable);
    }
    first
}

/// Executables of an installed Riot product, from Riot Client's own records: the
/// product's install folder in its settings file, or the client's path.
pub(super) fn riot_executables(id: &str) -> Vec<std::path::PathBuf> {
    let Some(data) = std::env::var_os("PROGRAMDATA")
        .map(|path| std::path::PathBuf::from(path).join("Riot Games"))
    else {
        return Vec::new();
    };
    let (product, name) = match id {
        "valorant" | "valorant-launcher" => ("valorant.live", "valorant"),
        "league-of-legends" | "league-client" => ("league_of_legends.live", "league"),
        "client" => {
            let installs =
                local_fs::read_to_string(data.join("RiotClientInstalls.json")).unwrap_or_default();
            return json_string(&installs, "rc_default")
                .map(std::path::PathBuf::from)
                .filter(|path| local_fs::metadata(path).is_ok())
                .into_iter()
                .collect();
        }
        _ => return Vec::new(),
    };
    let settings = data
        .join("Metadata")
        .join(product)
        .join(format!("{product}.product_settings.yaml"));
    let Some(install) = local_fs::read_to_string(settings).ok().and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("product_install_full_path:")?;
            Some(value.trim().trim_matches(['"', '\'']).to_owned())
        })
    }) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut budget = 400;
    collect_executables(std::path::Path::new(&install), 0, &mut budget, &mut found);
    found.sort_by_key(|(path, depth)| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        (
            is_helper_executable(&stem),
            !stem.contains(name),
            *depth,
            path.clone(),
        )
    });
    found.into_iter().map(|(path, _)| path).collect()
}

/// A top-level string value from a small JSON object, without a JSON parser.
fn json_string(text: &str, key: &str) -> Option<String> {
    let start = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = text[start..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(rest[..rest.find('"')?].replace("\\\\", "\\"))
}

/// Executables of a product the recognizer knows by key (`product:<id>`, see
/// `recognition::products`), where its installer puts them. A product recorded under its own
/// key has no executable path in history to take the icon from.
pub(super) fn product_executables(id: &str) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    match id {
        "steam" => steam_root()
            .map(|root| root.join("steam.exe"))
            .into_iter()
            .collect(),
        "spotify" => {
            let mut paths: Vec<PathBuf> = env("APPDATA")
                .map(|data| data.join("Spotify").join("Spotify.exe"))
                .into_iter()
                .collect();
            paths.extend(package_executables(
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            ));
            paths
        }
        "discord" | "discord-ptb" | "discord-canary" => {
            let (folder, exe) = match id {
                "discord" => ("Discord", "Discord.exe"),
                "discord-ptb" => ("DiscordPTB", "DiscordPTB.exe"),
                _ => ("DiscordCanary", "DiscordCanary.exe"),
            };
            // Discord installs each version as app-<version>; the newest sorts last.
            let Some(root) = env("LOCALAPPDATA").map(|data| data.join(folder)) else {
                return Vec::new();
            };
            let mut versions: Vec<PathBuf> = local_fs::read_dir(&root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("app-"))
                })
                .collect();
            versions.sort();
            versions
                .into_iter()
                .rev()
                .map(|dir| dir.join(exe))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Executables of an installed Steam game that may carry its icon, best first: those
/// named like the install folder, shallow before deep, helpers (crash reporters,
/// installers, redistributables) last.
pub(super) fn steam_executables(app_id: &str) -> Vec<std::path::PathBuf> {
    let Some(steam) = steam_root() else {
        return Vec::new();
    };
    for library in steam_libraries(&steam) {
        let steamapps = library.join("steamapps");
        let Ok(manifest) =
            local_fs::read_to_string(steamapps.join(format!("appmanifest_{app_id}.acf")))
        else {
            continue;
        };
        let Some(folder) = vdf_value(&manifest, "installdir") else {
            continue;
        };
        let install = steamapps.join("common").join(&folder);
        let mut found = Vec::new();
        let mut budget = 400;
        collect_executables(&install, 0, &mut budget, &mut found);
        let folder = folder.to_lowercase().replace([' ', '-', '_'], "");
        found.sort_by_key(|(path, depth)| {
            let stem = path
                .file_stem()
                .map(|stem| {
                    stem.to_string_lossy()
                        .to_lowercase()
                        .replace([' ', '-', '_'], "")
                })
                .unwrap_or_default();
            (
                is_helper_executable(&stem),
                !stem.contains(&folder) && !folder.contains(&stem),
                *depth,
                path.clone(),
            )
        });
        return found.into_iter().map(|(path, _)| path).collect();
    }
    Vec::new()
}

fn is_helper_executable(stem: &str) -> bool {
    [
        "crash", "unins", "setup", "redist", "vcredist", "dxsetup", "prereq", "report", "helper",
        "updater",
    ]
    .iter()
    .any(|word| stem.contains(word))
}

/// Executables up to three folders deep, visiting at most `budget` folders.
fn collect_executables(
    folder: &std::path::Path,
    depth: usize,
    budget: &mut usize,
    found: &mut Vec<(std::path::PathBuf, usize)>,
) {
    if depth > 3 || *budget == 0 {
        return;
    }
    *budget -= 1;
    let Ok(entries) = local_fs::read_dir(folder) else {
        return;
    };
    let mut folders = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            folders.push(path);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            found.push((path, depth));
        }
    }
    folders.sort();
    for child in folders {
        collect_executables(&child, depth + 1, budget, found);
    }
}

/// The Steam install folder from the registry, or the default location.
fn steam_root() -> Option<std::path::PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = "Software\\Valve\\Steam"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value: Vec<u16> = "SteamPath"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: NUL-terminated names; `buffer` holds `size` bytes.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    let path = if result == 0 {
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..end]))
    } else {
        std::path::PathBuf::from(r"C:\Program Files (x86)\Steam")
    };
    local_fs::metadata(&path).is_ok().then_some(path)
}

/// The Steam folder and every library listed in `libraryfolders.vdf`.
fn steam_libraries(steam: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut libraries = vec![steam.to_path_buf()];
    if let Ok(text) = local_fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in text.lines() {
            if let Some(path) = vdf_line_value(line, "path") {
                let path = std::path::PathBuf::from(path);
                if !libraries.contains(&path)
                    && local_fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir())
                {
                    libraries.push(path);
                }
            }
        }
    }
    libraries
}

/// The first `"key" "value"` pair in a Valve KeyValues text file.
fn vdf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| vdf_line_value(line, key))
}

fn vdf_line_value(line: &str, key: &str) -> Option<String> {
    let mut quoted = line.split('"').skip(1).step_by(2);
    let found = quoted.next()?;
    if !found.eq_ignore_ascii_case(key) {
        return None;
    }
    Some(quoted.next()?.replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_library_config_ignores_network_locations() {
        let root = std::env::temp_dir().join(format!("gta-icon-libraries-{}", std::process::id()));
        let steamapps = root.join("steamapps");
        let manifest = steamapps.join("libraryfolders.vdf");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(&manifest, r#""path" "\\\\server\\library""#).unwrap();
        let libraries = steam_libraries(&root);
        std::fs::remove_file(manifest).unwrap();
        std::fs::remove_dir(steamapps).unwrap();
        std::fs::remove_dir(&root).unwrap();
        assert_eq!(libraries, [root]);
    }

    #[test]
    fn extracts_a_system_executable_icon() {
        let explorer = Path::new(r"C:\Windows\explorer.exe");
        if !explorer.exists() {
            return;
        }
        let image = extract(explorer).expect("explorer.exe has an icon");
        assert_eq!(image.size, [ICON_PIXELS as usize; 2]);
        assert!(image.pixels.iter().any(|pixel| pixel.a() == 255));
        assert!(extract(Path::new(r"C:\does\not\exist.exe")).is_none());
    }

    #[test]
    fn wanted_keys_are_taken_once_and_then_resolved() {
        let ctx = Context::default();
        assert!(lookup(&ctx, "steam:1").is_none());
        assert_eq!(take_wanted(&ctx), ["steam:1"]);
        assert!(lookup(&ctx, "steam:1").is_none());
        assert!(
            take_wanted(&ctx).is_empty(),
            "pending keys aren't requested again"
        );
        store(
            &ctx,
            "steam:1".into(),
            Some(ColorImage::new([2, 2], vec![egui::Color32::WHITE; 4])),
        );
        assert!(lookup(&ctx, "steam:1").is_some());
    }

    #[test]
    fn manifest_executable_matches_the_application_id() {
        let manifest = r#"<Package><Applications>
            <Application Id="Helper" Executable="helper.exe" EntryPoint="x"/>
            <Application Id="Spotify" Executable="Spotify.exe" EntryPoint="Windows.FullTrustApplication">
            </Application></Applications></Package>"#;
        assert_eq!(
            manifest_executable(manifest, Some("spotify")).as_deref(),
            Some("Spotify.exe")
        );
        assert_eq!(
            manifest_executable(manifest, None).as_deref(),
            Some("helper.exe")
        );
        assert_eq!(manifest_executable("<Package/>", None), None);
    }

    /// Resolves the Spotify Store package where it is installed; skipped elsewhere.
    #[test]
    fn store_packages_resolve_to_an_executable_with_an_icon() {
        let candidates = package_executables("spotifyab.spotifymusic_zpdnekdrzrea0!spotify");
        if candidates.is_empty() {
            return;
        }
        assert!(
            candidates.iter().any(|path| extract(path).is_some()),
            "one of {candidates:?} has an icon"
        );
    }

    #[test]
    fn valve_keyvalues_lines_and_helper_names() {
        let manifest =
            "\"AppState\"\n{\n\t\"appid\"\t\t\"824270\"\n\t\"installdir\"\t\t\"FPSAimTrainer\"\n}";
        assert_eq!(
            vdf_value(manifest, "installdir").as_deref(),
            Some("FPSAimTrainer")
        );
        assert_eq!(
            vdf_line_value("\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"", "path").as_deref(),
            Some(r"D:\SteamLibrary")
        );
        assert!(!is_helper_executable("unrealcefsubprocess"));
        assert!(is_helper_executable("crashreportclient"));
    }

    /// Resolves an installed Steam game where one exists; skipped elsewhere.
    #[test]
    fn installed_steam_games_resolve_to_an_icon() {
        let Some(steam) = steam_root() else {
            return;
        };
        let Some(app_id) = steam_libraries(&steam).iter().find_map(|library| {
            std::fs::read_dir(library.join("steamapps"))
                .ok()?
                .filter_map(Result::ok)
                .find_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.strip_prefix("appmanifest_")?
                        .strip_suffix(".acf")
                        .map(str::to_owned)
                })
        }) else {
            return;
        };
        let candidates = steam_executables(&app_id);
        eprintln!("steam app {app_id}: {:?}", candidates.first());
        if candidates.is_empty() {
            return;
        }
        assert!(
            candidates
                .iter()
                .take(5)
                .any(|path| extract(path).is_some()),
            "{candidates:?}"
        );
    }

    #[test]
    fn riot_client_path_reads_from_its_installs_file() {
        let installs = r#"{"associated_client": {}, "rc_default": "C:/Riot Games/Riot Client/RiotClientServices.exe"}"#;
        assert_eq!(
            json_string(installs, "rc_default").as_deref(),
            Some("C:/Riot Games/Riot Client/RiotClientServices.exe")
        );
        assert_eq!(json_string(installs, "rc_live"), None);
        assert!(riot_executables("unknown-product").is_empty());
    }

    /// Resolves VALORANT where Riot Client has installed it; skipped elsewhere.
    #[test]
    fn installed_riot_products_resolve_to_an_icon() {
        let candidates = riot_executables("valorant");
        eprintln!("valorant: {:?}", candidates.first());
        if candidates.is_empty() {
            return;
        }
        assert!(
            candidates
                .iter()
                .take(5)
                .any(|path| extract(path).is_some()),
            "{candidates:?}"
        );
    }

    /// Store apps whose executables carry no usable icon get the Start menu's icon from the
    /// shell. Photos ships with Windows; skipped where it's been removed.
    #[test]
    fn a_store_app_gets_its_icon_from_the_shell() {
        let photos = "microsoft.windows.photos_8wekyb3d8bbwe";
        if package_root(photos).is_none() {
            return;
        }
        let image = package_icon(photos).expect("Photos icon from the shell");
        assert!(
            image.size[0] >= 32 && image.size[0] == image.size[1],
            "{:?}",
            image.size
        );
    }
}

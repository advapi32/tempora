//! Categories for apps published in WinGet, matched by what their installers leave behind.
//!
//! The data file is built from labelled WinGet manifests by the `build_catalog` example, which
//! uses [`Package::keys`] and [`encode`] from here, so lookups and the build make their keys the
//! same way. It holds no names or paths, only a hash per match key and a category:
//!
//! - `TCAT`, a version byte, three zero bytes and the entry count as a little-endian `u32`;
//! - the 64-bit FNV-1a hash of each key, little-endian, sorted ascending;
//! - one category code per hash, in the same order.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::Category;

static EMBEDDED: &[u8] = include_bytes!("../../data/winget-catalog.bin");

const MAGIC: &[u8; 4] = b"TCAT";
const VERSION: u8 = 1;
const HEADER: usize = 12;

/// Category codes in the data file.
const CODES: [Category; 9] = [
    Category::Games,
    Category::Browsers,
    Category::Communication,
    Category::Media,
    Category::Productivity,
    Category::Development,
    Category::Creative,
    Category::Utilities,
    Category::Other,
];

/// Tokens that say which build or architecture a name refers to, not which app.
const ARCHITECTURES: [&str; 8] = [
    "x64", "x86", "arm64", "amd64", "win64", "win32", "64bit", "32bit",
];

/// Company-form words dropped from publishers, so "Spotify AB" and "Spotify" agree.
const COMPANY_FORMS: [&str; 26] = [
    "inc",
    "llc",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "company",
    "gmbh",
    "ag",
    "sa",
    "sas",
    "srl",
    "sro",
    "spa",
    "bv",
    "oy",
    "ab",
    "as",
    "aps",
    "pty",
    "plc",
    "kk",
    "ltda",
    "llp",
    "the",
];

#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    hashes: &'static [[u8; 8]],
    categories: &'static [u8],
}

impl Catalog {
    /// The catalog built into this binary. A damaged file gives an empty catalog.
    pub fn embedded() -> Self {
        Self::parse(EMBEDDED).unwrap_or_else(Self::empty)
    }

    pub fn empty() -> Self {
        Self {
            hashes: &[],
            categories: &[],
        }
    }

    pub fn parse(data: &'static [u8]) -> Option<Self> {
        let header = data.get(..HEADER)?;
        if &header[..4] != MAGIC || header[4] != VERSION {
            return None;
        }
        let count = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
        let body = &data[HEADER..];
        let hash_bytes = count.checked_mul(8)?;
        if body.len() != hash_bytes.checked_add(count)? {
            return None;
        }
        let (hashes, categories) = body.split_at(hash_bytes);
        Some(Self {
            hashes: hashes.as_chunks::<8>().0,
            categories,
        })
    }

    pub fn len(&self) -> usize {
        self.categories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    fn get(&self, key: &str) -> Option<Category> {
        let hash = fnv1a(key.as_bytes());
        let index = self
            .hashes
            .binary_search_by_key(&hash, |bytes| u64::from_le_bytes(*bytes))
            .ok()?;
        CODES.get(usize::from(self.categories[index])).copied()
    }

    /// The category of an installed app, from its Uninstall entry: the entry's key name
    /// (an MSI product code, an Inno Setup app ID or the installer's own name), then its display
    /// name together with its publisher.
    pub fn installed(
        &self,
        entry: &str,
        display_name: Option<&str>,
        publisher: Option<&str>,
    ) -> Option<Category> {
        product_key(entry)
            .and_then(|key| self.get(&key))
            .or_else(|| self.get(&name_key(display_name?, publisher?)?))
    }

    /// The category of a Store app, from its package family.
    pub fn package_family(&self, family: &str) -> Option<Category> {
        self.get(&family_key(family))
    }

    /// The category of a portable app, from its executable's file name. The publisher, from the
    /// version resource, must agree, since a file name alone is ambiguous.
    pub fn executable(&self, file_name: &str, publisher: &str) -> Option<Category> {
        self.get(&executable_key(file_name, publisher)?)
    }
}

/// What the WinGet manifests say about one package, as `scripts/catalog/extract.py` collects it.
#[derive(Debug, Default)]
pub struct Package<'a> {
    pub names: Vec<&'a str>,
    pub publishers: Vec<&'a str>,
    pub product_codes: Vec<&'a str>,
    pub families: Vec<&'a str>,
    /// Paths of the files a portable package installs.
    pub file_paths: Vec<&'a str>,
    /// Declared commands and portable command aliases.
    pub commands: Vec<&'a str>,
}

impl Package<'_> {
    /// Every key the lookups on [`Catalog`] can find this package by.
    pub fn keys(&self) -> BTreeSet<String> {
        let mut keys: BTreeSet<String> = self
            .product_codes
            .iter()
            .filter_map(|code| product_key(code))
            .collect();
        for name in &self.names {
            for publisher in &self.publishers {
                keys.extend(name_key(name, publisher));
            }
        }
        keys.extend(
            self.families
                .iter()
                .filter(|family| !family.trim().is_empty())
                .map(|family| family_key(family)),
        );
        let mut executables: BTreeSet<String> = self
            .file_paths
            .iter()
            .filter_map(|path| path.rsplit(['/', '\\']).next())
            .map(str::to_owned)
            .collect();
        // Declared commands are the executables' own names (pwsh for pwsh.exe), so they name
        // portable copies too. The publisher still has to agree.
        for command in &self.commands {
            let command = command.trim();
            if command.is_empty() || command.contains([' ', '/', '\\']) {
                continue;
            }
            executables.insert(if command.to_lowercase().ends_with(".exe") {
                command.to_owned()
            } else {
                format!("{command}.exe")
            });
        }
        for executable in &executables {
            if !executable.trim().to_lowercase().ends_with(".exe") {
                continue;
            }
            for publisher in &self.publishers {
                keys.extend(executable_key(executable, publisher));
            }
        }
        keys
    }
}

/// The data file for these keys and categories. The caller leaves out keys it can't settle.
pub fn encode<'a>(entries: impl IntoIterator<Item = (&'a str, Category)>) -> Vec<u8> {
    let rows: BTreeMap<u64, u8> = entries
        .into_iter()
        .map(|(key, category)| {
            let code = CODES
                .iter()
                .position(|item| *item == category)
                .expect("CODES lists every category");
            (fnv1a(key.as_bytes()), code as u8)
        })
        .collect();
    let mut data = MAGIC.to_vec();
    data.extend([VERSION, 0, 0, 0]);
    data.extend(
        u32::try_from(rows.len())
            .expect("fewer than 2^32 keys")
            .to_le_bytes(),
    );
    data.extend(rows.keys().flat_map(|hash| hash.to_le_bytes()));
    data.extend(rows.values());
    data
}

fn product_key(entry: &str) -> Option<String> {
    let entry = normalize(entry);
    (!entry.is_empty()).then(|| format!("p:{entry}"))
}

fn name_key(name: &str, publisher: &str) -> Option<String> {
    let name = normalize(name);
    let publisher = normalize_publisher(publisher);
    (!name.is_empty() && !publisher.is_empty()).then(|| format!("n:{name}|{publisher}"))
}

fn family_key(family: &str) -> String {
    format!("f:{}", family.trim().to_lowercase())
}

fn executable_key(file_name: &str, publisher: &str) -> Option<String> {
    let publisher = normalize_publisher(publisher);
    let file_name = file_name.trim().to_lowercase();
    (!file_name.is_empty() && !publisher.is_empty()).then(|| format!("e:{file_name}|{publisher}"))
}

/// Lowercase words of a name without bracketed text, versions or architectures:
/// "7-Zip 23.01 (x64)" and "7-Zip" both become "7 zip".
pub fn normalize(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut depth = 0usize;
    for character in text.to_lowercase().chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            _ if character.is_alphanumeric() || character == '.' => kept.push(character),
            _ => kept.push(' '),
        }
    }
    let mut words = Vec::new();
    for token in kept.split_whitespace() {
        let token = token.trim_matches('.');
        if token.is_empty() || is_version(token) || ARCHITECTURES.contains(&token) {
            continue;
        }
        words.push(token.replace('.', ""));
    }
    words.join(" ")
}

/// [`normalize`] without company forms: "JetBrains s.r.o." becomes "jetbrains".
pub fn normalize_publisher(text: &str) -> String {
    normalize(text)
        .split(' ')
        .filter(|word| !word.is_empty() && !COMPANY_FORMS.contains(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A dotted number such as "23.01" or "v1.2.3".
fn is_version(token: &str) -> bool {
    let number = token.strip_prefix('v').unwrap_or(token);
    number.contains('.')
        && number.bytes().any(|byte| byte.is_ascii_digit())
        && number
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
pub(super) fn build(entries: &[(&str, Category)]) -> Catalog {
    let data = encode(entries.iter().copied());
    Catalog::parse(Box::leak(data.into_boxed_slice())).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMALIZE_CASES: [(&str, &str); 8] = [
        ("7-Zip 23.01 (x64)", "7 zip"),
        ("Mozilla Firefox (x64 en-US)", "mozilla firefox"),
        ("Node.js", "nodejs"),
        ("Python 3.12.4 (64-bit)", "python"),
        (
            "{23170F69-40C1-2702-2301-000001000000}",
            "23170f69 40c1 2702 2301 000001000000",
        ),
        ("Visual Studio Code v1.95.3", "visual studio code"),
        ("Notepad++ [x86]", "notepad"),
        ("  ", ""),
    ];

    #[test]
    fn normalizing_drops_versions_architectures_and_brackets() {
        for (input, expected) in NORMALIZE_CASES {
            assert_eq!(normalize(input), expected, "{input}");
        }
        assert_eq!(normalize_publisher("JetBrains s.r.o."), "jetbrains");
        assert_eq!(normalize_publisher("Microsoft Corporation"), "microsoft");
        assert_eq!(normalize_publisher("Spotify AB"), "spotify");
        assert_eq!(
            normalize_publisher("The Document Foundation"),
            "document foundation"
        );
    }

    #[test]
    fn fnv1a_matches_the_reference_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn lookups_use_the_entry_then_the_name_and_publisher() {
        let catalog = build(&[
            ("p:7 zip", Category::Utilities),
            ("n:obsidian|obsidian", Category::Productivity),
            ("f:mozilla.mozillafirefox_gmpnhwe7bv608", Category::Browsers),
            ("e:hwinfo64.exe|martin malik realix", Category::Utilities),
        ]);
        assert_eq!(catalog.len(), 4);
        assert_eq!(
            catalog.installed("7-Zip", None, None),
            Some(Category::Utilities)
        );
        assert_eq!(
            catalog.installed(
                "bd400747-f0c1-5638-a859-982036102edf",
                Some("Obsidian 1.7.7"),
                Some("Obsidian")
            ),
            Some(Category::Productivity)
        );
        assert_eq!(
            catalog.installed("Unrelated", Some("Obsidian"), None),
            None,
            "a name needs its publisher"
        );
        assert_eq!(
            catalog.package_family("Mozilla.MozillaFirefox_gmpnhwe7bv608"),
            Some(Category::Browsers)
        );
        assert_eq!(
            catalog.executable("HWiNFO64.EXE", "Martin Malik - REALiX"),
            Some(Category::Utilities)
        );
        assert_eq!(catalog.executable("HWiNFO64.exe", "Someone Else"), None);
    }

    #[test]
    fn a_built_package_is_found_by_every_lookup() {
        let package = Package {
            // Tamil vowel signs are combining marks, which once made the build disagree.
            names: vec!["Holy Bible வேதம் 2.1"],
            publishers: vec!["Elroi TheCreator Ltd."],
            product_codes: vec!["{8EB86E9F-9120-43CF-9D1B-2810CD854BCD}_is1"],
            families: vec!["Elroi.Bible_abc123"],
            file_paths: vec![r"bin\Bible.EXE", "readme.txt"],
            commands: vec!["bible-cli", "two words", "dir/tool"],
        };
        let keys = package.keys();
        let catalog = build(
            &keys
                .iter()
                .map(|key| (key.as_str(), Category::Productivity))
                .collect::<Vec<_>>(),
        );
        let found = Some(Category::Productivity);
        assert_eq!(
            catalog.installed("{8EB86E9F-9120-43CF-9D1B-2810CD854BCD}_is1", None, None),
            found
        );
        assert_eq!(
            catalog.installed("Unknown", Some("Holy Bible வேதம்"), Some("Elroi TheCreator")),
            found
        );
        assert_eq!(catalog.package_family("elroi.bible_ABC123"), found);
        assert_eq!(catalog.executable("bible.exe", "Elroi TheCreator"), found);
        assert_eq!(
            catalog.executable("BIBLE-CLI.exe", "Elroi TheCreator"),
            found
        );
        assert_eq!(
            keys.iter().filter(|key| key.starts_with("e:")).count(),
            2,
            "{keys:?}"
        );
    }

    #[test]
    fn damaged_data_is_rejected() {
        assert!(Catalog::parse(b"TCAT").is_none());
        assert!(Catalog::parse(b"XXXX\x01\0\0\0\0\0\0\0").is_none());
        assert!(Catalog::parse(b"TCAT\x01\0\0\0\x02\0\0\0").is_none());
        assert!(
            Catalog::parse(b"TCAT\x01\0\0\0\0\0\0\0")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_embedded_catalog_knows_common_apps() {
        let catalog = Catalog::embedded();
        assert!(catalog.len() > 10_000, "{} entries", catalog.len());
        assert_eq!(
            catalog.installed("7-Zip", Some("7-Zip 24.09 (x64)"), Some("Igor Pavlov")),
            Some(Category::Utilities)
        );
        assert_eq!(
            catalog.installed(
                "Unknown entry",
                Some("Mozilla Firefox (x64 en-US)"),
                Some("Mozilla")
            ),
            Some(Category::Browsers)
        );
        assert_eq!(
            catalog.installed(
                "Unknown entry",
                Some("Audacity 3.7.1"),
                Some("Audacity Team")
            ),
            Some(Category::Creative)
        );
        assert_eq!(
            catalog.executable("pwsh.exe", "Microsoft Corporation"),
            Some(Category::Development),
            "a portable copy matches through the manifest's declared command"
        );
    }
}

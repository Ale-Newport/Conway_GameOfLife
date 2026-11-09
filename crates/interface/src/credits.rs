//! Credits and licences: asset attributions read from the asset folder at
//! startup (`models/*/ATTRIBUTION.txt`, the OTCA `PROVENANCE.md` and the
//! licence fields of the pattern catalogue) plus the software libraries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

/// Root folder of the application assets (resolved by the binary).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct AssetRoot(pub PathBuf);

/// One attribution file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributionFile {
    /// Folder the file belongs to (e.g. `models/kenney`).
    pub source: String,
    /// First line of the file (its title).
    pub title: String,
    /// Full text.
    pub text: String,
}

/// A third-party library and its licence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryCredit {
    /// Crate name and version.
    pub name: &'static str,
    /// SPDX licence expression.
    pub license: &'static str,
    /// Project URL.
    pub url: &'static str,
}

/// Libraries linked into the desktop application's user interface.
pub const LIBRARIES: &[LibraryCredit] = &[
    LibraryCredit { name: "Bevy 0.20.1", license: "MIT OR Apache-2.0", url: "https://bevyengine.org" },
    LibraryCredit { name: "bevy_egui 0.43.1", license: "MIT", url: "https://github.com/vladbat00/bevy_egui" },
    LibraryCredit { name: "egui 0.36", license: "MIT OR Apache-2.0", url: "https://github.com/emilk/egui" },
    LibraryCredit { name: "egui_plot 0.37", license: "MIT OR Apache-2.0", url: "https://github.com/emilk/egui_plot" },
    LibraryCredit {
        name: "memory-stats 1.2",
        license: "MIT OR Apache-2.0",
        url: "https://github.com/Antigroup/memory-stats",
    },
    LibraryCredit { name: "postcard 1.1", license: "MIT OR Apache-2.0", url: "https://github.com/jamesmunns/postcard" },
    LibraryCredit { name: "serde 1.0", license: "MIT OR Apache-2.0", url: "https://serde.rs" },
    LibraryCredit { name: "toml 1.2", license: "MIT OR Apache-2.0", url: "https://github.com/toml-rs/toml" },
];

/// Everything shown in the credits window.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct Credits {
    /// Attribution files of 3D model packs.
    pub attributions: Vec<AttributionFile>,
    /// Provenance and licensing notes of the OTCA metapixel data (Markdown).
    pub otca_provenance: Option<String>,
    /// Licence of each pattern-library entry, with counts.
    pub pattern_licenses: Vec<(String, usize)>,
    /// Files that could not be read.
    pub problems: Vec<String>,
}

fn read_text(path: &Path, problems: &mut Vec<String>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            problems.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// Counts the `license` field of every `[[pattern]]` in a catalogue.
pub fn count_pattern_licenses(catalog_toml: &str) -> Result<Vec<(String, usize)>, String> {
    let value: toml::Table = catalog_toml.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(patterns) = value.get("pattern").and_then(toml::Value::as_array) {
        for pattern in patterns {
            let license = pattern.get("license").and_then(toml::Value::as_str).unwrap_or("unspecified");
            *counts.entry(license.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(out)
}

/// Reads the credits from an asset folder. Missing files are skipped;
/// unreadable ones are listed in [`Credits::problems`].
pub fn load_credits(asset_root: &Path) -> Credits {
    let mut credits = Credits::default();
    let models = asset_root.join("models");
    match std::fs::read_dir(&models) {
        Ok(read) => {
            let mut folders: Vec<PathBuf> =
                read.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
            folders.sort();
            for folder in folders {
                let file = folder.join("ATTRIBUTION.txt");
                if let Some(text) = read_text(&file, &mut credits.problems) {
                    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let title = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(&name).to_string();
                    credits.attributions.push(AttributionFile { source: format!("models/{name}"), title, text });
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => credits.problems.push(format!("{}: {e}", models.display())),
    }
    credits.otca_provenance =
        read_text(&asset_root.join("patterns").join("otca").join("PROVENANCE.md"), &mut credits.problems);
    let catalog = asset_root.join("patterns").join("catalog.toml");
    if let Some(text) = read_text(&catalog, &mut credits.problems) {
        match count_pattern_licenses(&text) {
            Ok(counts) => credits.pattern_licenses = counts,
            Err(e) => credits.problems.push(format!("{}: {e}", catalog.display())),
        }
    }
    credits
}

/// Startup system: loads [`Credits`] from [`AssetRoot`].
pub(crate) fn load_credits_system(root: Option<Res<AssetRoot>>, mut commands: Commands) {
    let credits = match root {
        Some(root) => load_credits(&root.0),
        None => Credits { problems: vec!["the asset folder was not configured".to_string()], ..Default::default() },
    };
    for problem in &credits.problems {
        warn!("credits: {problem}");
    }
    commands.insert_resource(credits);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_licenses() {
        let toml = r#"
[[pattern]]
id = "a"
license = "CC0"
[[pattern]]
id = "b"
license = "GPL"
[[pattern]]
id = "c"
license = "CC0"
[[pattern]]
id = "d"
"#;
        assert_eq!(
            count_pattern_licenses(toml).unwrap(),
            vec![("CC0".to_string(), 2), ("GPL".to_string(), 1), ("unspecified".to_string(), 1)]
        );
        assert!(count_pattern_licenses("not = [toml").is_err());
    }

    #[test]
    fn loads_from_a_folder() {
        let root = std::env::temp_dir().join(format!("emergence-credits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("models/zeta")).unwrap();
        std::fs::create_dir_all(root.join("models/alpha")).unwrap();
        std::fs::create_dir_all(root.join("patterns/otca")).unwrap();
        std::fs::write(root.join("models/zeta/ATTRIBUTION.txt"), "\nZeta pack (CC BY)\nline").unwrap();
        std::fs::write(root.join("models/alpha/ATTRIBUTION.txt"), "Alpha (CC0)").unwrap();
        std::fs::write(root.join("patterns/otca/PROVENANCE.md"), "# OTCA").unwrap();
        std::fs::write(root.join("patterns/catalog.toml"), "[[pattern]]\nlicense = \"X\"\n").unwrap();
        let credits = load_credits(&root);
        let titles: Vec<&str> = credits.attributions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha (CC0)", "Zeta pack (CC BY)"]);
        assert_eq!(credits.otca_provenance.as_deref(), Some("# OTCA"));
        assert_eq!(credits.pattern_licenses, vec![("X".to_string(), 1)]);
        assert!(credits.problems.is_empty());
        let empty = load_credits(&root.join("missing"));
        assert!(empty.attributions.is_empty() && empty.problems.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The real asset folder ships the attributions the credits must show.
    #[test]
    fn repository_assets_have_required_attributions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let credits = load_credits(&root);
        assert!(credits.problems.is_empty(), "{:?}", credits.problems);
        assert!(credits.attributions.iter().any(|a| a.text.contains("CC BY 3.0")), "CC BY credit lines missing");
        assert!(credits.otca_provenance.as_deref().is_some_and(|t| t.contains("Brice Due")));
        assert!(!credits.pattern_licenses.is_empty());
    }
}

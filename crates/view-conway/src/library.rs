//! The pattern library: the asset catalog (`assets/patterns/catalog.toml`,
//! read through the `patterns` crate) or, when it cannot be read, the
//! classics embedded in the `patterns` crate. Categories, search, metadata
//! formatting and a cache of loaded patterns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::*;
use cellular_automata::PatternLibrary;
use interface::AssetRoot;
use patterns::{Catalog, CatalogEntry, Pattern};

/// Where the library came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySource {
    /// The asset catalog at this path.
    Assets(PathBuf),
    /// The embedded classics, because the asset catalog could not be read.
    Builtin {
        /// Why the asset catalog is unavailable.
        reason: String,
    },
}

/// The loaded library and the resolver used to build scenarios.
#[derive(Resource)]
pub struct ConwayLibrary {
    /// The catalog shown in the pattern browser.
    pub catalog: Catalog,
    /// Resolves `Library("id")` sources of scenario files.
    pub resolver: PatternLibrary,
    /// Origin of the catalog.
    pub source: LibrarySource,
    cache: HashMap<String, Result<Arc<Pattern>, String>>,
}

impl Default for ConwayLibrary {
    fn default() -> Self {
        Self::builtin("no asset folder was configured".into())
    }
}

impl ConwayLibrary {
    /// The embedded classics only.
    pub fn builtin(reason: String) -> Self {
        Self {
            catalog: Catalog::builtin(),
            resolver: PatternLibrary::builtin(),
            source: LibrarySource::Builtin { reason },
            cache: HashMap::new(),
        }
    }

    /// Reads `<asset_root>/patterns/catalog.toml`, falling back to the
    /// embedded classics (with the reason) when it cannot be read.
    pub fn load(asset_root: &Path) -> Self {
        let path = asset_root.join("patterns").join("catalog.toml");
        match Catalog::load(&path) {
            Ok(catalog) => Self {
                resolver: PatternLibrary::with_catalog(catalog.clone()),
                catalog,
                source: LibrarySource::Assets(path),
                cache: HashMap::new(),
            },
            Err(e) => {
                warn!("pattern catalog unavailable, using the built-in classics: {e}");
                Self::builtin(e.to_string())
            }
        }
    }

    /// Entry by id.
    pub fn entry(&self, id: &str) -> Option<&CatalogEntry> {
        self.catalog.by_id(id)
    }

    /// Loads (once) and returns a pattern, normalised so its bounding box
    /// starts at the origin.
    pub fn pattern(&mut self, id: &str) -> Result<Arc<Pattern>, String> {
        if let Some(cached) = self.cache.get(id) {
            return cached.clone();
        }
        let loaded = match self.catalog.by_id(id) {
            Some(entry) => self
                .catalog
                .load_pattern(entry)
                .map(|p| Arc::new(normalized_keeping_metadata(&p)))
                .map_err(|e| e.to_string()),
            None => Err(format!("no pattern with id `{id}` in the library")),
        };
        self.cache.insert(id.to_string(), loaded.clone());
        loaded
    }

    /// A loaded pattern without loading it.
    pub fn cached(&self, id: &str) -> Option<&Result<Arc<Pattern>, String>> {
        self.cache.get(id)
    }

    /// Categories with their entry counts, in catalog order.
    pub fn categories(&self) -> Vec<(String, usize)> {
        self.catalog.categories().into_iter().map(|c| (c.to_string(), self.catalog.in_category(c).count())).collect()
    }

    /// Ids of the entries in `category` (all when `None`) matching `query`.
    pub fn filtered(&self, category: Option<&str>, query: &str) -> Vec<String> {
        self.catalog
            .entries()
            .iter()
            .filter(|e| category.is_none_or(|c| e.category.as_deref() == Some(c)))
            .filter(|e| matches_query(e, query))
            .map(|e| e.id.clone())
            .collect()
    }
}

/// The pattern with its cells moved so the bounding box starts at `(0, 0)`
/// (metadata kept).
pub fn normalized_keeping_metadata(pattern: &Pattern) -> Pattern {
    match pattern.bounding_box() {
        Some((x0, y0, _, _)) if (x0, y0) != (0, 0) => pattern.translated(-x0, -y0),
        _ => pattern.clone(),
    }
}

/// `Startup`: loads the library from the shell's asset folder.
pub(crate) fn load_library(mut commands: Commands, root: Option<Res<AssetRoot>>) {
    let library = match root {
        Some(root) if !root.0.as_os_str().is_empty() => ConwayLibrary::load(&root.0),
        _ => ConwayLibrary::default(),
    };
    commands.insert_resource(library);
}

/// Whether every whitespace-separated term of `query` occurs (ignoring case)
/// in the entry's id, name, category, kind, description, discoverer or year.
pub fn matches_query(entry: &CatalogEntry, query: &str) -> bool {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return true;
    }
    let haystack = [
        Some(entry.id.as_str()),
        Some(entry.name.as_str()),
        entry.category.as_deref(),
        entry.kind.as_deref(),
        entry.description.as_deref(),
        entry.discoverer.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::to_lowercase)
    .chain(entry.year.map(|y| y.to_string()))
    .collect::<Vec<_>>()
    .join(" ");
    terms.iter().all(|t| haystack.contains(t.as_str()))
}

/// Display name of a category slug (`eaters-reflectors` → `Eaters & reflectors`).
pub fn category_label(slug: &str) -> String {
    match slug {
        "still-lifes" => "Still lifes".into(),
        "eaters-reflectors" => "Eaters & reflectors".into(),
        "misc" => "Miscellaneous".into(),
        other => {
            let text = other.replace(['-', '_'], " ");
            let mut chars = text.chars();
            chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        }
    }
}

/// Display name of a kind (`still_life` → `Still life`).
pub fn kind_label(kind: &str) -> String {
    category_label(kind)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Speed of a moving pattern: the catalog's `speed` text when given, else
/// computed from the displacement per period (`c/4 diagonal`, `2c/5
/// orthogonal`, `(2, 1)/5 oblique`). `None` for patterns that do not move.
pub fn speed_text(entry: &CatalogEntry) -> Option<String> {
    if let Some(speed) = entry.extra.get("speed") {
        return Some(speed.clone());
    }
    let (dx, dy) = entry.displacement?;
    let period = entry.period.filter(|p| *p > 0)?;
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    if ax == 0 && ay == 0 {
        return None;
    }
    let fraction = |d: u64| {
        let g = gcd(d, period);
        let (n, p) = (d / g, period / g);
        if n == 1 { format!("c/{p}") } else { format!("{n}c/{p}") }
    };
    Some(if ax == 0 || ay == 0 {
        format!("{} orthogonal", fraction(ax.max(ay)))
    } else if ax == ay {
        format!("{} diagonal", fraction(ax))
    } else {
        format!("({ax}, {ay})/{period} oblique")
    })
}

/// Period as shown in the browser: `still life` for period 0 still lifes,
/// `—` for patterns without a period, else the number.
pub fn period_text(entry: &CatalogEntry) -> String {
    match (entry.period, entry.kind.as_deref()) {
        (Some(0), Some("still_life")) => "1 (still life)".into(),
        (Some(0) | None, _) => "—".into(),
        (Some(p), _) => p.to_string(),
    }
}

/// Measured behaviour fields of the catalog worth showing, as `(label, value)`.
pub fn measured_facts(entry: &CatalogEntry) -> Vec<(&'static str, String)> {
    const FIELDS: [(&str, &str); 12] = [
        ("population_range", "Population over a period"),
        ("emits", "Emits"),
        ("emits_per_period", "Emitted per period"),
        ("stabilizes_at", "Stabilises at generation"),
        ("final_population", "Final population"),
        ("dies_at", "Dies at generation"),
        ("growth", "Growth"),
        ("growth_period", "Growth period"),
        ("growth_per_period", "Cells added per growth period"),
        ("linear_from", "Linear growth from generation"),
        ("object_population", "Object population (without the demo glider)"),
        ("reflector_period", "Reflector recovery period"),
    ];
    FIELDS.iter().filter_map(|(key, label)| entry.extra.get(*key).map(|v| (*label, v.clone()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> CatalogEntry {
        Catalog::builtin().by_id(id).cloned().expect("built-in entry")
    }

    #[test]
    fn search_matches_every_term() {
        let glider = entry("glider");
        assert!(matches_query(&glider, ""));
        assert!(matches_query(&glider, "GLIDER"));
        assert!(matches_query(&glider, "spaceship glider"));
        assert!(!matches_query(&glider, "glider gun"));
        let gun = entry("gosper-glider-gun");
        assert!(matches_query(&gun, "gosper 1970"));
        assert!(matches_query(&gun, "glider gun"));
    }

    #[test]
    fn library_filters_and_caches() {
        let mut lib = ConwayLibrary::builtin("test".into());
        let categories = lib.categories();
        assert!(categories.iter().any(|(c, n)| c == "spaceships" && *n >= 4), "{categories:?}");
        let spaceships = lib.filtered(Some("spaceships"), "");
        assert!(spaceships.contains(&"glider".to_string()) && !spaceships.contains(&"block".to_string()));
        assert_eq!(lib.filtered(None, "gosper"), vec!["gosper-glider-gun".to_string()]);
        let glider = lib.pattern("glider").expect("loads");
        assert_eq!(glider.population(), 5);
        assert_eq!(glider.bounding_box().map(|b| (b.0, b.1)), Some((0, 0)), "normalised");
        assert!(lib.cached("glider").is_some());
        assert!(lib.pattern("no-such-pattern").is_err());
    }

    #[test]
    fn asset_catalog_loads_when_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let mut lib = ConwayLibrary::load(&root);
        assert!(matches!(lib.source, LibrarySource::Assets(_)), "{:?}", lib.source);
        assert_eq!(lib.catalog.len(), 80);
        let breeder = lib.pattern("breeder-1").expect("Golly breeder loads");
        assert_eq!(breeder.population(), 4060);
        let missing = ConwayLibrary::load(Path::new("/nonexistent/assets"));
        assert!(matches!(missing.source, LibrarySource::Builtin { .. }));
        assert_eq!(missing.catalog.len(), patterns::builtin_patterns().len());
    }

    #[test]
    fn labels_and_speeds() {
        assert_eq!(category_label("still-lifes"), "Still lifes");
        assert_eq!(category_label("eaters-reflectors"), "Eaters & reflectors");
        assert_eq!(category_label("guns"), "Guns");
        assert_eq!(kind_label("still_life"), "Still life");
        let mut e = CatalogEntry { period: Some(4), displacement: Some((1, 1)), ..CatalogEntry::default() };
        assert_eq!(speed_text(&e).as_deref(), Some("c/4 diagonal"));
        e.displacement = Some((-2, 0));
        assert_eq!(speed_text(&e).as_deref(), Some("c/2 orthogonal"));
        e.period = Some(5);
        assert_eq!(speed_text(&e).as_deref(), Some("2c/5 orthogonal"));
        e.displacement = Some((2, 1));
        assert_eq!(speed_text(&e).as_deref(), Some("(2, 1)/5 oblique"));
        e.displacement = Some((0, 0));
        assert_eq!(speed_text(&e), None);
        e.extra.insert("speed".into(), "c/7".into());
        assert_eq!(speed_text(&e).as_deref(), Some("c/7"));
        assert_eq!(period_text(&entry("block")), "1 (still life)");
        assert_eq!(period_text(&entry("pulsar")), "3");
        assert_eq!(period_text(&entry("acorn")), "—");
    }
}

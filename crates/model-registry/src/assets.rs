//! Data files some models need: the pattern catalog, the OTCA metapixel
//! tiles and the species definitions.
//!
//! An [`Assets`] value is created from an optional asset root (the
//! workspace `assets/` folder or a packaged copy). Each data set is loaded
//! on first use and cached, so building many simulations (replicate runs,
//! validation) reads every file once. Without a root, the embedded data of
//! the model crates is used where it exists (classic patterns, built-in
//! species); the metapixel tiles are only distributed as asset files.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cellular_automata::PatternLibrary;
use ecosystem::SpeciesCatalog;
use metapixel::TileSet;

use crate::error::RegistryError;

/// Pattern catalog file, relative to the asset root.
pub const PATTERN_CATALOG: &str = "patterns/catalog.toml";
/// OTCA tile folder, relative to the asset root.
pub const OTCA_TILES: &str = "patterns/otca";
/// Species folder, relative to the asset root.
pub const SPECIES: &str = "species";

/// Lazily loaded, shared model data.
#[derive(Debug, Default)]
pub struct Assets {
    root: Option<PathBuf>,
    patterns: OnceLock<Result<PatternLibrary, String>>,
    tiles: OnceLock<Result<Arc<TileSet>, String>>,
    species: OnceLock<Result<SpeciesCatalog, String>>,
}

impl Assets {
    /// Uses the asset folder `root` (`None`: embedded data only).
    pub fn new(root: Option<PathBuf>) -> Self {
        Self { root, ..Self::default() }
    }

    /// The asset root, if any.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Pattern ids are resolved against the embedded classics and, with an
    /// asset root, `assets/patterns/catalog.toml`.
    ///
    /// # Errors
    /// [`RegistryError::Asset`] if the catalog exists but cannot be read.
    pub fn patterns(&self) -> Result<&PatternLibrary, RegistryError> {
        let loaded = self.patterns.get_or_init(|| match &self.root {
            Some(root) => {
                let path = root.join(PATTERN_CATALOG);
                PatternLibrary::load(&path).map_err(|e| format!("pattern catalog {}: {e}", path.display()))
            }
            None => Ok(PatternLibrary::builtin()),
        });
        loaded.as_ref().map_err(|e| RegistryError::Asset(e.clone()))
    }

    /// The unprogrammed OTCA metapixel tiles (`assets/patterns/otca/`).
    ///
    /// # Errors
    /// [`RegistryError::Asset`] without an asset root or when the tiles are
    /// missing or do not match the published construction.
    pub fn tiles(&self) -> Result<Arc<TileSet>, RegistryError> {
        let loaded = self.tiles.get_or_init(|| match &self.root {
            Some(root) => {
                let dir = root.join(OTCA_TILES);
                TileSet::from_dir(&dir).map(Arc::new).map_err(|e| format!("OTCA tiles in {}: {e}", dir.display()))
            }
            None => Err("the OTCA metapixel tiles are asset files (assets/patterns/otca); \
                         give the asset folder (emergence-cli --assets <DIR>)"
                .to_string()),
        });
        loaded.clone().map_err(RegistryError::Asset)
    }

    /// Species definitions: `assets/species/` with an asset root, otherwise
    /// the copies embedded in the ecosystem crate.
    ///
    /// # Errors
    /// [`RegistryError::Asset`] for unreadable or invalid species files.
    pub fn species(&self) -> Result<&SpeciesCatalog, RegistryError> {
        let loaded = self.species.get_or_init(|| match &self.root {
            Some(root) => {
                let dir = root.join(SPECIES);
                SpeciesCatalog::load_dir(&dir).map_err(|e| format!("species in {}: {e}", dir.display()))
            }
            None => SpeciesCatalog::builtin().map_err(|e| format!("embedded species: {e}")),
        });
        loaded.as_ref().map_err(|e| RegistryError::Asset(e.clone()))
    }
}

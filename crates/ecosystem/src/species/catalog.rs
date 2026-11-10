//! The species catalogue: animals, tree species, vegetation layers and
//! fungal guilds, loaded from RON files and cross-validated.
//!
//! Directory layout (`assets/species/`):
//!
//! ```text
//! animals/<id>.ron     one `Species` per file
//! trees/<id>.ron       one `TreeSpecies` per file
//! vegetation.ron       list of the seven `LayerParams`
//! fungi.ron            `FungiParams`
//! ```
//!
//! [`SpeciesCatalog::builtin`] uses copies embedded with `include_str!`, so
//! the application works without the folder; [`SpeciesCatalog::load_dir`]
//! reads a directory with the same layout (e.g. user-modified species).

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::plants::{FungiParams, LAYER_COUNT, LayerParams, PlantLayer, TreeSpecies};
use super::{Food, HuntingStyle, Representation, Species};
use crate::error::EcosystemError;

/// Largest data file read from disk.
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Largest number of animal species (indices are `u16`).
pub const MAX_SPECIES: usize = 1024;

/// Embedded animal species: `(file name, contents)`, sorted by file name
/// (the order of [`SpeciesCatalog::load_dir`], so indices agree).
pub const BUILTIN_ANIMALS: [(&str, &str); 41] = [
    ("alpaca.ron", include_str!("../../../../assets/species/animals/alpaca.ron")),
    ("ant.ron", include_str!("../../../../assets/species/animals/ant.ron")),
    ("bat.ron", include_str!("../../../../assets/species/animals/bat.ron")),
    ("bear.ron", include_str!("../../../../assets/species/animals/bear.ron")),
    ("bee.ron", include_str!("../../../../assets/species/animals/bee.ron")),
    ("beetle.ron", include_str!("../../../../assets/species/animals/beetle.ron")),
    ("bison.ron", include_str!("../../../../assets/species/animals/bison.ron")),
    ("boar.ron", include_str!("../../../../assets/species/animals/boar.ron")),
    ("bream.ron", include_str!("../../../../assets/species/animals/bream.ron")),
    ("butterfly.ron", include_str!("../../../../assets/species/animals/butterfly.ron")),
    ("deer.ron", include_str!("../../../../assets/species/animals/deer.ron")),
    ("dolphin.ron", include_str!("../../../../assets/species/animals/dolphin.ron")),
    ("duck.ron", include_str!("../../../../assets/species/animals/duck.ron")),
    ("eagle.ron", include_str!("../../../../assets/species/animals/eagle.ron")),
    ("elephant.ron", include_str!("../../../../assets/species/animals/elephant.ron")),
    ("fox.ron", include_str!("../../../../assets/species/animals/fox.ron")),
    ("frog.ron", include_str!("../../../../assets/species/animals/frog.ron")),
    ("heron.ron", include_str!("../../../../assets/species/animals/heron.ron")),
    ("horse.ron", include_str!("../../../../assets/species/animals/horse.ron")),
    ("lion.ron", include_str!("../../../../assets/species/animals/lion.ron")),
    ("lizard.ron", include_str!("../../../../assets/species/animals/lizard.ron")),
    ("manta.ron", include_str!("../../../../assets/species/animals/manta.ron")),
    ("perch.ron", include_str!("../../../../assets/species/animals/perch.ron")),
    ("pike.ron", include_str!("../../../../assets/species/animals/pike.ron")),
    ("rabbit.ron", include_str!("../../../../assets/species/animals/rabbit.ron")),
    ("rat.ron", include_str!("../../../../assets/species/animals/rat.ron")),
    ("raven.ron", include_str!("../../../../assets/species/animals/raven.ron")),
    ("reef_fish.ron", include_str!("../../../../assets/species/animals/reef_fish.ron")),
    ("shark.ron", include_str!("../../../../assets/species/animals/shark.ron")),
    ("sheep.ron", include_str!("../../../../assets/species/animals/sheep.ron")),
    ("small_fish.ron", include_str!("../../../../assets/species/animals/small_fish.ron")),
    ("snake.ron", include_str!("../../../../assets/species/animals/snake.ron")),
    ("songbird.ron", include_str!("../../../../assets/species/animals/songbird.ron")),
    ("spider.ron", include_str!("../../../../assets/species/animals/spider.ron")),
    ("stag.ron", include_str!("../../../../assets/species/animals/stag.ron")),
    ("turtle.ron", include_str!("../../../../assets/species/animals/turtle.ron")),
    ("vulture.ron", include_str!("../../../../assets/species/animals/vulture.ron")),
    ("wasp.ron", include_str!("../../../../assets/species/animals/wasp.ron")),
    ("wolf.ron", include_str!("../../../../assets/species/animals/wolf.ron")),
    ("worm.ron", include_str!("../../../../assets/species/animals/worm.ron")),
    ("zebra.ron", include_str!("../../../../assets/species/animals/zebra.ron")),
];

/// Embedded tree species: `(file name, contents)`, sorted by file name.
pub const BUILTIN_TREES: [(&str, &str); 7] = [
    ("acacia.ron", include_str!("../../../../assets/species/trees/acacia.ron")),
    ("birch.ron", include_str!("../../../../assets/species/trees/birch.ron")),
    ("ceiba.ron", include_str!("../../../../assets/species/trees/ceiba.ron")),
    ("oak.ron", include_str!("../../../../assets/species/trees/oak.ron")),
    ("palm.ron", include_str!("../../../../assets/species/trees/palm.ron")),
    ("pine.ron", include_str!("../../../../assets/species/trees/pine.ron")),
    ("spruce.ron", include_str!("../../../../assets/species/trees/spruce.ron")),
];

/// Embedded vegetation layer parameters.
pub const BUILTIN_VEGETATION: &str = include_str!("../../../../assets/species/vegetation.ron");
/// Embedded fungal guild parameters.
pub const BUILTIN_FUNGI: &str = include_str!("../../../../assets/species/fungi.ron");

/// Parses a RON document, reporting the source name and position on error.
pub(crate) fn parse_ron<T: DeserializeOwned>(text: &str, source: &str) -> Result<T, EcosystemError> {
    ron::from_str(text).map_err(|e| EcosystemError::Parse { source_name: source.to_string(), reason: e.to_string() })
}

/// Reads a text file with a size limit.
pub(crate) fn read_text(path: &Path) -> Result<String, EcosystemError> {
    let io = |e: std::io::Error| EcosystemError::Io { path: path.to_path_buf(), reason: e.to_string() };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_FILE_BYTES {
        return Err(EcosystemError::Io {
            path: path.to_path_buf(),
            reason: format!("{size} bytes exceed the {MAX_FILE_BYTES}-byte limit for data files"),
        });
    }
    std::fs::read_to_string(path).map_err(io)
}

/// Sorted `*.ron` files of a directory.
pub(crate) fn ron_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, EcosystemError> {
    let io = |e: std::io::Error| EcosystemError::Io { path: dir.to_path_buf(), reason: e.to_string() };
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(io)?
        .map(|entry| entry.map(|e| e.path()).map_err(io))
        .collect::<Result<_, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "ron") && p.is_file());
    paths.sort();
    Ok(paths)
}

/// Validated set of species definitions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesCatalog {
    animals: Vec<Species>,
    trees: Vec<TreeSpecies>,
    layers: Vec<LayerParams>,
    fungi: FungiParams,
    /// Row-major `predator × prey` diet preference (0 = does not eat).
    #[serde(skip)]
    diet_matrix: Vec<f32>,
    /// Prey species index of every diet entry (`None` for other foods).
    #[serde(skip)]
    diet_prey: Vec<Vec<Option<u16>>>,
    /// Whether each species hunts individual animals.
    #[serde(skip)]
    hunters: Vec<bool>,
}

impl SpeciesCatalog {
    /// Builds and validates a catalogue.
    pub fn new(
        animals: Vec<Species>,
        trees: Vec<TreeSpecies>,
        layers: Vec<LayerParams>,
        fungi: FungiParams,
    ) -> Result<Self, EcosystemError> {
        let mut catalog =
            Self { animals, trees, layers, fungi, diet_matrix: Vec::new(), diet_prey: Vec::new(), hunters: Vec::new() };
        catalog.validate()?;
        catalog.rebuild_matrix();
        Ok(catalog)
    }

    /// The species embedded in the crate.
    pub fn builtin() -> Result<Self, EcosystemError> {
        let animals = BUILTIN_ANIMALS.iter().map(|(name, text)| parse_ron(text, name)).collect::<Result<_, _>>()?;
        let trees = BUILTIN_TREES.iter().map(|(name, text)| parse_ron(text, name)).collect::<Result<_, _>>()?;
        let layers = parse_ron(BUILTIN_VEGETATION, "vegetation.ron")?;
        let fungi = parse_ron(BUILTIN_FUNGI, "fungi.ron")?;
        Self::new(animals, trees, layers, fungi)
    }

    /// Reads a species directory (see the module documentation).
    pub fn load_dir(dir: &Path) -> Result<Self, EcosystemError> {
        let load = |path: &Path| -> Result<String, EcosystemError> { read_text(path) };
        let animals = ron_files(&dir.join("animals"))?
            .iter()
            .map(|p| parse_ron(&load(p)?, &p.display().to_string()))
            .collect::<Result<Vec<Species>, _>>()?;
        let trees = ron_files(&dir.join("trees"))?
            .iter()
            .map(|p| parse_ron(&load(p)?, &p.display().to_string()))
            .collect::<Result<Vec<TreeSpecies>, _>>()?;
        let vegetation = dir.join("vegetation.ron");
        let layers = parse_ron(&load(&vegetation)?, &vegetation.display().to_string())?;
        let fungi_path = dir.join("fungi.ron");
        let fungi = parse_ron(&load(&fungi_path)?, &fungi_path.display().to_string())?;
        Self::new(animals, trees, layers, fungi)
    }

    /// Checks every record and the cross references.
    fn validate(&mut self) -> Result<(), EcosystemError> {
        if self.animals.len() > MAX_SPECIES {
            return Err(EcosystemError::config(
                "animals",
                format!("{} species exceed {MAX_SPECIES}", self.animals.len()),
            ));
        }
        for (i, species) in self.animals.iter().enumerate() {
            species.validate()?;
            if self.animals[..i].iter().any(|s| s.id == species.id) {
                return Err(EcosystemError::species(&species.id, "id", "is defined twice"));
            }
        }
        for species in &self.animals {
            let mut individual_prey = false;
            for entry in &species.diet {
                let Food::Prey(prey) = &entry.food else { continue };
                let Some(target) = self.animals.iter().find(|s| &s.id == prey) else {
                    return Err(EcosystemError::species(
                        &species.id,
                        "diet",
                        format!("prey `{prey}` is not in the catalogue"),
                    ));
                };
                match (&target.representation, species.is_individual()) {
                    (Representation::Colony(_), _) => {
                        return Err(EcosystemError::species(
                            &species.id,
                            "diet",
                            format!("`{prey}` lives in colonies, which are not modelled as prey"),
                        ));
                    }
                    (Representation::Individual, false) => {
                        return Err(EcosystemError::species(
                            &species.id,
                            "diet",
                            format!("density-field and colony species can only eat density-field prey, not `{prey}`"),
                        ));
                    }
                    (Representation::Individual, true) => individual_prey = true,
                    (Representation::Field(_), _) => {}
                }
            }
            // Insectivores that only eat density-field prey glean it like
            // forage; hunters of individual animals need a technique.
            if individual_prey && species.behaviour.hunting == HuntingStyle::None {
                return Err(EcosystemError::species(
                    &species.id,
                    "behaviour.hunting",
                    "species with individual prey in the diet need a hunting style",
                ));
            }
        }
        for (i, tree) in self.trees.iter().enumerate() {
            tree.validate()?;
            if self.trees[..i].iter().any(|t| t.id == tree.id) {
                return Err(EcosystemError::species(&tree.id, "id", "is defined twice"));
            }
        }
        if self.layers.len() != LAYER_COUNT {
            return Err(EcosystemError::config(
                "vegetation",
                format!("expected {LAYER_COUNT} layers ({}), found {}", layer_keys(), self.layers.len()),
            ));
        }
        // Sort into storage order and require each layer exactly once.
        self.layers.sort_by_key(|l| l.layer.index());
        for (i, layer) in self.layers.iter().enumerate() {
            layer.validate()?;
            if layer.layer.index() != i {
                return Err(EcosystemError::config(
                    "vegetation",
                    format!(
                        "layer `{}` is missing or duplicated (expected {})",
                        PlantLayer::ALL[i].key(),
                        layer_keys()
                    ),
                ));
            }
        }
        self.fungi.validate()
    }

    fn rebuild_matrix(&mut self) {
        let n = self.animals.len();
        self.diet_prey = self
            .animals
            .iter()
            .map(|s| {
                s.diet
                    .iter()
                    .map(|d| match &d.food {
                        Food::Prey(id) => self.animals.iter().position(|a| &a.id == id).map(|i| i as u16),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        self.hunters = self
            .diet_prey
            .iter()
            .map(|prey| prey.iter().flatten().any(|&p| self.animals[usize::from(p)].is_individual()))
            .collect();
        self.diet_matrix = vec![0.0; n * n];
        for (pred, species) in self.animals.iter().enumerate() {
            for entry in &species.diet {
                if let Food::Prey(prey) = &entry.food
                    && let Some(j) = self.animals.iter().position(|s| &s.id == prey)
                {
                    self.diet_matrix[pred * n + j] = entry.preference;
                }
            }
        }
    }

    /// Restores derived lookup tables after deserialisation and re-validates
    /// (snapshots are user input).
    pub(crate) fn rebuild(&mut self) -> Result<(), EcosystemError> {
        self.validate()?;
        self.rebuild_matrix();
        Ok(())
    }

    /// Animal species in catalogue order (index = species index).
    pub fn animals(&self) -> &[Species] {
        &self.animals
    }

    /// Tree species in catalogue order.
    pub fn trees(&self) -> &[TreeSpecies] {
        &self.trees
    }

    /// Vegetation layer parameters in [`PlantLayer`] order.
    pub fn layers(&self) -> &[LayerParams] {
        &self.layers
    }

    /// Parameters of one layer.
    pub fn layer(&self, layer: PlantLayer) -> &LayerParams {
        &self.layers[layer.index()]
    }

    /// Fungal guild parameters.
    pub fn fungi(&self) -> &FungiParams {
        &self.fungi
    }

    /// Animal species by id.
    pub fn animal(&self, id: &str) -> Option<&Species> {
        self.animals.iter().find(|s| s.id == id)
    }

    /// Index of an animal species.
    pub fn animal_index(&self, id: &str) -> Result<u16, EcosystemError> {
        self.animals
            .iter()
            .position(|s| s.id == id)
            .map(|i| i as u16)
            .ok_or_else(|| EcosystemError::UnknownSpecies(id.to_string()))
    }

    /// Index of a tree species.
    pub fn tree_index(&self, id: &str) -> Result<u16, EcosystemError> {
        self.trees
            .iter()
            .position(|s| s.id == id)
            .map(|i| i as u16)
            .ok_or_else(|| EcosystemError::UnknownSpecies(id.to_string()))
    }

    /// Diet preference of `predator` for `prey` (species indices; 0 = not eaten).
    pub fn eats(&self, predator: u16, prey: u16) -> f32 {
        let n = self.animals.len();
        let (p, q) = (usize::from(predator), usize::from(prey));
        if p < n && q < n { self.diet_matrix[p * n + q] } else { 0.0 }
    }

    /// Prey species of diet entry `entry` of `species` (`None` for foods
    /// other than live prey).
    pub fn diet_prey(&self, species: usize, entry: usize) -> Option<u16> {
        self.diet_prey.get(species).and_then(|d| d.get(entry)).copied().flatten()
    }

    /// Whether the species at `index` hunts individual animals (insectivores
    /// that only glean density-field prey do not).
    pub fn hunts_animals(&self, index: u16) -> bool {
        self.hunters.get(usize::from(index)).copied().unwrap_or(false)
    }

    /// Whether any species of the catalogue eats `prey`.
    pub fn has_predators(&self, prey: u16) -> bool {
        (0..self.animals.len()).any(|p| self.eats(p as u16, prey) > 0.0)
    }

    /// Whether the species at `index` is simulated as individual agents.
    pub fn is_individual(&self, index: u16) -> bool {
        self.animals.get(usize::from(index)).is_some_and(Species::is_individual)
    }

    /// Indices of the density-field species, in catalogue order.
    pub fn field_species(&self) -> Vec<u16> {
        self.indices(|s| s.representation.field().is_some())
    }

    /// Indices of the colony species, in catalogue order.
    pub fn colony_species(&self) -> Vec<u16> {
        self.indices(|s| s.representation.colony().is_some())
    }

    fn indices(&self, filter: impl Fn(&Species) -> bool) -> Vec<u16> {
        self.animals.iter().enumerate().filter(|(_, s)| filter(s)).map(|(i, _)| i as u16).collect()
    }

    /// Adds (or replaces) an animal species, re-validating the catalogue.
    pub fn insert_animal(&mut self, species: Species) -> Result<(), EcosystemError> {
        let mut next = self.clone();
        match next.animals.iter().position(|s| s.id == species.id) {
            Some(i) => next.animals[i] = species,
            None => next.animals.push(species),
        }
        next.validate()?;
        next.rebuild_matrix();
        *self = next;
        Ok(())
    }
}

fn layer_keys() -> String {
    PlantLayer::ALL.iter().map(|l| l.key()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalogue_is_valid_and_cross_referenced() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        assert_eq!(catalog.animals().len(), BUILTIN_ANIMALS.len());
        assert_eq!(catalog.trees().len(), BUILTIN_TREES.len());
        let wolf = catalog.animal_index("wolf").unwrap();
        let deer = catalog.animal_index("deer").unwrap();
        let rabbit = catalog.animal_index("rabbit").unwrap();
        assert!(catalog.eats(wolf, deer) > 0.0);
        assert_eq!(catalog.eats(deer, wolf), 0.0);
        assert!(catalog.has_predators(rabbit));
        assert!(!catalog.has_predators(wolf));
        for (i, layer) in catalog.layers().iter().enumerate() {
            assert_eq!(layer.layer.index(), i);
        }
    }

    #[test]
    fn unknown_prey_and_duplicates_are_rejected() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let mut fox = catalog.animal("fox").unwrap().clone();
        fox.diet.push(super::super::DietEntry {
            food: Food::Prey("unicorn".into()),
            preference: 0.5,
            assimilation: None,
        });
        let mut animals = catalog.animals().to_vec();
        animals.push(Species { id: "fox2".into(), ..fox });
        let err =
            SpeciesCatalog::new(animals, catalog.trees().to_vec(), catalog.layers().to_vec(), catalog.fungi().clone())
                .unwrap_err();
        assert!(err.to_string().contains("unicorn"), "{err}");

        let mut animals = catalog.animals().to_vec();
        animals.push(catalog.animal("deer").unwrap().clone());
        let err =
            SpeciesCatalog::new(animals, catalog.trees().to_vec(), catalog.layers().to_vec(), catalog.fungi().clone())
                .unwrap_err();
        assert!(err.to_string().contains("defined twice"), "{err}");

        let mut layers = catalog.layers().to_vec();
        layers.pop();
        let err =
            SpeciesCatalog::new(catalog.animals().to_vec(), catalog.trees().to_vec(), layers, catalog.fungi().clone())
                .unwrap_err();
        assert!(err.to_string().contains("expected 7 layers"), "{err}");
    }

    #[test]
    fn parse_errors_report_position() {
        let err = parse_ron::<Species>("(id: \"x\", name: 3)", "broken.ron").unwrap_err();
        let text = err.to_string();
        assert!(text.starts_with("broken.ron:") && text.contains("1:"), "{text}");
    }
}

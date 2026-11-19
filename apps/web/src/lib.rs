//! # emergence-web
//!
//! The browser demo of Emergence Lab. It compiles the same `cellular-automata`
//! Life engines and the embedded `patterns` library that the desktop
//! **Classic Conway** laboratory uses to WebAssembly, and exposes a small API
//! to the page in `www/`.
//!
//! [`Demo`] is plain Rust so it can be tested natively; [`WebLife`] is the thin
//! `wasm-bindgen` wrapper the page calls.

use cellular_automata::{
    LifeCommand, LifeConfig, LifeMode, LifeSim, PasteMode, PatternLibrary, Rotation, RulePreset, Symmetry,
};
use simulation_core::{ParamValue, Simulation, Topology};
use wasm_bindgen::prelude::*;

/// Largest side the demo accepts, to keep the per-frame cell copy small.
pub const MAX_SIDE: u32 = 1024;

/// A Life world on a torus with the operations the web page needs.
pub struct Demo {
    sim: LifeSim,
    width: u32,
    height: u32,
    library: PatternLibrary,
}

impl Demo {
    /// A `width` × `height` torus running `preset` (machine name, e.g.
    /// `conway`), filled with a random soup of `density` drawn from `seed`.
    ///
    /// # Errors
    /// Sizes outside `1..=MAX_SIDE`, unknown presets and engine errors.
    pub fn new(width: u32, height: u32, preset: &str, density: f64, seed: u64) -> Result<Self, String> {
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return Err(format!("world sides must be 1..={MAX_SIDE}, got {width}×{height}"));
        }
        let preset = parse_preset(preset)?;
        let config =
            LifeConfig { preset, topology: Topology::Torus, width, height, init: vec![], ..LifeConfig::default() };
        let library = PatternLibrary::builtin();
        let sim = LifeSim::new(&config, seed, &library, LifeMode::Classic).map_err(|e| e.to_string())?;
        let mut demo = Self { sim, width, height, library };
        if density > 0.0 {
            demo.randomize(density, seed)?;
        }
        Ok(demo)
    }

    /// World width in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// World height in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Advances `generations` generations and returns how many ran.
    pub fn step(&mut self, generations: u32) -> u64 {
        self.sim.advance(u64::from(generations))
    }

    /// Generations since the world was created.
    pub fn generation(&self) -> u64 {
        self.sim.tick()
    }

    /// Live cells.
    pub fn population(&self) -> u64 {
        self.sim.population()
    }

    /// The engine the automatic selection chose (`bit`, `dense`, ...).
    pub fn engine(&self) -> &'static str {
        self.sim.engine_kind().as_str()
    }

    /// The rule in `B/S` notation.
    pub fn rule(&self) -> String {
        self.sim.rule().to_string()
    }

    /// Selects a rule preset by machine name; it takes effect on the next
    /// generation.
    ///
    /// # Errors
    /// Unknown presets.
    pub fn set_preset(&mut self, preset: &str) -> Result<(), String> {
        parse_preset(preset)?;
        self.sim.set_param("preset", ParamValue::Text(preset.to_string())).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Replaces the world by a random soup.
    ///
    /// # Errors
    /// Densities outside `0..=1`.
    pub fn randomize(&mut self, density: f64, seed: u64) -> Result<(), String> {
        self.command(LifeCommand::Randomize { density, seed: Some(seed), region: None, symmetry: Symmetry::None })
    }

    /// Kills every cell.
    pub fn clear(&mut self) {
        // Clearing a finite world cannot fail.
        let _ = self.command(LifeCommand::Clear);
    }

    /// Sets one cell (coordinates wrap around the torus).
    pub fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        let x = x.rem_euclid(i64::from(self.width));
        let y = y.rem_euclid(i64::from(self.height));
        // In-bounds single-cell edits cannot fail.
        let _ = self.command(LifeCommand::SetCell { x, y, alive });
    }

    /// Clears the world and pastes the embedded pattern `id` in its centre.
    ///
    /// # Errors
    /// Unknown ids and patterns larger than the world.
    pub fn load_pattern(&mut self, id: &str) -> Result<(), String> {
        let pattern = self.library.resolve(id).map_err(|e| e.to_string())?;
        let (w, h) = (pattern.width(), pattern.height());
        if w > u64::from(self.width) || h > u64::from(self.height) {
            return Err(format!("pattern `{id}` is {w}×{h}, larger than the {}×{} world", self.width, self.height));
        }
        self.clear();
        let x = (i64::from(self.width) - w as i64) / 2;
        let y = (i64::from(self.height) - h as i64) / 2;
        self.command(LifeCommand::Paste { pattern, x, y, rotation: Rotation::None, mirror: false, mode: PasteMode::Or })
    }

    /// Writes one byte per cell (row-major, 1 = alive) into `out`, which is
    /// resized to `width × height`.
    pub fn fill_cells(&self, out: &mut Vec<u8>) {
        let (w, h) = (self.width as usize, self.height as usize);
        out.clear();
        out.resize(w * h, 0);
        for (x, y) in self.sim.live_cells() {
            if (0..w as i64).contains(&x) && (0..h as i64).contains(&y) {
                out[y as usize * w + x as usize] = 1;
            }
        }
    }

    fn command(&mut self, command: LifeCommand) -> Result<(), String> {
        self.sim.apply(command).map_err(|e| e.to_string())
    }
}

fn parse_preset(name: &str) -> Result<RulePreset, String> {
    match RulePreset::parse(name) {
        Some(RulePreset::Custom) | None => Err(format!("unknown rule preset `{name}`")),
        Some(preset) => Ok(preset),
    }
}

/// `id|name|category|description` for every embedded pattern, in library
/// order.
pub fn pattern_list() -> Vec<String> {
    patterns::builtin_patterns()
        .iter()
        .map(|p| format!("{}|{}|{}|{}", p.id, p.name, p.category, p.description))
        .collect()
}

/// `machine name|label` for every named rule preset.
pub fn preset_list() -> Vec<String> {
    RulePreset::ALL
        .iter()
        .filter(|p| **p != RulePreset::Custom)
        .map(|p| format!("{}|{}", p.as_str(), p.label()))
        .collect()
}

/// The `wasm-bindgen` face of [`Demo`].
#[wasm_bindgen]
pub struct WebLife {
    demo: Demo,
    cells: Vec<u8>,
}

#[wasm_bindgen]
impl WebLife {
    /// See [`Demo::new`].
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32, preset: &str, density: f64, seed: u64) -> Result<WebLife, JsError> {
        let demo = Demo::new(width, height, preset, density, seed).map_err(|e| JsError::new(&e))?;
        Ok(Self { demo, cells: Vec::new() })
    }

    /// World width.
    pub fn width(&self) -> u32 {
        self.demo.width()
    }

    /// World height.
    pub fn height(&self) -> u32 {
        self.demo.height()
    }

    /// See [`Demo::step`].
    pub fn step(&mut self, generations: u32) -> f64 {
        self.demo.step(generations) as f64
    }

    /// See [`Demo::generation`].
    pub fn generation(&self) -> f64 {
        self.demo.generation() as f64
    }

    /// See [`Demo::population`].
    pub fn population(&self) -> f64 {
        self.demo.population() as f64
    }

    /// See [`Demo::engine`].
    pub fn engine(&self) -> String {
        self.demo.engine().to_string()
    }

    /// See [`Demo::rule`].
    pub fn rule(&self) -> String {
        self.demo.rule()
    }

    /// See [`Demo::set_preset`].
    #[wasm_bindgen(js_name = setPreset)]
    pub fn set_preset(&mut self, preset: &str) -> Result<(), JsError> {
        self.demo.set_preset(preset).map_err(|e| JsError::new(&e))
    }

    /// See [`Demo::randomize`].
    pub fn randomize(&mut self, density: f64, seed: u64) -> Result<(), JsError> {
        self.demo.randomize(density, seed).map_err(|e| JsError::new(&e))
    }

    /// See [`Demo::clear`].
    pub fn clear(&mut self) {
        self.demo.clear();
    }

    /// See [`Demo::set_cell`].
    #[wasm_bindgen(js_name = setCell)]
    pub fn set_cell(&mut self, x: i32, y: i32, alive: bool) {
        self.demo.set_cell(i64::from(x), i64::from(y), alive);
    }

    /// See [`Demo::load_pattern`].
    #[wasm_bindgen(js_name = loadPattern)]
    pub fn load_pattern(&mut self, id: &str) -> Result<(), JsError> {
        self.demo.load_pattern(id).map_err(|e| JsError::new(&e))
    }

    /// One byte per cell, row-major (1 = alive).
    pub fn cells(&mut self) -> Vec<u8> {
        self.demo.fill_cells(&mut self.cells);
        self.cells.clone()
    }
}

/// See [`pattern_list`].
#[wasm_bindgen(js_name = patternList)]
pub fn js_pattern_list() -> Vec<String> {
    pattern_list()
}

/// See [`preset_list`].
#[wasm_bindgen(js_name = presetList)]
pub fn js_preset_list() -> Vec<String> {
    preset_list()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glider_crosses_the_torus_and_keeps_five_cells() {
        let mut demo = Demo::new(32, 32, "conway", 0.0, 1).unwrap();
        demo.load_pattern("glider").unwrap();
        assert_eq!(demo.population(), 5);
        assert_eq!(demo.step(4 * 32), 4 * 32);
        assert_eq!(demo.population(), 5);
        assert_eq!(demo.generation(), 128);
    }

    #[test]
    fn diehard_dies_at_generation_130() {
        let mut demo = Demo::new(128, 128, "conway", 0.0, 1).unwrap();
        demo.load_pattern("diehard").unwrap();
        demo.step(129);
        assert!(demo.population() > 0);
        demo.step(1);
        assert_eq!(demo.population(), 0);
    }

    #[test]
    fn same_seed_gives_the_same_soup() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut x = Demo::new(64, 48, "conway", 0.35, 7).unwrap();
        let mut y = Demo::new(64, 48, "conway", 0.35, 7).unwrap();
        x.step(50);
        y.step(50);
        x.fill_cells(&mut a);
        y.fill_cells(&mut b);
        assert_eq!(a.len(), 64 * 48);
        assert_eq!(a, b);
        assert_eq!(a.iter().map(|&c| u64::from(c)).sum::<u64>(), x.population());
    }

    #[test]
    fn every_embedded_pattern_loads_in_a_256_world() {
        let mut demo = Demo::new(256, 256, "conway", 0.0, 1).unwrap();
        for entry in pattern_list() {
            let id = entry.split('|').next().unwrap();
            demo.load_pattern(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(demo.population() > 0, "{id} pasted no cells");
        }
    }

    #[test]
    fn presets_switch_the_rule_on_the_next_generation() {
        let mut demo = Demo::new(32, 32, "conway", 0.0, 1).unwrap();
        assert_eq!(demo.rule(), "B3/S23");
        demo.set_preset("highlife").unwrap();
        demo.step(1);
        assert_eq!(demo.rule(), "B36/S23");
        assert!(demo.set_preset("custom").is_err());
        assert!(demo.set_preset("nope").is_err());
        assert_eq!(preset_list().len(), RulePreset::ALL.len() - 1);
    }

    #[test]
    fn edits_wrap_and_bad_input_is_refused() {
        let mut demo = Demo::new(16, 16, "conway", 0.0, 1).unwrap();
        demo.set_cell(-1, 16, true);
        let mut cells = Vec::new();
        demo.fill_cells(&mut cells);
        assert_eq!(cells[15], 1);
        assert!(Demo::new(0, 16, "conway", 0.0, 1).is_err());
        assert!(Demo::new(MAX_SIDE + 1, 16, "conway", 0.0, 1).is_err());
        assert!(demo.load_pattern("gosper-glider-gun").is_err());
        assert!(demo.load_pattern("no-such-pattern").is_err());
        assert!(demo.randomize(1.5, 1).is_err());
    }
}

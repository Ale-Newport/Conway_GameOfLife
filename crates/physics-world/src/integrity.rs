//! Structural validation of a deserialised [`World`] (snapshots are user
//! input: a corrupted or hand-edited one must be rejected with an error
//! instead of panicking on an out-of-range index later).

use simulation_core::Grid2;

use crate::calendar::Calendar;
use crate::world::World;

impl World {
    /// Checks the structural consistency of a deserialised world (valid
    /// configuration, matching calendar, every layer `width × height`,
    /// burning-cell indices sorted and inside the map), so that a corrupted
    /// or hand-edited snapshot is rejected instead of panicking later.
    pub(crate) fn check_layout(&self) -> Result<(), String> {
        self.config.validate().map_err(|e| e.to_string())?;
        let (w, h) = (self.config.width, self.config.height);
        if self.calendar != Calendar::from_config(&self.config) {
            return Err("calendar does not match the configuration".to_string());
        }
        let layers: [(&str, usize, usize, usize); 25] = [
            dims("elevation", &self.elevation),
            dims("slope", &self.slope),
            dims("surface", &self.surface),
            dims("biome", &self.biome),
            dims("soil", &self.soil),
            dims("receivers", &self.receivers),
            dims("accumulation", &self.accumulation),
            dims("mean_temperature", &self.climate.mean_temperature),
            dims("precip_factor", &self.climate.precip_factor),
            dims("continental", &self.climate.continental),
            dims("humidity_base", &self.climate.humidity_base),
            dims("water", &self.water),
            dims("water_depth", &self.water_depth),
            dims("snow", &self.snow),
            dims("soil_moisture", &self.soil_moisture),
            dims("temperature", &self.temperature),
            dims("humidity", &self.humidity),
            dims("light", &self.light),
            dims("precipitation", &self.precipitation),
            dims("fertility", &self.fertility),
            dims("nutrients", &self.nutrients),
            dims("organic", &self.organic),
            dims("fuel", &self.fuel),
            dims("canopy", &self.canopy),
            dims("fire", &self.fire),
        ];
        for (name, width, height, len) in layers {
            if (width, height, len) != (w, h, w * h) {
                return Err(format!("layer `{name}` is {width}x{height} ({len} cells), expected {w}x{h}"));
            }
        }
        let n = w * h;
        if self.burning.windows(2).any(|p| p[0] >= p[1]) || self.burning.iter().any(|&i| i as usize >= n) {
            return Err("burning-cell list is not sorted or lies outside the map".to_string());
        }
        Ok(())
    }
}

/// `(name, width, height, buffer length)` of a layer, for
/// `World::check_layout`.
fn dims<T>(name: &'static str, grid: &Grid2<T>) -> (&'static str, usize, usize, usize) {
    (name, grid.width(), grid.height(), grid.as_slice().len())
}

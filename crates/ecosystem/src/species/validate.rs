//! Validation of animal species records.

use super::{Development, Food, Locomotion, Refuge, Representation, Role, SocialStructure, Species};
use crate::error::{EcosystemError, check_id, check_pair, check_range};

impl Species {
    /// Validates the record on its own (cross references are checked by the
    /// catalogue).
    pub fn validate(&self) -> Result<(), EcosystemError> {
        check_id("species", &self.id)?;
        let id = self.id.as_str();
        let err = |field: &str, reason: String| Err(EcosystemError::species(id, field, reason));
        if self.name.trim().is_empty() {
            return err("name", "is empty".into());
        }
        if self.roles.is_empty() {
            return err("roles", "must list at least one role".into());
        }
        if self.diet.is_empty() {
            return err("diet", "must list at least one food".into());
        }
        if self.locomotion.is_empty() {
            return err("locomotion", "must list at least one mode".into());
        }
        self.validate_diet()?;
        self.validate_roles()?;
        self.validate_body_and_motion()?;
        self.validate_life()?;
        self.validate_behaviour()?;
        self.validate_lifestyle()?;
        Ok(())
    }

    fn validate_diet(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        for (i, entry) in self.diet.iter().enumerate() {
            let field = format!("diet[{i}]");
            check_range(id, &field, entry.preference, 1e-6, 1.0)?;
            if let Some(a) = entry.assimilation {
                check_range(id, &format!("{field}.assimilation"), a, 1e-3, 1.0)?;
            }
            if self.diet[..i].iter().any(|d| d.food == entry.food) {
                return Err(EcosystemError::species(id, &field, format!("{:?} is listed twice", entry.food)));
            }
            if let Food::Prey(prey) = &entry.food
                && prey == id
            {
                return Err(EcosystemError::species(
                    id,
                    &field,
                    "a species cannot prey on itself (cannibalism is not modelled)",
                ));
            }
        }
        Ok(())
    }

    fn validate_roles(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        let plants = self.diet.iter().any(|d| matches!(d.food, Food::Plant(_) | Food::Browse | Food::Nectar));
        let requires = |role: Role, ok: bool, what: &str| -> Result<(), EcosystemError> {
            if self.has_role(role) && !ok {
                return Err(EcosystemError::species(id, "roles", format!("role {role:?} requires {what} in the diet")));
            }
            Ok(())
        };
        requires(Role::Herbivore, plants, "plant food")?;
        let meat = self.hunts() || self.diet_entry(&Food::Carrion).is_some() || self.diet_entry(&Food::Eggs).is_some();
        requires(Role::Carnivore, meat, "prey, carrion or eggs")?;
        requires(Role::Omnivore, plants, "plant food")?;
        requires(Role::Predator, self.hunts(), "a prey species")?;
        requires(Role::Scavenger, self.diet_entry(&Food::Carrion).is_some(), "carrion")?;
        requires(Role::Pollinator, self.diet_entry(&Food::Nectar).is_some(), "nectar")?;
        requires(Role::Detritivore, self.diet_entry(&Food::Detritus).is_some(), "detritus")?;
        if self.has_role(Role::Social) && self.has_role(Role::Solitary) {
            return Err(EcosystemError::species(id, "roles", "cannot be both social and solitary"));
        }
        let social = !matches!(self.social.structure, SocialStructure::Solitary);
        if self.has_role(Role::Solitary) && social {
            return Err(EcosystemError::species(
                id,
                "social.structure",
                "solitary species must use structure `solitary`",
            ));
        }
        if self.habitat.aquatic && !self.can(Locomotion::Swim) {
            return Err(EcosystemError::species(id, "habitat.aquatic", "aquatic species must be able to swim"));
        }
        for refuge in &self.habitat.refuges {
            let ok = match refuge {
                Refuge::Burrow => self.can(Locomotion::Burrow),
                Refuge::Trees => self.can(Locomotion::Climb) || self.can(Locomotion::Fly),
                Refuge::Water => self.can(Locomotion::Swim),
                Refuge::DenseVegetation => true,
            };
            if !ok {
                return Err(EcosystemError::species(
                    id,
                    "habitat.refuges",
                    format!("{refuge:?} needs a matching locomotion mode"),
                ));
            }
        }
        Ok(())
    }

    fn validate_body_and_motion(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        check_range(id, "body.mass_kg", self.body.mass_kg, 1e-6, 20_000.0)?;
        check_range(id, "body.length_m", self.body.length_m, 1e-4, 40.0)?;
        check_range(id, "body.body_temperature_c", self.body.body_temperature_c, 0.0, 45.0)?;
        check_range(id, "habitat.max_slope", self.habitat.max_slope, 0.0, 100.0)?;
        check_range(id, "habitat.max_wade_depth_m", self.habitat.max_wade_depth_m, 0.0, 10.0)?;
        check_range(id, "habitat.min_depth_m", self.habitat.min_depth_m, 0.0, 1000.0)?;
        if self.habitat.breeds_in_water && !self.can(Locomotion::Swim) {
            return Err(EcosystemError::species(id, "habitat.breeds_in_water", "needs the `swim` locomotion mode"));
        }
        let m = &self.metabolism;
        check_range(id, "metabolism.field_multiplier", m.field_multiplier, 1.0, 10.0)?;
        if let Some(days) = m.fasting_days {
            check_range(id, "metabolism.fasting_days", days, 0.1, 1000.0)?;
        }
        check_range(id, "metabolism.dehydration_days", m.dehydration_days, 0.1, 1000.0)?;
        check_range(id, "metabolism.food_water", m.food_water, 0.0, 10.0)?;
        check_range(id, "metabolism.intake_per_hour", m.intake_per_hour, 1e-3, 50.0)?;
        let mv = &self.movement;
        if let Some(v) = mv.sprint_speed_m_s {
            check_range(id, "movement.sprint_speed_m_s", v, 1e-4, 100.0)?;
        }
        check_range(id, "movement.cruise_fraction", mv.cruise_fraction, 0.01, 1.0)?;
        check_range(id, "movement.endurance_s", mv.endurance_s, 1.0, 1e6)?;
        check_range(id, "movement.agility", mv.agility, 0.1, 20.0)?;
        check_range(id, "movement.flight_altitude_m", mv.flight_altitude_m, 0.0, 5000.0)?;
        let p = &self.perception;
        check_range(id, "perception.vision_m", p.vision_m, 0.01, 10_000.0)?;
        check_range(id, "perception.fov_deg", p.fov_deg, 1.0, 360.0)?;
        check_range(id, "perception.hearing_m", p.hearing_m, 0.0, 10_000.0)?;
        check_range(id, "perception.smell_m", p.smell_m, 0.0, 10_000.0)?;
        check_pair(id, "climate.preferred_c", self.climate.preferred_c, -80.0, 80.0)?;
        check_pair(id, "climate.tolerance_c", self.climate.tolerance_c, -80.0, 80.0)?;
        let (pl, ph) = self.climate.preferred_c;
        let (tl, th) = self.climate.tolerance_c;
        if tl > pl || th < ph {
            return Err(EcosystemError::species(id, "climate", "the tolerance range must contain the preferred range"));
        }
        let (r, g, b) = self.visual.color;
        for (name, c) in [("r", r), ("g", g), ("b", b)] {
            check_range(id, &format!("visual.color.{name}"), c, 0.0, 1.0)?;
        }
        check_range(id, "visual.scale", self.visual.scale, 0.01, 100.0)?;
        let valid_id = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if let Some(model) = &self.visual.model
            && !valid_id(model)
        {
            return Err(EcosystemError::species(id, "visual.model", format!("`{model}` is not a valid model id")));
        }
        if let Some(rig) = &self.visual.procedural
            && !valid_id(rig)
        {
            return Err(EcosystemError::species(id, "visual.procedural", format!("`{rig}` is not a valid rig id")));
        }
        if self.visual.model.is_none() && self.visual.procedural.is_none() {
            return Err(EcosystemError::species(
                id,
                "visual",
                "needs a manifest `model` or a `procedural` rig id so that renderers can draw it",
            ));
        }
        Ok(())
    }

    fn validate_life(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        check_range(id, "life.lifespan_years", self.life.lifespan_years, 0.01, 500.0)?;
        check_range(id, "life.maturity_days", self.life.maturity_days, 0.1, 100_000.0)?;
        if self.life.maturity_days >= self.life.lifespan_years * 365.0 {
            return Err(EcosystemError::species(
                id,
                "life.maturity_days",
                "maturity must come before the lifespan ends",
            ));
        }
        let r = &self.reproduction;
        match r.development {
            Development::Live { gestation_days } => {
                check_range(id, "reproduction.gestation_days", gestation_days, 0.1, 1000.0)?;
            }
            Development::Eggs { laying_days, incubation_days } => {
                check_range(id, "reproduction.laying_days", laying_days, 0.01, 1000.0)?;
                check_range(id, "reproduction.incubation_days", incubation_days, 0.1, 1000.0)?;
            }
        }
        let (lo, hi) = r.litter;
        if lo == 0 || lo > hi || hi > 10_000 {
            return Err(EcosystemError::species(
                id,
                "reproduction.litter",
                format!("({lo}, {hi}) must satisfy 1 ≤ min ≤ max ≤ 10000"),
            ));
        }
        check_range(id, "reproduction.interval_days", r.interval_days, 0.0, 10_000.0)?;
        check_range(id, "reproduction.parental_care_days", r.parental_care_days, 0.0, 10_000.0)?;
        if let Some(f) = r.birth_mass_fraction {
            check_range(id, "reproduction.birth_mass_fraction", f, 1e-6, 0.5)?;
        }
        check_range(id, "reproduction.min_energy_fraction", r.min_energy_fraction, 0.0, 1.0)?;
        let s = &self.social;
        let (gl, gh) = s.group_size;
        if gl == 0 || gl > gh {
            return Err(EcosystemError::species(
                id,
                "social.group_size",
                format!("({gl}, {gh}) must satisfy 1 ≤ min ≤ max"),
            ));
        }
        check_range(id, "social.cohesion", s.cohesion, 0.0, 1.0)?;
        check_range(id, "social.territory_radius_m", s.territory_radius_m, 0.0, 100_000.0)?;
        if s.territorial && s.territory_radius_m <= 0.0 {
            return Err(EcosystemError::species(
                id,
                "social.territory_radius_m",
                "must be positive for territorial species",
            ));
        }
        Ok(())
    }

    fn validate_behaviour(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        let b = &self.behaviour;
        let w = &b.weights;
        for (name, v) in [
            ("forage", w.forage),
            ("hunt", w.hunt),
            ("scavenge", w.scavenge),
            ("flee", w.flee),
            ("drink", w.drink),
            ("rest", w.rest),
            ("sleep", w.sleep),
            ("mate", w.mate),
            ("wander", w.wander),
            ("defend", w.defend),
            ("migrate", w.migrate),
        ] {
            check_range(id, &format!("behaviour.weights.{name}"), v, 0.0, 100.0)?;
        }
        for (name, curve) in [
            ("hunger_curve", &b.hunger_curve),
            ("thirst_curve", &b.thirst_curve),
            ("fear_curve", &b.fear_curve),
            ("fatigue_curve", &b.fatigue_curve),
        ] {
            curve.validate().map_err(|e| EcosystemError::species(id, &format!("behaviour.{name}"), e.to_string()))?;
        }
        check_range(id, "behaviour.boldness", b.boldness, 0.0, 1.0)?;
        check_range(id, "behaviour.vigilance", b.vigilance, 0.0, 1.0)?;
        if let Some(alignment) = self.social.alignment {
            check_range(id, "social.alignment", alignment, 0.0, 1.0)?;
        }
        let c = &self.combat;
        check_range(id, "combat.attack", c.attack, 0.0, 100.0)?;
        check_range(id, "combat.defence", c.defence, 0.0, 100.0)?;
        check_range(id, "combat.aggression", c.aggression, 0.0, 1.0)?;
        Ok(())
    }
}

impl Species {
    /// Seasonal behaviour and representation.
    fn validate_lifestyle(&self) -> Result<(), EcosystemError> {
        let id = self.id.as_str();
        let s = &self.seasonal;
        check_range(id, "seasonal.migration_range_m", s.migration_range_m, 1.0, 1e6)?;
        if let Some(h) = &s.hibernation {
            check_range(id, "seasonal.hibernation.below_c", h.below_c, -60.0, 40.0)?;
            check_range(id, "seasonal.hibernation.wake_above_c", h.wake_above_c, -60.0, 45.0)?;
            if h.wake_above_c <= h.below_c {
                return Err(EcosystemError::species(
                    id,
                    "seasonal.hibernation.wake_above_c",
                    "must be above `below_c`",
                ));
            }
            check_range(id, "seasonal.hibernation.metabolic_fraction", h.metabolic_fraction, 0.001, 1.0)?;
            check_range(id, "seasonal.hibernation.min_reserve_fraction", h.min_reserve_fraction, 0.0, 1.0)?;
        }
        match &self.representation {
            Representation::Individual => Ok(()),
            Representation::Field(p) => {
                check_range(id, "representation.max_density_m2", p.max_density_m2, 1e-6, 1e6)?;
                check_range(id, "representation.max_birth_per_day", p.max_birth_per_day, 0.0, 10.0)?;
                check_range(id, "representation.mortality_per_day", p.mortality_per_day, 0.0, 1.0)?;
                check_range(id, "representation.dispersal_per_day", p.dispersal_per_day, 0.0, 1.0)?;
                check_range(id, "representation.half_saturation_kg_m2", p.half_saturation_kg_m2, 1e-9, 10.0)?;
                check_range(id, "representation.flower_visits_per_hour", p.flower_visits_per_hour, 0.0, 1e4)?;
                check_range(id, "representation.individuals_per_marker", p.individuals_per_marker, 1e-3, 1e9)?;
                self.check_aggregate_diet(false)
            }
            Representation::Colony(p) => {
                check_range(id, "representation.workers_per_forager", p.workers_per_forager, 1.0, 1e6)?;
                check_range(id, "representation.founding_workers", p.founding_workers, 1.0, 1e7)?;
                check_range(id, "representation.max_workers", p.max_workers, p.founding_workers, 1e7)?;
                check_range(id, "representation.forager_fraction", p.forager_fraction, 0.0, 1.0)?;
                check_range(id, "representation.foraging_range_m", p.foraging_range_m, 0.1, 1e5)?;
                check_range(id, "representation.travel_fraction", p.travel_fraction, 0.001, 1.0)?;
                check_range(id, "representation.load_kg", p.load_kg, 1e-9, 1.0)?;
                check_range(id, "representation.worker_lifespan_days", p.worker_lifespan_days, 0.1, 10_000.0)?;
                check_range(id, "representation.development_days", p.development_days, 0.1, 1_000.0)?;
                check_range(id, "representation.max_eggs_per_day", p.max_eggs_per_day, 0.0, 1e5)?;
                check_range(id, "representation.flower_visits_per_trip", p.flower_visits_per_trip, 0.0, 1e4)?;
                check_range(id, "representation.swarm_workers", p.swarm_workers, p.founding_workers, 1e7)?;
                if let Some(t) = p.nest_temperature_c {
                    check_range(id, "representation.nest_temperature_c", t, -10.0, 45.0)?;
                }
                if p.max_foragers == 0 {
                    return Err(EcosystemError::species(id, "representation.max_foragers", "must be at least 1"));
                }
                self.check_aggregate_diet(true)
            }
        }
    }

    /// Density fields eat plants, nectar, litter, mushrooms and other
    /// fields; colony foragers also collect carrion.
    fn check_aggregate_diet(&self, colony: bool) -> Result<(), EcosystemError> {
        for (i, entry) in self.diet.iter().enumerate() {
            let ok = match entry.food {
                Food::Plant(_) | Food::Nectar | Food::Detritus | Food::Fungi | Food::Prey(_) => true,
                Food::Carrion => colony,
                Food::Browse | Food::Eggs => false,
            };
            if !ok {
                let what = if colony { "colony" } else { "density-field" };
                return Err(EcosystemError::species(
                    &self.id,
                    &format!("diet[{i}]"),
                    format!("{:?} cannot be eaten by a {what} species", entry.food),
                ));
            }
        }
        Ok(())
    }
}

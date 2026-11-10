//! Factories: scenario data + seed → a runnable simulation, using each model
//! crate's own constructors.

use cellular_automata::{BuiltSimulation, LifeSim};
use continuous::ContinuousSim;
use ecosystem::EcosystemSim;
use hashlife::{HashlifeSim, Universe};
use metapixel::MetapixelSim;
use simulation_core::params::ParamKind;
use simulation_core::{DynSimulation, erase};

use crate::assets::Assets;
use crate::error::RegistryError;
use crate::scenario::{Scenario, ScenarioData};

/// Builds the simulation of `scenario` with `seed`.
///
/// # Errors
/// [`RegistryError::Build`] with the model crate's message,
/// [`RegistryError::Asset`] when required data is missing and
/// [`RegistryError::InvalidSeed`] for seeds a model cannot represent.
pub(crate) fn construct(
    assets: &Assets,
    scenario: &Scenario,
    seed: u64,
) -> Result<Box<dyn DynSimulation>, RegistryError> {
    let sim = construct_unchecked(assets, scenario, seed)?;
    check_seed(sim.as_ref(), seed)?;
    Ok(sim)
}

fn construct_unchecked(
    assets: &Assets,
    scenario: &Scenario,
    seed: u64,
) -> Result<Box<dyn DynSimulation>, RegistryError> {
    let target = scenario.info.id.as_str();
    let fail = |e: &dyn std::fmt::Display| RegistryError::build(target, e);
    Ok(match &scenario.data {
        ScenarioData::Ca(s) => s.build_with_seed(assets.patterns()?, seed).map_err(|e| fail(&e))?.into_dyn(),
        ScenarioData::Hashlife(_) => erase(hashlife_sim(scenario, seed)?),
        ScenarioData::Metapixel(_) => erase(metapixel_sim(assets, scenario, seed)?),
        ScenarioData::Continuous(_) => erase(continuous_sim(scenario, seed)?),
        ScenarioData::Voxel(s) => {
            let mut s = (**s).clone();
            s.seed = seed;
            erase(voxel_life::VoxelLife::new(s).map_err(|e| fail(&e))?)
        }
        ScenarioData::Environment(preset) => {
            let config = physics_world::WorldConfig::default().with_preset(*preset).with_seed(seed);
            erase(physics_world::EnvironmentSim::new(config).map_err(|e| fail(&e))?)
        }
        ScenarioData::Ecosystem(_) => erase(ecosystem_sim(assets, scenario, seed)?),
        ScenarioData::CustomLife(s) => {
            let mut s = (**s).clone();
            s.seed = seed;
            erase(rule_engine::CustomLifeSim::from_scenario(&s).map_err(|e| fail(&e))?)
        }
        ScenarioData::Evolution(s) => {
            let mut s = (**s).clone();
            s.seed = seed;
            erase(evolution::EvolutionSim::from_scenario(&s).map_err(|e| fail(&e))?)
        }
        ScenarioData::Civilization(s) => {
            let mut s = (**s).clone();
            s.seed = seed;
            erase(civilization::CivilizationSim::new(s).map_err(|e| fail(&e))?)
        }
    })
}

/// Models that publish their seed as an integer `seed` parameter (most
/// agent, field and voxel models) only represent seeds in that parameter's
/// range: a larger seed would be reported (and restored by a reset) as
/// another value, so it is rejected.
fn check_seed(sim: &dyn DynSimulation, seed: u64) -> Result<(), RegistryError> {
    let descriptor = sim.descriptor();
    let Some(&ParamKind::Int { min, max }) = descriptor.param(crate::params::SEED_PARAM).map(|p| &p.kind) else {
        return Ok(());
    };
    if i64::try_from(seed).is_ok_and(|s| (min..=max).contains(&s)) {
        return Ok(());
    }
    Err(RegistryError::InvalidSeed {
        model: descriptor.id,
        seed,
        message: format!("its `seed` parameter accepts {min} to {max}"),
    })
}

/// A Life-like scenario as a typed [`LifeSim`] (`None` for the other
/// cellular-automata models).
pub(crate) fn life_sim(assets: &Assets, scenario: &Scenario, seed: u64) -> Result<Option<LifeSim>, RegistryError> {
    match &scenario.data {
        ScenarioData::Ca(s) => {
            match s.build_with_seed(assets.patterns()?, seed).map_err(|e| RegistryError::build(&scenario.info.id, e))? {
                BuiltSimulation::Life(sim) => Ok(Some(sim)),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// A pattern of the `patterns` library loaded into a Hashlife universe.
pub(crate) fn hashlife_sim(scenario: &Scenario, seed: u64) -> Result<HashlifeSim, RegistryError> {
    let ScenarioData::Hashlife(entry) = &scenario.data else {
        return Err(RegistryError::build(&scenario.info.id, "not a Hashlife pattern scenario"));
    };
    let fail = |e: &dyn std::fmt::Display| RegistryError::build(&scenario.info.id, e);
    let pattern = entry.pattern().map_err(|e| fail(&e))?;
    let rule = hashlife::Rule::parse(pattern.rule.as_deref().unwrap_or("B3/S23")).map_err(|e| fail(&e))?;
    let mut universe = Universe::with_rule(rule);
    universe.insert_cells(pattern.live_cells());
    Ok(HashlifeSim::with_universe(universe, seed))
}

/// A metapixel scenario as a typed [`MetapixelSim`].
pub(crate) fn metapixel_sim(assets: &Assets, scenario: &Scenario, seed: u64) -> Result<MetapixelSim, RegistryError> {
    let ScenarioData::Metapixel(s) = &scenario.data else {
        return Err(RegistryError::build(&scenario.info.id, "not a metapixel scenario"));
    };
    let mut s = (**s).clone();
    s.seed = seed;
    MetapixelSim::from_scenario(assets.tiles()?, &s).map_err(|e| RegistryError::build(&scenario.info.id, e))
}

/// A continuous scenario as a typed [`ContinuousSim`].
pub(crate) fn continuous_sim(scenario: &Scenario, seed: u64) -> Result<ContinuousSim, RegistryError> {
    let ScenarioData::Continuous(s) = &scenario.data else {
        return Err(RegistryError::build(&scenario.info.id, "not a continuous scenario"));
    };
    ContinuousSim::with_seed((**s).clone(), seed).map_err(|e| RegistryError::build(&scenario.info.id, e))
}

/// An ecosystem scenario as a typed [`EcosystemSim`].
pub(crate) fn ecosystem_sim(assets: &Assets, scenario: &Scenario, seed: u64) -> Result<EcosystemSim, RegistryError> {
    let ScenarioData::Ecosystem(s) = &scenario.data else {
        return Err(RegistryError::build(&scenario.info.id, "not an ecosystem scenario"));
    };
    let mut s = (**s).clone();
    s.seed = seed;
    EcosystemSim::from_scenario(&s, assets.species()?.clone()).map_err(|e| RegistryError::build(&scenario.info.id, e))
}

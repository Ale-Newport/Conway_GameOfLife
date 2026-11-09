//! Minimal tour of the organism engine: wolves decide with utility AI, claim
//! deer as intents, conflicts are resolved deterministically, and offspring
//! inherit mutated genomes recorded in a genealogy.
//!
//! Run with `cargo run -p organism-engine --example wolves_and_deer`.

use organism_engine::intent::resolve_matching;
use organism_engine::perception::{self, Query};
use organism_engine::steering;
use organism_engine::{
    ActionScore, AgentId, AgentStore, Bounds, Claim, Consideration, Crossover, Curve, Drive, GeneSpec, Genealogy,
    Genome, GenomeSchema, Kinematics, Lineage, MotionLimits, OrganismError, choose_best, genetic_distance,
};
use simulation_core::glam::Vec2;
use simulation_core::{SimRng, SpatialGrid};

struct Wolf {
    kin: Kinematics,
    hunger: Drive,
    genome: Genome,
}

struct Deer {
    kin: Kinematics,
}

const SPEED: usize = 0;

fn main() -> Result<(), OrganismError> {
    let mut rng = SimRng::derive(42, 1);
    let schema = GenomeSchema::new(vec![
        GeneSpec::new("speed", 1.0, 3.0, 2.0).with_mutation_sd(0.08),
        GeneSpec::new("sight", 4.0, 16.0, 10.0),
    ])?;
    let bounds = Bounds::Clamp { min: Vec2::ZERO, max: Vec2::splat(50.0) };
    let hungry = Drive::new(0.01, 0.3)?.with_value(0.8);

    let mut deer: AgentStore<Deer> =
        (0..20).map(|i| Deer { kin: Kinematics::new(Vec2::new(2.5 * i as f32, 25.0), 0.0) }).collect();
    let mut wolves: AgentStore<Wolf> = (0..3)
        .map(|i| Wolf {
            kin: Kinematics::new(Vec2::new(8.0 + 15.0 * i as f32, 18.0), 0.0),
            hunger: hungry,
            genome: Genome::random(&schema, &mut rng),
        })
        .collect();
    let mut genealogy = Genealogy::new();
    for (id, _) in wolves.iter() {
        genealogy.record_birth(Lineage::founder(id, 0))?;
    }

    let hunger_curve = Curve::Logistic { k: 10.0, x0: 0.5 };
    let near_curve = Curve::InverseLinear;
    let mut grid = SpatialGrid::new(50.0, 50.0, 4.0);
    for tick in 1..=200u64 {
        perception::index_agents(&mut grid, &deer, |d| d.kin.position)?;

        // 1. Decide from read-only state (pure: could run in parallel).
        let mut claims = Vec::new();
        let mut accelerations = Vec::new();
        for (id, wolf) in wolves.iter() {
            let limits = MotionLimits::new(wolf.genome.express(&schema, SPEED), 1.5);
            let sight = wolf.genome.express_named(&schema, "sight").unwrap_or(10.0);
            let prey = perception::nearest(&grid, &Query::new(wolf.kin.position, sight), |_, _| true);
            let hunt = prey.map_or(0.0, |p| {
                let considerations = [
                    Consideration::new(wolf.hunger.value, &hunger_curve),
                    Consideration::new(p.distance / sight, &near_curve).weighted(0.5),
                ];
                ActionScore::evaluate(1.0, &considerations).value
            });
            let accel = match (choose_best(&[hunt, 0.1], None, 0.0), prey) {
                (Some(0), Some(p)) => {
                    let target = deer.ids()[p.slot as usize];
                    claims.push(Claim::new(id, target.get()).priority(wolf.hunger.value).distance(p.distance));
                    steering::seek(&wolf.kin, p.position, limits)
                }
                // Resting: brake to a stop.
                _ => steering::arrive(&wolf.kin, wolf.kin.position, 1.0, limits),
            };
            accelerations.push((accel, limits));
        }

        // 2. Resolve conflicts: at most one wolf per deer and one deer per wolf.
        let winners = resolve_matching(&claims);

        // 3. Apply in id order.
        let (mut pack, births) = wolves.split_mut();
        for ((_, wolf), (accel, limits)) in pack.iter_mut().zip(accelerations) {
            wolf.kin.integrate(accel, 0.2, limits, bounds);
            wolf.hunger.update(1.0);
        }
        for claim in winners.iter().filter(|c| c.distance < 1.0) {
            deer.queue_despawn(AgentId(claim.target));
            // A fed wolf breeds with the oldest other wolf (a deterministic choice).
            let Some(&mate_id) = pack.ids().iter().find(|&&w| w != claim.claimant) else { continue };
            let Some((parent, mate)) = pack.get_pair_mut(claim.claimant, mate_id) else { continue };
            parent.hunger.satisfy(0.6);
            let mut genome = Genome::crossover(&parent.genome, &mate.genome, &schema, Crossover::Uniform, &mut rng)?;
            genome.mutate(&schema, 0.5, &mut rng)?;
            let child_id = births.spawn(Wolf { kin: parent.kin, hunger: hungry, genome });
            let lineage = |id| genealogy.get(id).map(|n| n.lineage).ok_or(OrganismError::UnknownAgent(id));
            let (first, second) = (lineage(claim.claimant)?, lineage(mate_id)?);
            genealogy.record_birth(Lineage::offspring(child_id, &first, Some(&second), tick))?;
        }
        let eaten = deer.commit().despawned.len();
        wolves.commit();
        if eaten > 0 {
            println!("tick {tick}: {eaten} deer eaten, {} deer and {} wolves left", deer.len(), wolves.len());
        }
    }

    let founders: Vec<&Wolf> = wolves.iter().take(2).map(|(_, w)| w).collect();
    if let [a, b] = founders[..] {
        println!(
            "genetic distance between the first two wolves: {:.3}",
            genetic_distance(&a.genome, &b.genome, &schema)?
        );
    }
    let summary = genealogy.summary();
    println!(
        "{} births, {} living, {} generations, family edges: {:?}",
        summary.births,
        summary.living,
        summary.max_generation,
        genealogy.edges().iter().map(|e| (e.parent.get(), e.child.get())).collect::<Vec<_>>()
    );
    Ok(())
}

//! End-to-end check that the building blocks compose into a deterministic,
//! snapshot-replayable agent world: store + spatial index + perception +
//! steering + utility + intents + genomes + genealogy + drives. Every test runs
//! in a walled world (reflecting edges) and on a torus (wrap-aware perception,
//! steering towards the perceived images).

use organism_engine::intent::resolve_matching;
use organism_engine::perception::{self, Query};
use organism_engine::steering::{self, WanderParams, WanderState};
use organism_engine::{
    ActionScore, AgentId, AgentStore, Bounds, Claim, Consideration, Curve, Drive, GeneSpec, Genealogy, Genome,
    GenomeSchema, Kinematics, Lineage, MotionLimits, choose_best,
};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{SimRng, SpatialGrid};

const SIZE: f32 = 60.0;
const SPEED_GENE: usize = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Agent {
    kin: Kinematics,
    wander: WanderState,
    hunger: Drive,
    genome: Genome,
    lineage: Lineage,
    predator: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct World {
    tick: u64,
    rng: SimRng,
    bounds: Bounds,
    store: AgentStore<Agent>,
    genealogy: Genealogy,
    schema: GenomeSchema,
    eaten: u64,
    /// Percepts whose image differs from the stored position (seen across a wrapped edge).
    seen_across_edges: u64,
}

const WORLDS: [Bounds; 2] = [
    Bounds::Reflect { min: Vec2::ZERO, max: Vec2::splat(SIZE) },
    Bounds::Wrap { min: Vec2::ZERO, max: Vec2::splat(SIZE) },
];

/// The perceived agent, moved to the image the observer saw.
fn as_seen(agent: &Agent, image: Vec2) -> Kinematics {
    Kinematics { position: image, ..agent.kin }
}

impl World {
    fn new(seed: u64, bounds: Bounds) -> Self {
        let schema = GenomeSchema::new(vec![GeneSpec::new("speed", 1.0, 3.0, 2.0).with_mutation_sd(0.1)]).unwrap();
        let mut rng = SimRng::seed_from_u64(seed);
        let mut store = AgentStore::new();
        let mut genealogy = Genealogy::with_max_nodes(500);
        for i in 0..80 {
            let id = store.next_id();
            let lineage = Lineage::founder(id, 0);
            let position = Vec2::new(rng.range_f32(0.0, SIZE), rng.range_f32(0.0, SIZE));
            let agent = Agent {
                kin: Kinematics::new(position, rng.range_f32(-3.0, 3.0)),
                wander: WanderState::default(),
                hunger: Drive::new(0.004, 0.4).unwrap().with_value(rng.f32()),
                genome: Genome::random(&schema, &mut rng),
                lineage,
                predator: i % 8 == 0,
            };
            assert_eq!(store.queue_spawn(agent), id);
            genealogy.record_birth(lineage).unwrap();
        }
        store.commit();
        Self { tick: 0, rng, bounds, store, genealogy, schema, eaten: 0, seen_across_edges: 0 }
    }

    fn step(&mut self) {
        self.tick += 1;
        let mut grid = SpatialGrid::new(SIZE, SIZE, 6.0);
        perception::index_agents(&mut grid, &self.store, |a| a.kin.position).unwrap();
        let agents = self.store.as_slice();
        let ids = self.store.ids();
        let hunt_curve = Curve::Logistic { k: 12.0, x0: 0.4 };
        let close_curve = Curve::InverseLinear;

        // Decision phase: read-only, in id order; randomness from the world rng.
        let mut accelerations = Vec::with_capacity(agents.len());
        let mut wander_states = Vec::with_capacity(agents.len());
        let mut claims = Vec::new();
        let mut neighbours = Vec::new();
        let mut across = 0u64;
        let mut note = |slot: u32, image: Vec2| {
            if image.distance(agents[slot as usize].kin.position) > 1e-3 {
                across += 1;
            }
        };
        for (slot, agent) in agents.iter().enumerate() {
            let speed = agent.genome.express(&self.schema, SPEED_GENE);
            let limits = MotionLimits::new(speed, 2.0);
            let mut wander = agent.wander;
            let roam = steering::wander(&agent.kin, &mut wander, &WanderParams::default(), &mut self.rng, limits);
            wander_states.push(wander);
            let me = slot as u32;
            let accel = if agent.predator {
                let query =
                    Query::new(agent.kin.position, 15.0).with_fov(agent.kin.direction(), 4.0).in_world(self.bounds);
                let prey = perception::nearest(&grid, &query, |s, _| s != me && !agents[s as usize].predator);
                let scores = [
                    prey.map_or(0.0, |p| {
                        ActionScore::evaluate(
                            1.0,
                            &[
                                Consideration::new(agent.hunger.value, &hunt_curve),
                                Consideration::new(p.distance / 15.0, &close_curve),
                            ],
                        )
                        .value
                    }),
                    0.2,
                ];
                match (choose_best(&scores, None, 0.0), prey) {
                    (Some(0), Some(p)) => {
                        note(p.slot, p.position);
                        let target = as_seen(&agents[p.slot as usize], p.position);
                        if p.distance < 1.0 {
                            claims.push(
                                Claim::new(ids[slot], ids[p.slot as usize].get())
                                    .priority(agent.hunger.value)
                                    .distance(p.distance),
                            );
                        }
                        steering::pursue(&agent.kin, &target, 1.0, limits)
                    }
                    _ => roam,
                }
            } else {
                let query = Query::new(agent.kin.position, 6.0).in_world(self.bounds);
                perception::within(&grid, &query, |s, _| s != me && !agents[s as usize].predator, &mut neighbours);
                let flock: Vec<Kinematics> = neighbours
                    .iter()
                    .map(|p| {
                        note(p.slot, p.position);
                        as_seen(&agents[p.slot as usize], p.position)
                    })
                    .collect();
                let threat_query = Query::new(agent.kin.position, 8.0).in_world(self.bounds);
                let threat = perception::nearest(&grid, &threat_query, |s, _| agents[s as usize].predator);
                let evade = threat.map_or(Vec2::ZERO, |t| {
                    note(t.slot, t.position);
                    steering::evade(&agent.kin, &as_seen(&agents[t.slot as usize], t.position), 1.0, limits)
                });
                steering::combine_prioritized(
                    &[
                        (evade, 1.0),
                        (steering::separation(&agent.kin, &flock, 1.5, limits), 1.0),
                        (steering::alignment(&agent.kin, &flock, 6.0, limits), 0.5),
                        (steering::cohesion(&agent.kin, &flock, 6.0, limits), 0.3),
                        (roam, 0.3),
                    ],
                    limits.max_accel,
                )
            };
            accelerations.push(accel);
        }
        self.seen_across_edges += across;

        // Resolution phase: every prey is eaten by at most one predator and
        // every predator eats at most once.
        let winners = resolve_matching(&claims);
        let mut targets: Vec<u64> = winners.iter().map(|c| c.target).collect();
        targets.dedup();
        assert_eq!(targets.len(), winners.len());

        // Apply phase.
        let tick = self.tick;
        let (mut view, pending) = self.store.split_mut();
        for (((_, agent), accel), wander) in view.iter_mut().zip(accelerations).zip(wander_states) {
            let speed = agent.genome.express(&self.schema, SPEED_GENE);
            agent.kin.integrate(accel, 0.2, MotionLimits::new(speed, 2.0), self.bounds);
            agent.wander = wander;
            agent.hunger.update(1.0);
        }
        for claim in &winners {
            pending.despawn(AgentId(claim.target));
            let parent = view.get(claim.claimant).unwrap().clone();
            let mut genome = parent.genome.clone();
            genome.mutate(&self.schema, 0.5, &mut self.rng).unwrap();
            let child_id = pending.next_id();
            let lineage = Lineage::offspring(child_id, &parent.lineage, None, tick);
            let child = Agent { genome, lineage, hunger: Drive::new(0.004, 0.4).unwrap(), ..parent };
            assert_eq!(pending.spawn(child), child_id);
            self.genealogy.record_birth(lineage).unwrap();
            view.get_mut(claim.claimant).unwrap().hunger.satisfy(1.0);
        }
        let report = self.store.commit();
        assert!(report.ignored.is_empty());
        for (id, _) in &report.despawned {
            self.genealogy.record_death(*id, tick).unwrap();
        }
        self.eaten += report.despawned.len() as u64;
    }

    fn bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).unwrap()
    }
}

#[test]
fn identical_seeds_give_identical_worlds() {
    for bounds in WORLDS {
        let mut a = World::new(7, bounds);
        let mut b = World::new(7, bounds);
        let mut c = World::new(8, bounds);
        for _ in 0..300 {
            a.step();
            b.step();
            c.step();
        }
        assert_eq!(a.bytes(), b.bytes());
        assert_ne!(a.bytes(), c.bytes());
        assert!(a.eaten > 0, "{bounds:?}: predators caught prey");
        assert_eq!(a.genealogy.summary().living, a.store.len() as u64);
        assert!(a.store.iter().all(|(_, ag)| bounds.contains(ag.kin.position)));
        match bounds {
            Bounds::Wrap { .. } => assert!(a.seen_across_edges > 0, "the torus is perceived across its edges"),
            _ => assert_eq!(a.seen_across_edges, 0, "walled worlds have no images"),
        }
    }
}

#[test]
fn snapshot_restore_replays_exactly() {
    for bounds in WORLDS {
        let mut world = World::new(21, bounds);
        for _ in 0..150 {
            world.step();
        }
        let snapshot = world.bytes();
        for _ in 0..150 {
            world.step();
        }
        let mut restored: World = postcard::from_bytes(&snapshot).unwrap();
        for _ in 0..150 {
            restored.step();
        }
        assert_eq!(restored, world);
        assert_eq!(restored.bytes(), world.bytes());
    }
}

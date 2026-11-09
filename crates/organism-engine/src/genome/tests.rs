//! Tests of the genome module: schema validation, expression, mutation
//! statistics, crossover inheritance, distance and serde.

use super::*;

fn schema() -> GenomeSchema {
    GenomeSchema::new(vec![
        GeneSpec::new("speed", 0.5, 2.5, 1.0),
        GeneSpec::new("size", 1.0, 3.0, 2.0).with_mutation_sd(0.1),
        GeneSpec::new("skill", 0.0, 10.0, 0.0).non_heritable(),
        GeneSpec::new("colour", 0.0, 1.0, 0.5),
    ])
    .unwrap()
}

fn mean_sd(xs: &[f32]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let var = xs.iter().map(|&x| (f64::from(x) - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

#[test]
fn schema_validation_reports_problems() {
    let bad = |g: GeneSpec| GenomeSchema::new(vec![g]).unwrap_err();
    assert!(matches!(bad(GeneSpec::new("a", 2.0, 1.0, 1.5)), OrganismError::InvalidGene { .. }));
    assert!(bad(GeneSpec::new("a", 0.0, 1.0, 2.0)).to_string().contains("default 2 is outside [0, 1]"));
    assert!(matches!(bad(GeneSpec::new("a", f32::NAN, 1.0, 0.5)), OrganismError::InvalidGene { .. }));
    assert!(matches!(bad(GeneSpec::new(" ", 0.0, 1.0, 0.5)), OrganismError::InvalidGene { .. }));
    assert!(matches!(bad(GeneSpec::new("a", 0.0, 1.0, 0.5).with_mutation_sd(-0.1)), OrganismError::InvalidGene { .. }));
    let dup = GenomeSchema::new(vec![GeneSpec::new("a", 0.0, 1.0, 0.5), GeneSpec::new("a", 0.0, 2.0, 1.0)]);
    assert_eq!(dup.unwrap_err(), OrganismError::DuplicateGene("a".into()));
    assert!(GenomeSchema::new(vec![GeneSpec::new("constant", 3.0, 3.0, 3.0)]).is_ok());
}

#[test]
fn expression_maps_normalised_values_to_ranges() {
    let s = schema();
    let d = Genome::defaults(&s);
    assert_eq!(d.values(), &[0.25, 0.5, 0.0, 0.5]);
    assert_eq!(d.express(&s, 0), 1.0);
    assert_eq!(d.express_named(&s, "size"), Some(2.0));
    assert_eq!(d.express_named(&s, "wings"), None);
    let g = Genome::from_normalised(&s, vec![1.0, 0.0, 0.3, 0.25]).unwrap();
    assert_eq!((g.express(&s, 0), g.express(&s, 1)), (2.5, 1.0));
    assert!((g.express(&s, 2) - 3.0).abs() < 1e-5);
    assert!(Genome::from_normalised(&s, vec![0.5; 3]).is_err());
    assert!(Genome::from_normalised(&s, vec![0.5, 0.5, 0.5, 1.5]).is_err());
    let constant = GeneSpec::new("c", 3.0, 3.0, 3.0);
    assert_eq!((constant.express(0.7), constant.normalise(3.0)), (3.0, 0.0));
    let unvalidated = GeneSpec::new("broken", 2.0, 1.0, 1.5);
    assert!(unvalidated.express(0.5).is_finite(), "never panics on an unvalidated spec");
}

#[test]
fn random_genomes_are_reproducible_and_keep_non_heritable_defaults() {
    let s = schema();
    let a = Genome::random(&s, &mut SimRng::seed_from_u64(1));
    assert_eq!(a, Genome::random(&s, &mut SimRng::seed_from_u64(1)));
    assert_ne!(a, Genome::random(&s, &mut SimRng::seed_from_u64(2)));
    assert_eq!(a.values()[2], 0.0);
    assert!(a.values().iter().all(|v| (0.0..=1.0).contains(v)));
}

#[test]
fn mutation_preserves_the_mean_and_has_the_declared_spread() {
    let s = schema();
    let base = Genome::from_normalised(&s, vec![0.5, 0.5, 0.5, 0.5]).unwrap();
    let mut rng = SimRng::seed_from_u64(42);
    let mut speeds = Vec::new();
    let mut sizes = Vec::new();
    for _ in 0..20_000 {
        let mut g = base.clone();
        assert_eq!(g.mutate(&s, 1.0, &mut rng).unwrap(), 3, "every heritable gene mutates at rate 1");
        assert_eq!(g.values()[2], 0.5, "non-heritable genes never mutate");
        speeds.push(g.values()[0]);
        sizes.push(g.values()[1]);
    }
    let (mean, sd) = mean_sd(&speeds);
    assert!((mean - 0.5).abs() < 0.002 && (sd - 0.05).abs() < 0.002, "speed mean {mean} sd {sd}");
    let (mean, sd) = mean_sd(&sizes);
    assert!((mean - 0.5).abs() < 0.004 && (sd - 0.1).abs() < 0.003, "size mean {mean} sd {sd}");
}

#[test]
fn mutation_rate_is_a_per_gene_probability() {
    let s = schema();
    let mut rng = SimRng::seed_from_u64(5);
    let mut total = 0;
    for _ in 0..10_000 {
        let mut g = Genome::defaults(&s);
        total += g.mutate(&s, 0.3, &mut rng).unwrap();
    }
    let fraction = total as f64 / 30_000.0;
    assert!((fraction - 0.3).abs() < 0.01, "{fraction}");
    let mut g = Genome::defaults(&s);
    assert_eq!(g.mutate(&s, 0.0, &mut rng).unwrap(), 0);
    assert_eq!(g, Genome::defaults(&s));
    // A NaN rate counts as 0: no mutation and no random draws.
    let before = rng.clone();
    assert_eq!(g.mutate(&s, f32::NAN, &mut rng).unwrap(), 0);
    assert_eq!(rng, before);
}

#[test]
fn contract_free_functions_match_the_methods() {
    let s = schema();
    let a = Genome::random(&s, &mut SimRng::seed_from_u64(1));
    let b = Genome::random(&s, &mut SimRng::seed_from_u64(2));
    for operator in [Crossover::Uniform, Crossover::OnePoint] {
        let via_fn = crossover(&a, &b, &s, operator, &mut SimRng::seed_from_u64(3)).unwrap();
        let via_method = Genome::crossover(&a, &b, &s, operator, &mut SimRng::seed_from_u64(3)).unwrap();
        assert_eq!(via_fn, via_method);
    }
    let (mut x, mut y) = (a.clone(), a);
    let n = mutate(&mut x, &s, 0.7, MutationBounds::Reflect, &mut SimRng::seed_from_u64(4)).unwrap();
    let m = y.mutate_with(&s, 0.7, MutationBounds::Reflect, &mut SimRng::seed_from_u64(4)).unwrap();
    assert_eq!((n, &x), (m, &y));
}

#[test]
fn mutation_clamps_to_the_gene_range() {
    let s = GenomeSchema::new(vec![GeneSpec::new("x", 0.0, 1.0, 0.98).with_mutation_sd(0.5)]).unwrap();
    let mut rng = SimRng::seed_from_u64(8);
    let mut at_max = 0;
    for _ in 0..10_000 {
        let mut g = Genome::defaults(&s);
        g.mutate(&s, 1.0, &mut rng).unwrap();
        let v = g.values()[0];
        assert!((0.0..=1.0).contains(&v));
        assert!((0.0..=1.0).contains(&g.express(&s, 0)));
        if v == 1.0 {
            at_max += 1;
        }
    }
    // P(N(0, 0.5) > 0.02) ≈ 0.484.
    assert!((4500..5200).contains(&at_max), "{at_max}");
}

#[test]
fn reflecting_mutation_avoids_pile_up_at_the_bounds() {
    let s = GenomeSchema::new(vec![GeneSpec::new("x", 0.0, 1.0, 0.98).with_mutation_sd(0.5)]).unwrap();
    let mut rng = SimRng::seed_from_u64(8);
    let values: Vec<f32> = (0..10_000)
        .map(|_| {
            let mut g = Genome::defaults(&s);
            g.mutate_with(&s, 1.0, MutationBounds::Reflect, &mut rng).unwrap();
            g.values()[0]
        })
        .collect();
    assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
    let on_bounds = values.iter().filter(|&&v| v == 0.0 || v == 1.0).count();
    assert!(on_bounds < 5, "{on_bounds}");
    assert_eq!(MutationBounds::Reflect.apply(1.25), 0.75);
    assert_eq!(MutationBounds::Reflect.apply(-0.25), 0.25);
    assert_eq!(MutationBounds::Reflect.apply(2.5), 0.5);
    assert_eq!(MutationBounds::Reflect.apply(f32::NAN), 0.0);
    assert_eq!(MutationBounds::Clamp.apply(1.25), 1.0);
}

#[test]
fn uniform_crossover_mixes_parents_gene_by_gene() {
    let s = schema();
    let a = Genome::from_normalised(&s, vec![0.0, 0.0, 0.9, 0.0]).unwrap();
    let b = Genome::from_normalised(&s, vec![1.0, 1.0, 0.8, 1.0]).unwrap();
    let mut rng = SimRng::seed_from_u64(3);
    let mut from_a = [0u32; 4];
    for _ in 0..4000 {
        let child = Genome::crossover_uniform(&a, &b, &s, &mut rng).unwrap();
        assert_eq!(child.values()[2], 0.0, "non-heritable genes start at the default");
        for (i, &v) in child.values().iter().enumerate() {
            if i != 2 {
                assert!(v == 0.0 || v == 1.0, "every gene comes from one parent");
                from_a[i] += u32::from(v == 0.0);
            }
        }
    }
    for i in [0, 1, 3] {
        assert!((1850..2150).contains(&from_a[i]), "gene {i}: {}", from_a[i]);
    }
}

#[test]
fn one_point_crossover_takes_a_prefix_and_a_suffix() {
    let plain = GenomeSchema::new((0..4).map(|i| GeneSpec::new(&format!("g{i}"), 0.0, 1.0, 0.5)).collect()).unwrap();
    let a = Genome::from_normalised(&plain, vec![0.0; 4]).unwrap();
    let b = Genome::from_normalised(&plain, vec![1.0; 4]).unwrap();
    let mut rng = SimRng::seed_from_u64(4);
    let mut cuts = [0u32; 5];
    for _ in 0..3000 {
        let child = Genome::crossover(&a, &b, &plain, Crossover::OnePoint, &mut rng).unwrap();
        let cut = child.values().iter().filter(|&&v| v == 0.0).count();
        let expected: Vec<f32> = (0..4).map(|i| if i < cut { 0.0 } else { 1.0 }).collect();
        assert_eq!(child.values(), expected.as_slice(), "prefix from a, suffix from b");
        cuts[cut] += 1;
    }
    assert_eq!((cuts[0], cuts[4]), (0, 0), "both parents always contribute");
    assert!(cuts[1..4].iter().all(|&c| (900..1100).contains(&c)), "uniform cut: {cuts:?}");
    // Non-heritable genes start at their default.
    let s = schema();
    let skilled = Genome::from_normalised(&s, vec![0.0, 0.0, 1.0, 0.0]).unwrap();
    let child = Genome::crossover_one_point(&skilled, &skilled, &s, &mut rng).unwrap();
    assert_eq!(child.values()[2], 0.0);
    // A single gene needs no cut and no random draw.
    let single = GenomeSchema::new(vec![GeneSpec::new("x", 0.0, 1.0, 0.5)]).unwrap();
    let one_a = Genome::from_normalised(&single, vec![0.2]).unwrap();
    let one_b = Genome::from_normalised(&single, vec![0.9]).unwrap();
    let mut untouched = SimRng::seed_from_u64(1);
    let child = Genome::crossover_one_point(&one_a, &one_b, &single, &mut untouched).unwrap();
    assert_eq!(child, one_a);
    assert_eq!(untouched, SimRng::seed_from_u64(1));
}

#[test]
fn crossover_rejects_mismatched_genomes() {
    let s = schema();
    let short = Genome::try_from(vec![0.5; 2]).unwrap();
    let ok = Genome::defaults(&s);
    let mut rng = SimRng::seed_from_u64(0);
    let err = Genome::crossover_uniform(&ok, &short, &s, &mut rng).unwrap_err();
    assert_eq!(err, OrganismError::GenomeLength { expected: 4, found: 2 });
    assert!(genetic_distance(&short, &ok, &s).is_err());
    assert!(short.clone().mutate(&s, 0.5, &mut rng).is_err());
}

#[test]
fn genetic_distance_is_a_normalised_metric_over_heritable_genes() {
    let s = schema();
    let zero = Genome::from_normalised(&s, vec![0.0, 0.0, 0.0, 0.0]).unwrap();
    let one = Genome::from_normalised(&s, vec![1.0, 1.0, 0.0, 1.0]).unwrap();
    let skilled = Genome::from_normalised(&s, vec![0.0, 0.0, 1.0, 0.0]).unwrap();
    assert_eq!(genetic_distance(&zero, &zero, &s).unwrap(), 0.0);
    assert_eq!(genetic_distance(&zero, &one, &s).unwrap(), 1.0);
    assert_eq!(genetic_distance(&zero, &skilled, &s).unwrap(), 0.0, "acquired traits are ignored");
    let mid = Genome::from_normalised(&s, vec![0.5, 0.0, 0.0, 0.0]).unwrap();
    let d = genetic_distance(&zero, &mid, &s).unwrap();
    assert!((d - (0.25f32 / 3.0).sqrt()).abs() < 1e-6);
    assert_eq!(d, genetic_distance(&mid, &zero, &s).unwrap());
}

#[test]
fn set_normalised_validates() {
    let s = schema();
    let mut g = Genome::defaults(&s);
    g.set_normalised(2, 0.7).unwrap();
    assert!((g.express(&s, 2) - 7.0).abs() < 1e-5);
    assert_eq!(g.set_normalised(9, 0.5), Err(OrganismError::GeneIndex { index: 9, len: 4 }));
    assert!(g.set_normalised(0, f32::NAN).is_err());
}

#[test]
fn serde_round_trips_and_defaults() {
    let json = r#"[{"name":"speed","min":0.5,"max":2.5,"default":1.0},
                   {"name":"skill","min":0.0,"max":10.0,"default":0.0,"heritable":false,"mutation_sd":0.0}]"#;
    let s: GenomeSchema = serde_json::from_str(json).unwrap();
    assert_eq!(s.genes()[0].mutation_sd, DEFAULT_MUTATION_SD);
    assert!(s.genes()[0].heritable && !s.genes()[1].heritable);
    let back: GenomeSchema = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    assert!(serde_json::from_str::<GenomeSchema>(r#"[{"name":"x","min":1.0,"max":0.0,"default":0.5}]"#).is_err());
    let g = Genome::random(&s, &mut SimRng::seed_from_u64(1));
    let bytes = postcard::to_stdvec(&g).unwrap();
    assert_eq!(postcard::from_bytes::<Genome>(&bytes).unwrap(), g);
    assert!(serde_json::from_str::<Genome>("[0.5, 1.5]").is_err());
    assert_eq!(serde_json::to_string(&Crossover::OnePoint).unwrap(), r#""one_point""#);
}

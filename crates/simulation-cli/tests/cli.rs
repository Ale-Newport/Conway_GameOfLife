//! Drives the `emergence-cli` binary: listing, info, runs with CSV and JSON
//! export, snapshots and resume, comparisons, validation, benchmarks,
//! pattern conversion and exit codes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_emergence-cli");

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Runs the binary with the workspace assets and built-in scenarios only
/// (so that a user's own scenario folder cannot change the results).
fn cli(args: &[&str]) -> Output {
    cli_with_assets(&assets(), args)
}

fn cli_with_assets(assets: &Path, args: &[&str]) -> Output {
    Command::new(BIN).arg("--assets").arg(assets).arg("--builtin-only").args(args).output().expect("the binary runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(out.status.success(), "{args:?} failed: {}\n{}", stderr(&out), stdout(&out));
    stdout(&out)
}

/// Fresh, empty folder under the system temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("emergence-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn list_models_and_scenarios() {
    let text = ok(&["list", "models"]);
    assert!(text.starts_with("ID "), "{text}");
    for id in ["life.classic", "life.metapixel", "continuous.flow_lenia", "ecosystem", "civilization"] {
        assert!(text.lines().any(|l| l.starts_with(id)), "{id} missing:\n{text}");
    }
    assert!(text.contains("Conway within Conway (OTCA metapixel)"), "names come from the descriptors");
    assert!(text.contains("17 models"));

    let text = ok(&["list", "scenarios", "--model", "ca.wolfram"]);
    assert_eq!(text.lines().filter(|l| l.starts_with("ca-lab/wolfram-rule-")).count(), 4, "{text}");
    assert!(text.contains("4 scenarios; scenario directories: none"), "{text}");

    let json: serde_json::Value = serde_json::from_str(&ok(&["list", "scenarios", "--json"])).unwrap();
    assert_eq!(json["scenarios"].as_array().unwrap().len(), 119);
    assert_eq!(json["scenarios"][0]["source"]["kind"], "builtin");

    let out = cli(&["list", "scenarios", "--model", "life.nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("unknown model `life.nope`"), "{}", stderr(&out));
}

#[test]
fn info_shows_parameters_policies_metrics_and_rules() {
    let text = ok(&["info", "life.classic"]);
    assert!(text.contains("Model      life.classic — Classic Conway"), "{text}");
    assert!(text.contains("PARAMETERS (9)") && text.contains("METRICS (8)") && text.contains("RULES"));
    let width = text.lines().find(|l| l.starts_with("width ")).unwrap();
    assert!(width.contains("int [1, 65536]") && width.contains("requires reset"), "{width}");
    let rule = text.lines().find(|l| l.starts_with("rule ")).unwrap();
    assert!(rule.contains("applies next tick"), "{rule}");

    let json: serde_json::Value = serde_json::from_str(&ok(&["info", "evolution/r_vs_k", "--json"])).unwrap();
    assert_eq!(json["scenario"]["id"], "evolution/r_vs_k");
    assert_eq!(json["descriptor"]["id"], "evolution");
    assert!(json["descriptor"]["params"].as_array().unwrap().len() > 3);
}

#[test]
fn run_exports_csv_and_json() {
    let dir = temp_dir("run");
    let csv = dir.join("rule30.csv");
    let text = ok(&["run", "ca-lab/wolfram-rule-30", "--ticks", "20", "--metrics-every", "5", "--out", s(&csv)]);
    assert!(text.contains("scenario  ca-lab/wolfram-rule-30"), "{text}");
    assert!(text.contains("ran       20 ticks in "), "{text}");
    assert!(text.contains("tick counter 0 -> 20, simulated time 0 -> 20 step"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("METRIC") && l.contains("FINAL")), "{text}");
    let written = std::fs::read_to_string(&csv).unwrap();
    assert!(written.starts_with("# run_id: ca-lab/wolfram-rule-30@1\n# model_id: ca.wolfram\n"), "{written}");
    let table: Vec<&str> = written.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(table[0].starts_with("tick,"), "{}", table[0]);
    let ticks: Vec<&str> = table[1..].iter().map(|l| l.split(',').next().unwrap()).collect();
    assert_eq!(ticks, ["0", "5", "10", "15", "20"]);

    let json = dir.join("ant.json");
    ok(&["run", "ca.langtons_ant", "--ticks", "6", "--seed", "4", "--out", s(&json), "--quiet"]);
    let record = analytics::run_from_json(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(record.model_id, "ca.langtons_ant");
    assert_eq!(record.seed, 4);
    assert_eq!(record.series.iter().next().unwrap().len(), 7, "initial state + 6 ticks");

    let out = cli(&["run", "life.classic", "--ticks", "1", "--out", s(&dir.join("x.txt"))]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("must end in .csv or .json"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tick_is_one_model_step_as_in_the_application() {
    let dir = temp_dir("steps");
    // The metapixel's default step is one metageneration (35 328 generations).
    let json = dir.join("metapixel.json");
    let text = ok(&["run", "life.metapixel", "--ticks", "2", "--out", s(&json)]);
    assert!(text.contains("tick counter 0 -> 70656"), "{text}");
    let record = analytics::run_from_json(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let readouts = record.series.get("readouts").unwrap();
    assert_eq!(readouts.ticks().collect::<Vec<_>>(), [0, 35_328, 70_656]);
    assert_eq!(readouts.last().unwrap().value, 2.0, "one macro readout per tick");
    // Hashlife's step exponent sets the generations of one tick.
    let text = ok(&["run", "hashlife/glider", "--ticks", "3", "--param", "step_exponent=4", "--quiet"]);
    let generation = text.lines().find(|l| l.starts_with("generation ")).unwrap();
    assert!(generation.split_whitespace().nth(1) == Some("48"), "{generation}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parameters_seed_snapshot_and_resume() {
    let dir = temp_dir("resume");
    let common = ["--param", "width=48", "--param", "height=40", "--param", "rule=B36/S23", "--seed", "9", "--quiet"];
    let straight = dir.join("straight.json");
    let mut args = vec!["run", "conway/random-soup-torus", "--ticks", "30", "--out", s(&straight)];
    args.extend(common);
    ok(&args);

    let snapshot = dir.join("half.elsnap");
    let mut args = vec!["run", "conway/random-soup-torus", "--ticks", "12", "--snapshot-out", s(&snapshot)];
    args.extend(common);
    ok(&args);
    let resumed = dir.join("resumed.json");
    let text = ok(&[
        "run",
        "conway/random-soup-torus",
        "--ticks",
        "18",
        "--resume",
        s(&snapshot),
        "--out",
        s(&resumed),
        "--quiet",
    ]);
    assert!(text.contains("population"));

    let a = analytics::run_from_json(&std::fs::read_to_string(&straight).unwrap()).unwrap();
    let b = analytics::run_from_json(&std::fs::read_to_string(&resumed).unwrap()).unwrap();
    assert_eq!(a.params.get("width"), Some(&simulation_core::ParamValue::Int(48)));
    let last = |r: &analytics::RunRecord, key: &str| r.series.get(key).unwrap().last().unwrap();
    assert_eq!(last(&b, "generation").value, 30.0);
    for key in ["population", "births", "deaths", "density"] {
        assert_eq!(last(&a, key), last(&b, key), "{key} after resuming at generation 12");
    }

    // A reset-only parameter cannot be applied to a resumed state.
    let out = cli(&["run", "conway/random-soup-torus", "--resume", s(&snapshot), "--param", "width=64"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("requires a reset"), "{}", stderr(&out));
    // A snapshot of another model is refused.
    let out = cli(&["run", "ca.wolfram", "--resume", s(&snapshot)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cannot resume"), "{}", stderr(&out));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compare_runs_replicates_and_writes_summaries() {
    let dir = temp_dir("compare");
    let json = dir.join("summary.json");
    let text = ok(&[
        "compare",
        "conway/random-soup-torus",
        "--seeds",
        "1..3",
        "--param",
        "width=32,48",
        "--param",
        "height=32",
        "--ticks",
        "8",
        "--metrics-every",
        "4",
        "--out",
        s(&json),
        "--metric",
        "population",
        "--quiet",
    ]);
    assert!(text.contains("seed=1 width=32 height=32") && text.contains("seed=2 width=48 height=32"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("population") && l.contains(" 4 ")), "{text}");
    let table = analytics::comparison_from_json(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(table.runs.len(), 4);
    assert!(table.runs.iter().all(|r| r.ticks == 8));
    let population = table.metric("population").unwrap();
    assert_eq!(population.final_across_runs.unwrap().count, 4);

    let csv = dir.join("summary.csv");
    ok(&["compare", "ca.wolfram", "--seeds", "1,2", "--ticks", "4", "--out", s(&csv), "--quiet"]);
    let written = std::fs::read_to_string(&csv).unwrap();
    assert!(written.starts_with("source,key,run_id,model_id,scenario,seed,samples,first_tick,final_tick,final,mean,"));

    let out = cli(&["compare", "ca.wolfram", "--ticks", "4"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("give --seeds"), "{}", stderr(&out));
    let out =
        cli(&["compare", "life.classic", "--seeds", "1", "--param", "rule=B3/S23,B9/S1", "--ticks", "2", "--quiet"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("error: run `seed=1 rule=B9/S1` failed")
            && stderr(&out).contains("parameter `rule`: digit 9 is not a valid neighbour count"),
        "{}",
        stderr(&out)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_reports_checks_and_exit_status() {
    let text = ok(&["validate", "ca.wolfram", "life.hashlife", "--ticks", "6"]);
    assert!(text.contains("PASS  determinism") && text.contains("PASS  snapshot round trip"), "{text}");
    assert!(text.contains("PASS  pattern periods"), "{text}");
    assert!(text.contains("2 target(s) validated: 2 passed, 0 failed"), "{text}");

    let out = cli(&["validate", "ca.wolfram", "no-such-model"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stdout(&out).contains("FAIL  build"), "{}", stdout(&out));

    let json: serde_json::Value =
        serde_json::from_str(&ok(&["validate", "ca.wolfram", "--ticks", "4", "--json"])).unwrap();
    assert_eq!(json[0]["report"]["checks"][0]["status"], "passed");
}

#[test]
fn bench_measures_and_writes_json() {
    let dir = temp_dir("bench");
    let json = dir.join("bench.json");
    let text = ok(&["bench", "ca.wolfram", "--ticks", "20", "--warmup", "2", "--json", s(&json)]);
    assert!(text.contains("ticks per second") && text.contains("time per tick, p95"), "{text}");
    assert!(text.contains("logical cores"), "{text}");
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["measured_ticks"], 20);
    assert_eq!(report["ticks_range"], serde_json::json!([2, 22]), "warm-up ticks are not measured");
    assert_eq!(report["sim_time_range"], serde_json::json!([2.0, 22.0]));
    assert_eq!(report["tick_unit"], "step");
    assert!(report["ticks_per_second"].as_f64().unwrap() > 0.0);
    assert!(report["machine"]["logical_cores"].as_u64().unwrap() >= 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_round_trips_through_every_format() {
    let dir = temp_dir("convert");
    let source = dir.join("gun.rle");
    std::fs::copy(assets().join("patterns/life/guns/gosper-glider-gun.rle"), &source).unwrap();
    let original = patterns::read_pattern_file(&source).unwrap().normalized();
    for (name, format) in
        [("a.cells", None), ("b.lif", None), ("c.lif", Some("life105")), ("d.mc", None), ("e.txt", Some("rle"))]
    {
        let out = dir.join(name);
        let mut args = vec!["convert", s(&source), s(&out)];
        if let Some(f) = format {
            args.extend(["--format", f]);
        }
        let text = ok(&args);
        assert!(text.contains("36 live cells, bounding box 36×9"), "{text}");
        let back = dir.join(format!("{name}.rle"));
        ok(&["convert", s(&out), s(&back)]);
        let cells: Vec<(i64, i64)> = patterns::read_pattern_file(&back).unwrap().normalized().live_cells().collect();
        assert_eq!(cells, original.live_cells().collect::<Vec<_>>(), "{name}");
    }
    let mc = std::fs::read_to_string(dir.join("d.mc")).unwrap();
    assert!(mc.starts_with("[M2]") && mc.contains("#R B3/S23"), "{mc}");

    let out = cli(&["convert", s(&source), s(&dir.join("x.unknown"))]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cannot tell the output format"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_standard_output_does_not_stop_the_run() {
    let dir = temp_dir("pipe");
    let csv = dir.join("wolfram.csv");
    // Standard output is a pipe whose reader is already gone (as with `| head`).
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = Command::new(BIN)
        .args(["--builtin-only", "run", "ca.wolfram", "--ticks", "5", "--quiet", "--out", s(&csv)])
        .stdout(writer)
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    assert!(std::fs::read_to_string(&csv).unwrap().lines().last().unwrap().starts_with("5,"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_have_context_and_exit_codes() {
    let out = cli(&["run", "no-such-scenario"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("`no-such-scenario` is not a model or scenario id"), "{}", stderr(&out));
    let out = cli(&["run", "island_evolution"]);
    assert!(stderr(&out).contains("ambiguous") && stderr(&out).contains("evolution/island_evolution"));
    let out = cli(&["run", "life.classic", "--param", "width=wide"]);
    assert_eq!(
        stderr(&out).trim(),
        "error: cannot build `life.classic`: parameter `width`: expected an integer, got `wide`"
    );
    let out = cli(&["run", "ecosystem", "--seed", "18446744073709551615", "--ticks", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("its `seed` parameter accepts 0 to 9223372036854775807"), "{}", stderr(&out));
    let out = cli(&["run", "life.classic", "--ticks", "many"]);
    assert_eq!(out.status.code(), Some(2), "usage errors exit with 2");
    let missing = std::env::temp_dir().join("emergence-cli-no-such-folder");
    let out = Command::new(BIN).args(["list", "models"]).env("EMERGENCE_LAB_ASSETS", &missing).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("EMERGENCE_LAB_ASSETS names"), "{}", stderr(&out));
    // Without the OTCA tiles the metapixel model explains what is missing.
    let empty = temp_dir("empty-assets");
    let out = cli_with_assets(&empty, &["run", "life.metapixel", "--ticks", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("OTCA tiles"), "{}", stderr(&out));
    let _ = std::fs::remove_dir_all(&empty);
}

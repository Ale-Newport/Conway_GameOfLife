//! `live_workers` counts every runner thread of the process, so it is tested
//! alone in its own test binary.

use std::time::{Duration, Instant};

use simulation_core::{
    Metrics, ModelDescriptor, ModelFamily, ParamSet, Runner, RunnerConfig, SimError, Simulation, Snapshot, live_workers,
};

/// A model that does nothing.
struct Idle;

impl Simulation for Idle {
    type Command = ();
    type ViewRequest = ();
    type View = ();

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new("test.idle", "Idle", ModelFamily::Cellular)
    }
    fn tick(&self) -> u64 {
        0
    }
    fn step(&mut self) {}
    fn reset(&mut self) {}
    fn apply(&mut self, _command: ()) -> Result<(), SimError> {
        Ok(())
    }
    fn params(&self) -> ParamSet {
        ParamSet::new()
    }
    fn metrics(&self) -> Metrics {
        Metrics::new(0, 0.0)
    }
    fn view(&self, _request: &()) {}
    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode("test.idle", 1, 0, &())
    }
    fn restore(&mut self, _snapshot: &Snapshot) -> Result<(), SimError> {
        Ok(())
    }
}

#[test]
fn worker_threads_are_counted_until_they_exit() {
    assert_eq!(live_workers(), 0);
    let joined = Runner::spawn(Idle, RunnerConfig::default());
    let dropped = Runner::spawn(Idle, RunnerConfig::default());
    assert_eq!(live_workers(), 2);
    joined.shutdown_and_wait();
    assert_eq!(live_workers(), 1);
    // Dropping does not wait: the thread exits on its own shortly after.
    drop(dropped);
    let start = Instant::now();
    while live_workers() > 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "the dropped runner's thread never exited");
        std::thread::sleep(Duration::from_millis(2));
    }
}

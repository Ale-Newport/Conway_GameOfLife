//! Integration tests of the Hashlife engine (one test binary).
//!
//! * `agreement` — Hashlife against an independent naive stepper on seeded
//!   random soups, for several rules, step sizes and arbitrary `n`.
//! * `patterns` — known behaviour: glider, blinker, Gosper gun, methuselahs,
//!   2^40-generation jumps, queries.
//! * `fuzz` — randomised operation sequences under random rules against the
//!   naive stepper.
//! * `memory` — garbage collection and rule changes never change results.
//! * `io` — macrocell round trips.
//! * `simulation` — the `Simulation` wrapper.
//! * `golly` — `#[ignore]`d validation against Golly's own `.mc` files.

mod agreement;
mod common;
mod fuzz;
mod golly;
mod io;
mod memory;
mod patterns;
mod simulation;

//! Sandboxed Rhai script rules.
//!
//! Each [`crate::def::ScriptRule`] is compiled once into an `AST` when the
//! system is compiled. Every call runs `fn run(me)` with
//! `CallFnOptions::eval_ast(false)` (top-level statements never run) and a
//! per-call tag carrying the frozen frame, so API functions can read the
//! world and record intents without global state. Any runtime error —
//! budget exhausted, recursion too deep, data too large, unknown function,
//! bad argument, cancellation — disables that script for the rest of the
//! run and is reported as a [`ScriptError`] (script name, line:column,
//! message).

pub(crate) mod api;
pub mod sandbox;

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use rhai::{AST, CallFnOptions, Dynamic, Engine, EvalAltResult, Position, Scope};
use serde::{Deserialize, Serialize};

use crate::compile::ir::REvent;
use crate::compile::rules::lower_event;
use crate::compile::symbols::Symbols;
use crate::def::LifeSystem;
use crate::error::{Diagnostic, ItemRef, Span, item_index};
use crate::sim::frame::{Frame, Target};
use crate::sim::intent::Act;
use api::{EntityRef, Recorder, ScriptCtx};

/// Maximum script source length in bytes.
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;

/// Category of a script failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptErrorKind {
    /// The operation budget was exhausted (e.g. an endless loop).
    Budget,
    /// Function calls nested too deeply (e.g. endless recursion).
    Recursion,
    /// A string, array, map or variable limit was exceeded.
    Memory,
    /// A function that does not exist was called.
    UnknownFunction,
    /// A value of the wrong type was used.
    Type,
    /// The host cancelled the script.
    Cancelled,
    /// Any other runtime error (including invalid API arguments).
    Runtime,
}

/// A script failure, as shown in the UI's error list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptError {
    /// Script rule name.
    pub script: String,
    /// Entity type it ran for.
    pub entity: String,
    /// Tick of the failure.
    pub tick: u64,
    /// 1-based line inside the script, when known.
    pub line: Option<u32>,
    /// 1-based column, when known.
    pub column: Option<u32>,
    /// Category.
    pub kind: ScriptErrorKind,
    /// Message (Rhai's description, plus a hint for budget errors).
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "script `{}` ({})", self.script, self.entity)?;
        if let (Some(l), Some(c)) = (self.line, self.column) {
            write!(f, " at {l}:{c}")?;
        }
        write!(f, ", tick {}: {}", self.tick, self.message)
    }
}

fn position_parts(pos: Position) -> (Option<u32>, Option<u32>) {
    (pos.line().map(|l| l as u32), pos.position().map(|c| c as u32))
}

impl ScriptError {
    fn from_eval(script: &str, entity: &str, tick: u64, err: &EvalAltResult, budget: u64) -> Self {
        let inner = err.unwrap_inner();
        let pos = if inner.position().is_none() { err.position() } else { inner.position() };
        let (line, column) = position_parts(pos);
        let mut message = inner.to_string();
        if !inner.position().is_none() {
            let suffix = format!(" ({})", inner.position());
            if let Some(stripped) = message.strip_suffix(&suffix) {
                message = stripped.to_string();
            }
        }
        let kind = match inner {
            EvalAltResult::ErrorTooManyOperations(..) => {
                message = format!(
                    "{message}: the script exceeded its budget of {budget} operations in one call (look for loops that never end)"
                );
                ScriptErrorKind::Budget
            }
            EvalAltResult::ErrorStackOverflow(..) => {
                message = format!("{message}: function calls nested too deeply (endless recursion?)");
                ScriptErrorKind::Recursion
            }
            EvalAltResult::ErrorDataTooLarge(..) | EvalAltResult::ErrorTooManyVariables(..) => ScriptErrorKind::Memory,
            EvalAltResult::ErrorFunctionNotFound(..) => ScriptErrorKind::UnknownFunction,
            EvalAltResult::ErrorMismatchDataType(..) | EvalAltResult::ErrorMismatchOutputType(..) => {
                ScriptErrorKind::Type
            }
            EvalAltResult::ErrorTerminated(..) => ScriptErrorKind::Cancelled,
            _ => ScriptErrorKind::Runtime,
        };
        Self { script: script.to_string(), entity: entity.to_string(), tick, line, column, kind, message }
    }
}

/// A compiled script rule.
pub struct CompiledScript {
    /// Name.
    pub name: String,
    /// Entity type index.
    pub entity: u16,
    /// Event.
    pub event: REvent,
    /// Priority among the type's scripts.
    pub priority: i32,
    /// Whether the definition enables it.
    pub enabled: bool,
    ast: AST,
}

/// The sandboxed engine and all compiled scripts of a system.
pub struct ScriptHost {
    engine: Engine,
    scripts: Vec<CompiledScript>,
    cancel: Arc<AtomicBool>,
    budget: u64,
}

impl fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptHost").field("scripts", &self.scripts.len()).finish_non_exhaustive()
    }
}

impl ScriptHost {
    /// Compiles every script of `sys` (definitions must be valid).
    pub(crate) fn compile(sys: &LifeSystem, sym: &Symbols) -> Result<Self, Vec<Diagnostic>> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut engine = sandbox::sandboxed_engine(&sys.script_limits, Arc::clone(&cancel));
        api::register_api(&mut engine);
        let mut diags = Vec::new();
        let mut scripts = Vec::new();
        for (i, s) in sys.scripts.iter().enumerate() {
            let first = diags.len();
            Self::compile_one(&engine, sym, s, &mut diags, &mut scripts);
            for d in &mut diags[first..] {
                d.item.get_or_insert(ItemRef::Script(item_index(i)));
            }
        }
        if diags.is_empty() {
            Ok(Self { engine, scripts, cancel, budget: sys.script_limits.max_operations })
        } else {
            Err(diags)
        }
    }

    /// Compiles one script rule, appending it or its diagnostics.
    fn compile_one(
        engine: &Engine,
        sym: &Symbols,
        s: &crate::def::ScriptRule,
        diags: &mut Vec<Diagnostic>,
        scripts: &mut Vec<CompiledScript>,
    ) {
        let context = format!("script `{}` ({})", s.name, s.entity);
        let Some(entity) = sym.type_idx(&s.entity) else {
            let mut d = Diagnostic::error("E003", format!("unknown entity type `{}`", s.entity)).context(&context);
            if let Some(h) = crate::error::did_you_mean(&s.entity, sym.types.iter().map(String::as_str)) {
                d = d.help(h);
            }
            diags.push(d);
            return;
        };
        let Some(event) = lower_event(sym, entity, &s.event, &context, diags) else { return };
        if s.source.len() > MAX_SCRIPT_BYTES {
            diags.push(
                Diagnostic::error("E030", format!("script is longer than {MAX_SCRIPT_BYTES} bytes")).context(&context),
            );
            return;
        }
        let ast = match engine.compile(&s.source) {
            Ok(ast) => ast,
            Err(e) => {
                let (line, col) = position_parts(e.position());
                let span = Span::new(line.unwrap_or(0), col.unwrap_or(0), 1);
                diags.push(Diagnostic::error("E030", e.err_type().to_string()).context(&context).at(span));
                return;
            }
        };
        if !ast.iter_functions().any(|f| f.name == "run" && f.params.len() == 1) {
            diags.push(
                Diagnostic::error("E030", "the script must define `fn run(me)`")
                    .context(&context)
                    .help("fn run(me) { if me.energy < 5.0 { wander(); } }"),
            );
            return;
        }
        scripts.push(CompiledScript {
            name: s.name.clone(),
            entity,
            event,
            priority: s.priority,
            enabled: s.enabled,
            ast,
        });
    }

    /// Compiled scripts in definition order.
    pub fn scripts(&self) -> &[CompiledScript] {
        &self.scripts
    }

    /// Flag that aborts running scripts when set (each aborted script is
    /// disabled and reported). The simulation clears it after each tick.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Runs script `index` for the entity in `me` and returns the recorded intents.
    pub(crate) fn run(
        &self,
        index: usize,
        frame: &Arc<Frame>,
        me: usize,
        other: Option<Target>,
        rng: simulation_core::SimRng,
    ) -> Result<Vec<Act>, ScriptError> {
        let script = &self.scripts[index];
        let rec = Arc::new(Mutex::new(Recorder { acts: Vec::new(), rng }));
        let ctx = ScriptCtx {
            frame: Arc::clone(frame),
            me: me as u32,
            other,
            death: script.event == REvent::Death,
            rec: Arc::clone(&rec),
        };
        let me_ref = EntityRef::new(
            Arc::clone(frame),
            Some(Target { slot: me as u32, pos: frame.agent(me).kin.position, dist: 0.0 }),
        );
        let mut scope = Scope::new();
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true).with_tag(ctx);
        let result = self.engine.call_fn_with_options::<Dynamic>(options, &mut scope, &script.ast, "run", (me_ref,));
        if let Err(e) = result {
            let entity = &frame.compiled.types()[usize::from(script.entity)].def.name;
            return Err(ScriptError::from_eval(&script.name, entity, frame.tick, &e, self.budget));
        }
        let acts = std::mem::take(&mut rec.lock().unwrap_or_else(PoisonError::into_inner).acts);
        Ok(acts)
    }
}

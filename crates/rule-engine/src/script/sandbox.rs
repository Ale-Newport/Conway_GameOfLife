//! Construction of the sandboxed Rhai engine.
//!
//! Recipe verified for Rhai 1.27.0 (`docs/research/rules-scripting.md` §1):
//!
//! * `Engine::new_raw()` — no file module resolver, no `print` to stdout.
//! * Hand-picked packages: arithmetic, logic, math, bit fields, strings,
//!   iterators, function pointers, arrays and maps. `LanguageCorePackage`
//!   (blocking `sleep`, `exit`, `parse_json`), `BasicTimePackage`
//!   (`timestamp`, non-deterministic) and `BasicBlobPackage` are left out.
//!   No package gives access to files, the network, processes or the
//!   environment.
//! * `DummyModuleResolver` and `disable_symbol` for `eval`, `import` and
//!   `export` (`eval` is built into the evaluator, so it must be disabled
//!   explicitly).
//! * Every limit set explicitly (debug and release defaults differ).
//! * Strict variables, no shadowing, failing map property access.
//! * `on_progress` checks a host cancellation flag on every operation.
//!   Only the operation budget decides outcomes (it is deterministic); a
//!   cancellation disables the script and is reported.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rhai::module_resolvers::DummyModuleResolver;
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicFnPackage, BasicIteratorPackage, BasicMapPackage, BasicMathPackage,
    BasicStringPackage, BitFieldPackage, LogicPackage, MoreStringPackage, Package,
};
use rhai::{Dynamic, Engine};

use crate::def::ScriptLimits;

/// Builds the sandboxed engine (without the simulation API).
pub fn sandboxed_engine(limits: &ScriptLimits, cancel: Arc<AtomicBool>) -> Engine {
    let mut engine = Engine::new_raw();
    let packages = [
        ArithmeticPackage::new().as_shared_module(),
        LogicPackage::new().as_shared_module(),
        BasicMathPackage::new().as_shared_module(),
        BitFieldPackage::new().as_shared_module(),
        BasicStringPackage::new().as_shared_module(),
        MoreStringPackage::new().as_shared_module(),
        BasicIteratorPackage::new().as_shared_module(),
        BasicFnPackage::new().as_shared_module(),
        BasicArrayPackage::new().as_shared_module(),
        BasicMapPackage::new().as_shared_module(),
    ];
    for module in packages {
        engine.register_global_module(module);
    }
    engine.set_module_resolver(DummyModuleResolver::new());
    engine.disable_symbol("eval");
    engine.disable_symbol("import");
    engine.disable_symbol("export");
    engine
        .set_max_operations(limits.max_operations)
        .set_max_call_levels(limits.max_call_levels)
        .set_max_expr_depths(limits.max_expr_depth, limits.max_expr_depth)
        .set_max_string_size(limits.max_string_size)
        .set_max_array_size(limits.max_array_size)
        .set_max_map_size(limits.max_map_size)
        .set_max_variables(limits.max_variables)
        .set_max_functions(limits.max_functions)
        .set_max_modules(0);
    engine.set_strict_variables(true).set_allow_shadowing(false).set_fail_on_invalid_map_property(true);
    engine.on_progress(move |_ops| {
        if cancel.load(Ordering::Relaxed) { Some(Dynamic::from("cancelled by the host")) } else { None }
    });
    engine
}

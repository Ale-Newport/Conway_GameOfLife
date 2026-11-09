# Rules and Scripting for the Custom Life Creator

Research notes for Emergence Lab, covering:

1. Sandboxed Rhai 1.27.0, checked against the crate source and a compiled test program.
2. A no-code rule DSL: a survey of existing designs, an EBNF grammar, the serialized IR, validation rules and the evaluation order.
3. Relationship-editor semantics and how relationships compile into rules.
4. Options for drawing graphs in egui 0.36.

Date: 2026-11-09. Toolchain: Rust 1.98.1, macOS on an Apple M2 Max.
Verified means one of two things: the claim was read in the crate source under
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`, or it was confirmed by the verification
programs in `scratchpad/rhai-check/` (paths are in section 5). Anything that was not checked is marked **UNVERIFIED**.

---

## 0. Summary and decisions

| # | Decision | Why |
|---|----------|-----|
| D1 | The rule DSL compiles to a **typed Rust IR that a native interpreter runs** in the pure-Rust sim crate. Rhai is an **opt-in escape hatch** for advanced users, not the main path. | Measured cost of one Rhai `call_fn`: about 0.85 µs in release and about 11 µs in debug. The equivalent native code takes about 0.7 ns. Rhai on every agent every tick does not scale to 10^5 agents. |
| D2 | Build the Rhai engine with `Engine::new_raw()` and register packages by hand. Do **not** use `Engine::new()`. | `Engine::new()` installs `FileModuleResolver`, so `import` reads `.rhai` files from disk (verified). It also registers `LanguageCorePackage`, which exposes `sleep()`, a blocking call that `on_progress` cannot interrupt. Both behaviours were verified. |
| D3 | Always call `disable_symbol("eval")` (and also `"import"`/`"export"`). | `eval` is part of the evaluator, not a package, so it still works in `new_raw()` (verified). |
| D4 | Dependency line: `rhai = { version = "=1.27.0", features = ["sync", "no_time", "only_i64"] }`. Never enable `unchecked`. | `sync` is required to hold the engine in a Bevy resource, because Bevy 0.20.1 defines `Resource: Component` and `Component: Send + Sync + 'static`. `no_time` removes the nondeterministic `timestamp()`. If any crate in the graph enables `unchecked`, the limits API stops existing and our code fails to compile, which works as a built-in guard (verified). |
| D5 | Use `set_max_operations` as the primary per-call budget. Add `on_progress` for host cancellation and an optional wall-clock watchdog, sampled every 1024 operations. | Both verified. Budget errors cannot be caught by a script's `try/catch` (verified). |
| D6 | Compile each script to an `AST` once. Call it with `call_fn_with_options(CallFnOptions::new().eval_ast(false).with_tag(ctx), …)` and read the per-call context through `NativeCallContext::tag()`. | Plain `call_fn` re-runs top-level statements on every call (verified). The tag is a cheap way to give native API functions the agent's world context. |
| D7 | Decisions are **two-phase**. The decision phase reads only a snapshot, can run in parallel, and draws from a per-(seed, tick, agent, rule) RNG stream. The resolution phase is sequential and canonically ordered. | Results do not depend on thread count or iteration order. |
| D8 | Each relationship compiles into generated rules that carry `origin: Relationship{..}` provenance and sit in fixed priority bands. | The visual editor can show where a rule came from, users can "detach" a rule to edit it, and conflicts can be validated. |
| D9 | Graph UI uses **egui_graphs 0.32.0** for the relationship graph and **egui-snarl 0.12.0** for an optional node-based rule view. A custom `Painter`-based force-directed layout is the fallback. | Both crates depend on egui 0.36, and they resolve together with bevy_egui 0.43.1 to a single egui v0.36.2 (verified). Both also compiled and ran headless. egui-graph-edit (egui 0.35) and egui_node_graph2 (egui 0.29) are incompatible. |

---

## 1. Rhai 1.27.0 sandboxing (verified)

Crate: `rhai 1.27.0`. It is the newest version on crates.io as of 2026-11-09 (checked with `cargo info rhai`). License `MIT OR Apache-2.0`, MSRV `rust-version = "1.66.0"`.
Source: `rhai-1.27.0/` in the cargo registry. Repository: <https://github.com/rhaiscript/rhai>. Book: <https://rhai.rs/book>.

### 1.1 Cargo features (from `Cargo.toml.orig`)

| Feature | Meaning (paraphrasing the source comments) | Emergence Lab |
|---|---|---|
| `default = ["std", "ahash/runtime-rng"]` | std, plus runtime-random hashing seeds | keep |
| `sync` | "Require that all data types implement `Send + Sync`". `Shared` becomes `Arc` and `Locked` becomes `RwLock`. Callback types such as `OnProgressCallback` and `OnPrintCallback` gain `+ Send + Sync`. | **ON** |
| `no_time` | removes timestamps (`BasicTimePackage`: `timestamp()`, `elapsed`) | **ON**, for determinism |
| `only_i64` | "Disable all integer types (e.g. `u8`) other than `i64`" | **ON**. The full test suite passes with it. |
| `only_i32` | `INT = i32` and every other integer type disabled | off |
| `f32_float` | `FLOAT = f32` instead of `f64` | **off**. Keep `f64` in scripts and convert to and from glam `f32` at the boundary. The feature applies to every crate in the build because Cargo unifies features. |
| `no_float`, `no_index`, `no_object`, `no_function`, `no_closure`, `no_module`, `no_custom_syntax` | remove language features | off. `no_module` is optional, since `import` is already disabled via `disable_symbol` + `DummyModuleResolver`. |
| `no_std` | "Turn on `no-std` compilation (nightly only)". It implies `no_time`. | off. We do not need it, and it needs nightly. |
| `unchecked` | "Disable all safety checks" | **NEVER**. `api/limits.rs` starts with `#![cfg(not(feature = "unchecked"))]`, so `set_max_*` and `on_progress` do not exist under it. Verified: a test crate with `unchecked` fails with `E0599: no method named set_max_operations` / `on_progress`. |
| `no_position` | positions are not tracked | off. Positions are needed for error carets. |
| `metadata` | function metadata and doc comments (implies `serde`). Required by `Engine::gen_fn_signatures(bool) -> Vec<String>`. | optional, for editor autocompletion |
| `internals` | exposes internals. Needed for `Engine::definitions()`, `on_missing_function`, and `on_invalid_array_index` | off, unless tooling needs it |
| `grain` | New in this line: the "Rhai Grain" bytecode compiler and VM (`rhai::grain::{Compiler, Vm}`) | off. Not evaluated; listed only so readers know it exists. |
| `no_ast` | removes the parser and AST (used together with `grain`) | off |

The recommended dependency line was built and all checks passed: `rhai = { version = "=1.27.0", features = ["sync", "no_time", "only_i64"] }`.

### 1.2 `Engine::new()` vs `Engine::new_raw()` (from `src/engine.rs`)

```rust
pub fn new() -> Self                 // engine.rs:306
pub const fn new_raw() -> Self       // engine.rs:351, returns a copy of Engine::RAW
pub const RAW: Self                  // empty raw engine
```

`Engine::new()` = `new_raw()` plus the following:
1. `module_resolver = Some(Box::new(FileModuleResolver::new()))`, unless `no_module`/`no_std`/`no_ast`/wasm-unknown is set. **This allows filesystem reads through `import "path" as m;`**. Verified: the default engine imported a `secret.rhai` file from a temp directory and returned its exported constant (check 4a).
2. `set_max_strings_interned(256)` turns on the string interner.
3. Default `print` writes to stdout with `println!`, and default `debug` writes `"{source} @ {pos:?} | {s}"`.
4. `register_global_module(StandardPackage::new().as_shared_module())`.

`new_raw()` has no packages, `module_resolver: None` (so `module_resolver()` falls back to a static `DummyModuleResolver`), no interner, no print/debug callbacks, and the default `Limits`. Basic operators on built-in types still work in a raw engine through the fast-path built-ins in `func/builtin.rs`. **`eval`, `Fn`, `call`, `curry`, `type_of`, `is_def_var` and `is_def_fn` are handled by the evaluator itself.** For `eval`, see `func/call.rs:614`. They are therefore available even in `new_raw()`. Verified: `Engine::new_raw().eval::<INT>("eval(\"40 + 2\")") == Ok(42)` (check 4e).

### 1.3 Packages (`src/packages/`)

`StandardPackage` = `CorePackage` + `BitFieldPackage` + `LogicPackage` + `BasicMathPackage` + `BasicArrayPackage` + `BasicBlobPackage` + `BasicMapPackage` + `BasicTimePackage` (unless `no_time`) + `MoreStringPackage`.
`CorePackage` = `LanguageCorePackage` + `ArithmeticPackage` + `BasicStringPackage` + `BasicIteratorPackage` + `BasicFnPackage` (+ `DebuggingPackage` with `debugging`).

Functions that matter for sandboxing:

| Package | Problem functions | Problem |
|---|---|---|
| `LanguageCorePackage` (`lang_core.rs`) | `sleep(FLOAT)`, `sleep(INT)` | Calls `std::thread::sleep` and **blocks the thread**. It is a single native call, so neither `max_operations` nor `on_progress` can interrupt it. A script calling `sleep(1e9)` would freeze a worker. |
| | `exit()`, `exit(value)` | Ends the script with `EvalAltResult::Exit`. Harmless, but confusing in rules. |
| | `parse_json`, `take`, `tag` / `set_tag`, `get_fn_metadata_list` (reflection) | Not needed |
| `BasicTimePackage` | `timestamp()`, `elapsed` | Read `Instant::now()`, which is nondeterministic |
| all packages | no RNG and no file, net, process or env access. A grep of `src/packages` for `rand`, `std::fs`, `std::process`, `std::env` and `std::net` found nothing; the only `std::fs` read in the crate is in `module/resolvers/file.rs`. | fine |

**Sandbox package set (verified):** register `ArithmeticPackage`, `LogicPackage`, `BasicMathPackage`, `BitFieldPackage`, `BasicStringPackage`, `MoreStringPackage`, `BasicIteratorPackage` (needed for `range`, `chars`, `bits`), `BasicFnPackage`, `BasicArrayPackage` and `BasicMapPackage` individually through `register_global_module(Pkg::new().as_shared_module())`. Leave out `LanguageCorePackage`, `BasicTimePackage` and `BasicBlobPackage`. In that engine, `sleep(1)`, `exit()`, `timestamp()` and `parse_json(..)` all fail with `ErrorFunctionNotFound` (checks 4f).

Other things verified about removing functions:
- `disable_symbol("sleep")` has **no effect** on functions (check 4h). In `tokenizer.rs:1802-1824` it only applies to reserved keywords and operator or literal tokens, and `sleep` is a plain identifier.
- If `Engine::new()` must be used, a function registered on the engine with `register_fn` shadows the package version. The global namespace is `global_modules[0]` and packages are inserted at index 1. Registering `sleep(INT)` and `sleep(f64)` versions that return `Err(..)` worked (check 4i). Prefer the hand-picked package set.

### 1.4 Limits API (`src/api/limits.rs`, all `#[cfg(not(feature = "unchecked"))]`)

Exact signatures:

```rust
pub fn set_max_operations(&mut self, operations: u64) -> &mut Self          // 0 = unlimited
pub fn set_max_call_levels(&mut self, levels: usize) -> &mut Self           // not under no_function; 0 = no fn calls
pub fn set_max_expr_depths(&mut self, max_expr_depth: usize,
                           #[cfg(not(feature = "no_function"))] max_function_expr_depth: usize) -> &mut Self // 0 = unlimited
pub fn set_max_string_size(&mut self, max_len: usize) -> &mut Self          // bytes; 0 = unlimited
pub fn set_max_array_size(&mut self, max_size: usize) -> &mut Self          // not under no_index; 0 = unlimited (also BLOBs)
pub fn set_max_map_size(&mut self, max_size: usize) -> &mut Self            // not under no_object; 0 = unlimited
pub fn set_max_variables(&mut self, variables: usize) -> &mut Self          // EXISTS in 1.27; 0 = no variables
pub fn set_max_functions(&mut self, functions: usize) -> &mut Self          // not under no_function; 0 = no fn definitions
pub fn set_max_modules(&mut self, modules: usize) -> &mut Self              // not under no_module; 0 = no modules
// getters: max_operations() -> u64, max_call_levels(), max_expr_depth(), max_function_expr_depth(),
//          max_string_size(), max_array_size(), max_map_size(), max_variables(), max_functions(), max_modules() -> usize
pub fn set_max_strings_interned(&mut self, max: usize) -> &mut Self         // api/mod.rs (interner, not a limit)
```

**Zero does not mean the same thing everywhere.** For operations, expression depth, string, array and map, 0 means unlimited. For call levels, variables, functions and modules, 0 disables the feature. This comes from the `Limits` struct docs.

Defaults depend on whether `debug_assertions` is on. Values below were printed by the test program:

| Limit | Debug build | Release build |
|---|---|---|
| `max_call_levels` | **8** | 64 |
| `max_expr_depth` / `max_function_expr_depth` | 32 / 16 | 64 / 32 |
| operations, string, array, map | 0 (unlimited) | 0 (unlimited) |
| variables, functions, modules | `usize::MAX` | `usize::MAX` |

Always set the limits explicitly. Otherwise debug and release builds behave differently, for example deep but legitimate recursion fails only in debug.

Error variant each limit produces (verified):

| Limit | Error | Detected |
|---|---|---|
| operations | `EvalAltResult::ErrorTooManyOperations(pos)` → "Too many operations" | runtime |
| call levels | `ErrorStackOverflow(pos)`, wrapped in `ErrorInFunctionCall(..)` frames; `unwrap_inner()` gets to it | runtime |
| expression depth | `ParseErrorType::ExprTooDeep` → "Expression exceeds maximum complexity" | **compile** |
| string / array / map size | `ErrorDataTooLarge("Length of string" / "Size of array/BLOB" / "Size of object map", pos)` | runtime. A map literal over the limit is a **parse** error. |
| variables | `ErrorTooManyVariables(pos)` | runtime |
| functions | parse error "Number of functions defined exceeds maximum limit" | compile |
| modules | `ErrorTooManyModules(pos)` | runtime |

Data sizes are recursive totals (`eval/data_check.rs::calc_data_sizes`). The string limit applies to the **sum** of string bytes in a container. Verified: an array of 100 strings of 10 bytes each hits a 256-byte limit.

**Known gap (verified in 1.27.0).** Map index-assignment `m[key] = v` checks the size of the **item**, not of the map it belongs to (`eval/chaining.rs` around line 355: `self.check_data_size(item_ptr.as_ref(), …)`). New keys added that way therefore bypass `max_map_size` until the map is next passed to a function or property-assigned. `m.key = v` is checked. A script returned a 1000-key map under `set_max_map_size(256)` (check 3c), while `m.len()` inside the loop tripped the limit (check 3c'). Mitigations:
- `max_operations` bounds total growth anyway (check 3c'').
- The host must size-check values returned from scripts.
- Prefer returning typed host objects (`Action`) over maps.

### 1.5 Progress callback, cancellation and timeouts

```rust
pub fn on_progress(&mut self,
    callback: impl Fn(u64) -> Option<Dynamic> + SendSync + 'static) -> &mut Self   // api/events.rs:244
```

- The callback is called from `Engine::track_operation` (`eval/data_check.rs:184`) on **every operation**. Its argument is the operation count of the **current run**, because each `eval*` / `run*` / `call_fn*` creates a new `GlobalRuntimeState`. The `max_operations` check runs first.
- Returning `Some(token)` ends the run with `ErrorTerminated(token, pos)` (checks 1b and 1c).
- Budget and system errors cannot be caught by scripts. `EvalAltResult::is_catchable()` returns `false` for `ErrorTooManyOperations`, `ErrorTooManyVariables`, `ErrorTooManyModules`, `ErrorStackOverflow`, `ErrorDataTooLarge` and `ErrorTerminated`. Verified: `try { loop {} } catch { 1 }` still returns "Too many operations".
- Verified pattern: an `Arc<AtomicBool>` cancel flag checked on every op, plus an `Arc<AtomicU64>` deadline checked when `ops & 0x3ff == 0`. A 50 ms deadline stopped `while true {}` after 50.0 ms.
- **Determinism rule:** in the simulation, only `max_operations` decides outcomes, because it is deterministic. Wall-clock deadlines and UI cancellation are safety nets. If either fires, the script is **disabled** and reported, and the tick is retried without it or the simulation is paused. It must never quietly change sim state in a timing-dependent way.
- Measured: 100,000 operations take about 3.3 ms in release (roughly 33 ns per op) and about 21-34 ms in debug.

Other callbacks:

```rust
pub fn on_print(&mut self, callback: impl Fn(&str) + SendSync + 'static) -> &mut Self
pub fn on_debug(&mut self, callback: impl Fn(&str, Option<&str>, Position) + SendSync + 'static) -> &mut Self
// volatile (marked #[deprecated] = "considered volatile"): on_var, on_def_var; internals-only: on_missing_function
```

Route `print` and `debug` to the in-app script console. With `new_raw()`, a call to `print` finds the function through `BasicStringPackage` and does nothing if no callback is set (`func_call.rs:660`).

### 1.6 Disabling `eval`, `import` and other symbols

```rust
pub fn disable_symbol(&mut self, symbol: impl Into<Identifier>) -> &mut Self   // api/mod.rs:149
pub fn is_symbol_disabled(&self, symbol: &str) -> bool
```

- `disable_symbol("eval")` turns `eval("…")` into a parse error: "reserved keyword 'eval' is disabled" (check 4d).
- `disable_symbol("import")` gives "'import' is a reserved keyword" (check 4b). Also disable `"export"`.
- It works only for keywords and operators. `eval` is in the reserved-keyword table (`types/token.rs`), so it is affected, but ordinary function names are not.

Options worth setting (`api/options.rs`):

```rust
set_strict_variables(true)            // undefined variable => compile-time error (verified)
set_allow_shadowing(false)
set_fail_on_invalid_map_property(true)
set_allow_looping(false)              // "expression profile": `loop`/`while`/`for` => parse error (verified)
set_allow_anonymous_fn(false)         // closures => parse error (verified)
set_max_functions(0)                  // no `fn` definitions (verified)
```

There is an **expression-only profile** for user-written conditions. Use `compile_expression` together with the options above. It cannot loop or recurse, so it always terminates, and an operations budget is still a cheap safeguard.

### 1.7 Module resolvers

```rust
pub fn set_module_resolver(&mut self, resolver: impl ModuleResolver + 'static) -> &mut Self   // api/mod.rs:104
rhai::module_resolvers::{DummyModuleResolver, StaticModuleResolver, FileModuleResolver, ModuleResolversCollection}
```

`DummyModuleResolver::resolve` always returns `ErrorModuleNotFound(path, pos)` (verified, check 4c). To let users share helper modules, use a `StaticModuleResolver` that the host fills from the project's own validated scripts, never from disk paths given by scripts. `register_static_module("world", module)` exposes host APIs as `world::fn(..)`.

### 1.8 Registering the custom API

```rust
pub fn register_fn<A: 'static, const N: usize, const X: bool, R: Variant + Clone, const F: bool>(
    &mut self, name: impl AsRef<str> + Into<Identifier>,
    func: impl RhaiNativeFunc<A, N, X, R, F> + SendSync + 'static) -> &mut Self
pub fn register_type_with_name<T: Variant + Clone>(&mut self, name: &str) -> &mut Self
pub fn register_get<T: Variant + Clone, const X: bool, R: Variant + Clone, const F: bool>(
    &mut self, name: impl AsRef<str>, get_fn: impl RhaiNativeFunc<(Mut<T>,), 1, X, R, F> + SendSync + 'static) -> &mut Self
pub fn register_set<…>(&mut self, name: impl AsRef<str>, set_fn: impl RhaiNativeFunc<(Mut<T>, R), 2, X, (), F> + SendSync + 'static) -> &mut Self
pub fn build_type<T: CustomType>(&mut self) -> &mut Self
pub trait CustomType: Variant + Clone { fn build(builder: TypeBuilder<Self>); }
// TypeBuilder: with_name, with_fn, with_get, with_set, with_get_set, with_indexer_get/_set/_get_set,
//              on_print, on_debug, is_iterable, with_comments, and_comments
pub fn register_global_module(&mut self, module: SharedModule) -> &mut Self
pub fn register_static_module(&mut self, name: impl AsRef<str>, module: SharedModule) -> &mut Self
```

- A native function can fail by returning `Result<T, Box<EvalAltResult>>`. A `String` or `&str` converts with `.into()` into `ErrorRuntime`, and the script position is filled in automatically. Verified: `reproduce(1.5)` produced "Runtime error: reproduce(probability=1.5): probability must be in [0, 1] (line 2, position 5)".
- A native function can take `NativeCallContext` as its first parameter. `ctx.tag() -> Option<&Dynamic>` returns the per-call tag set with `CallFnOptions::with_tag(..)`. This is how free functions such as `nearby("food", 5)` find the calling agent's context without global state (verified, check 5).
- A missing getter gives a clear runtime error: "Unknown property 'enrgy' - a getter is not registered for type 'Organism'".
- **Design:** scripts receive read-only views (`OrganismView`) and return **intents** (`Action`, a host-registered type). They never mutate the world directly, so script calls can run in parallel and in any order, which supports D7.

### 1.9 Compile once, call many: Scope and `call_fn`

```rust
pub fn compile(&self, script: impl AsRef<str>) -> ParseResult<AST>                 // ParseResult<T> = Result<T, ParseError>
pub fn compile_expression(&self, script: impl AsRef<str>) -> ParseResult<AST>
pub fn compile_with_scope(&self, scope: &Scope, script: impl AsRef<str>) -> ParseResult<AST>
pub fn eval_ast_with_scope<T: Variant + Clone>(&self, scope: &mut Scope, ast: &AST) -> RhaiResultOf<T>  // = Result<T, Box<EvalAltResult>>
pub fn run_ast_with_scope(&self, scope: &mut Scope, ast: &AST) -> RhaiResultOf<()>
pub fn call_fn<T: Variant + Clone>(&self, scope: &mut Scope, ast: &AST,
    fn_name: impl AsRef<str>, args: impl FuncArgs) -> RhaiResultOf<T>
pub fn call_fn_with_options<T: Variant + Clone>(&self, options: CallFnOptions, scope: &mut Scope, ast: &AST,
    fn_name: impl AsRef<str>, args: impl FuncArgs) -> RhaiResultOf<T>
// CallFnOptions (#[non_exhaustive]): new(), bind_this_ptr(&mut Dynamic), with_tag(impl Variant+Clone),
//   eval_ast(bool) [default true], rewind_scope(bool) [default true], in_all_namespaces(bool)
// AST: iter_functions() -> impl Iterator<Item = ScriptFnMetadata> { name, params, access, this_type, .. }
// Scope: new, push, push_constant, push_dynamic, set_value, set_or_push, get_value::<T>, get_value_ref,
//        get_value_mut, contains, rewind, clear, len, iter
```

- **`call_fn` evaluates the whole AST before calling the function (`eval_ast` defaults to `true`).** Top-level statements therefore run on every call. Verified: 3 calls produced 3 top-level `print`s, while `eval_ast(false)` produced 0 (check 5c). For per-tick calls, always use `CallFnOptions::new().eval_ast(false)`.
- Arguments are passed as a tuple, for example `(org,)` with a trailing comma for one argument. Results are cast to `T` and return `ErrorMismatchOutputType` on mismatch.
- Use `ast.iter_functions()` at script-load time to check that the required entry points exist with the right arity, such as `decide(org)` and `on_birth(org)`.
- Hot reload: compile the new source. If it succeeds, swap in the new `Arc<AST>`. If it fails, keep the old AST and show the diagnostics.
- Keep one `Scope` per worker thread and reuse it. With the default `rewind_scope(true)`, it does not grow.

### 1.10 Threads and Bevy

- With `sync`, the test program confirmed at compile time that `Engine`, `AST`, `Scope<'static>`, `Dynamic`, `EvalAltResult` and `ParseError` are all `Send + Sync`. Eight threads shared one `&Engine` and one `&AST`, each with its own `Scope`, and got correct results (check 7).
- Without `sync`, compiling `assert_send_sync::<rhai::Engine>()` fails with "`Locked<StringsInterner>` cannot be shared between threads safely" and "`Rc<Locked<Dynamic>>` cannot be sent between threads safely". In bevy_ecs 0.20.1, `Resource: Component` (`resource.rs:87`) and `Component: Send + Sync + 'static` (`component/mod.rs:530`), so a non-`sync` engine could only be a `NonSend` resource.
- Under `sync`, `locked_read` and `locked_write` spin with `try_read` and `try_write`, sleeping 10 ms up to 5 times (`func/native.rs:636-720`). The engine's interner is only used while **parsing** (`parser.rs`). Do not compile on many threads at once with an interner-enabled engine. `new_raw()` has the interner turned off.
- Architecture: the Rhai engine lives in the **sim crate**, which has no Bevy dependency. The Bevy app holds the sim inside a `Resource`. Compilation happens on edit, through a UI action or an `AsyncComputeTaskPool` task.

### 1.11 Errors and rendering

`EvalAltResult` (`types/error.rs`) is `#[non_exhaustive]` and contains: `ErrorSystem`, `ErrorParsing(ParseErrorType, Position)`, `ErrorVariableExists`, `ErrorForbiddenVariable`, `ErrorVariableNotFound`, `ErrorPropertyNotFound`, `ErrorIndexNotFound`, `ErrorFunctionNotFound(String /*signature*/, Position)`, `ErrorModuleNotFound`, `ErrorInFunctionCall(String /*fn*/, String /*source*/, Box<Self>, Position)`, `ErrorInModule`, `ErrorUnboundThis`, `ErrorMismatchDataType(String, String, Position)`, `ErrorMismatchOutputType`, `ErrorIndexingType`, `ErrorArrayBounds`, `ErrorStringBounds`, `ErrorBitFieldBounds`, `ErrorFor`, `ErrorDataRace`, `ErrorNonPureMethodCallOnConstant`, `ErrorAssignmentToConstant`, `ErrorDotExpr`, `ErrorArithmetic`, `ErrorTooManyOperations`, `ErrorTooManyVariables`, `ErrorTooManyModules`, `ErrorStackOverflow`, `ErrorDataTooLarge(String, Position)`, `ErrorTerminated(Dynamic, Position)`, `ErrorCustomSyntax`, `ErrorRuntime(Dynamic, Position)`. It also has the pseudo-errors `LoopBreak`, `Return` and `Exit`.

Helpers: `position() -> Position`, `unwrap_inner(&self) -> &Self` (innermost error), `is_catchable()`, `is_system_exception()`, `is_pseudo_error()`, `clear_position()`, `take_position()`, `set_position()`.
Parse side: `ParseError` with `err_type() -> &ParseErrorType` and `position() -> Position`. `ParseErrorType` includes `UnexpectedEOF`, `BadInput(LexError)`, `MissingToken`, `VariableUndefined`, `ExprTooDeep`, `TooManyFunctions`, `Reserved`, `FnDuplicatedDefinition` and others. `LexError::ImproperSymbol` gives friendly hints such as "'===' is not a valid operator. This is not JavaScript! Should it be '=='?".
`Position` has `line() -> Option<usize>` and `position() -> Option<usize>` (1-based column; `None` for `Position::NONE`).
The `Display` output of both error types ends with " (line L, position C)". For UI messages, use `err_type()` for parse errors. For eval errors, use `take_position()` on the owned innermost error before formatting, or strip the suffix.

The rendering approach is verified to produce output like this (the test program's `render_eval_error`):

```
error[budget]: Too many operations (line 3, position 10)
  --> line 3, column 10
   |
  3|     x += 1;
   |          ^
error[parse]: Undefined variable: undefined_var          (strict_variables, at compile time)
  --> line 3, column 9
  3|     e + undefined_var
   |         ^
```

1. Walk `ErrorInFunctionCall` and `ErrorInModule` to build a "called from" trace.
2. Classify the innermost error into a user category (budget, recursion, memory, unknown-function, type, runtime, parse).
3. Show the caret with the source line.
4. In the editor, map the category to a help text, for example: "budget: your rule did more than 100,000 steps in one tick. Look for loops that never end."

For a polished text editor, `ariadne 0.6.0` (MIT), `codespan-reporting 0.13.1` (Apache-2.0) and `miette 7.6.0` (Apache-2.0) render multi-label diagnostics. Their versions and licenses come from `cargo info`; integration is **UNVERIFIED**.

### 1.12 Performance (measured, single thread, M2 Max)

| Case | Debug | Release |
|---|---|---|
| `call_fn_with_options(eval_ast(false).with_tag(ctx))` on `decide(org)`: 3 getters, 2 or 3 native calls, returns `Action` | 11.3 µs/call | **0.83-0.88 µs/call** (about 1.2 M calls/s) |
| Same logic as native Rust | - | 0.7 ns/call |
| 100k-operation budget exhausted | 21-34 ms | 3.3 ms |

Consequence: per-agent per-tick Rhai for 100k agents at 60 Hz needs about 6 M calls/s, which is the whole machine even at perfect 12-core scaling (UNVERIFIED). This supports D1: the DSL IR is native, and Rhai is used for low-frequency hooks (on_birth, on_death, per-species setup), small populations, or custom sensors that are cached per tick.

### 1.13 Recommended engine builder (condensed from the verified test program)

```rust
use rhai::{Engine, Dynamic, module_resolvers::DummyModuleResolver, packages::*};

pub fn sandboxed_engine(cancel: Arc<AtomicBool>, deadline_ns: Arc<AtomicU64>, epoch: Instant) -> Engine {
    let mut e = Engine::new_raw();
    for m in [ArithmeticPackage::new().as_shared_module(), LogicPackage::new().as_shared_module(),
              BasicMathPackage::new().as_shared_module(), BitFieldPackage::new().as_shared_module(),
              BasicStringPackage::new().as_shared_module(), MoreStringPackage::new().as_shared_module(),
              BasicIteratorPackage::new().as_shared_module(), BasicFnPackage::new().as_shared_module(),
              BasicArrayPackage::new().as_shared_module(), BasicMapPackage::new().as_shared_module()] {
        e.register_global_module(m);
    }                                                  // NOT LanguageCorePackage / BasicTimePackage
    e.set_module_resolver(DummyModuleResolver::new());
    e.disable_symbol("eval"); e.disable_symbol("import"); e.disable_symbol("export");
    e.set_max_operations(100_000).set_max_call_levels(32).set_max_expr_depths(64, 64)
     .set_max_string_size(1024).set_max_array_size(1024).set_max_map_size(256)
     .set_max_variables(128).set_max_functions(64).set_max_modules(0);
    e.set_strict_variables(true).set_allow_shadowing(false).set_fail_on_invalid_map_property(true);
    e.on_print(|s| log_script(s));                     // never stdout
    e.on_progress(move |ops| {
        if cancel.load(Relaxed) { return Some(Dynamic::from("cancelled by host")); }
        if ops & 0x3ff == 0 {
            let d = deadline_ns.load(Relaxed);
            if d != 0 && epoch.elapsed().as_nanos() as u64 > d { return Some("wall-clock budget exceeded".into()); }
        }
        None
    });
    e.build_type::<OrganismView>();                    // read-only getters
    e.register_type_with_name::<Action>("Action");     // intents
    e.register_fn("nearby", |ctx: NativeCallContext, kind: ImmutableString, r: INT| -> Result<INT, Box<EvalAltResult>> { /* ctx.tag() */ });
    e
}
```

Tune the operation budget per hook. Suggested starting points: `decide` 2,000; `on_birth`/`on_death` 10,000; per-species `setup` 100,000. RNG for scripts: register `rand() -> f64` and `chance(p) -> bool` so that they read a per-call ChaCha stream from the tag. Rhai's own packages contain no RNG. Rhai's `Map` is `BTreeMap<Identifier, Dynamic>` (`lib.rs:328`), so script-visible iteration order is deterministic. The `ahash` runtime seed (`func/hashing.rs`) affects only internal function-hash tables and `switch` hashing, not results (**UNVERIFIED** for every code path). It does mean ASTs are process-local.

---

## 2. A no-code rule DSL

### 2.1 Survey

| System | Rule / behavior model | Takeaways for us |
|---|---|---|
| **NetLogo**: <https://docs.netlogo.org/dictionary.html> | Imperative procedures over agentsets: `ask`, `with`, `in-radius`, `min-one-of`, `face`, `hatch`, `die`. The dictionary says "agentset members are always read in a random order". | Its vocabulary is the standard for agent-based models: sensing through agentset filters by radius, and "nearest" via `min-one-of … [distance myself]`. Random ask-order avoids first-mover bias but needs a seeded RNG for reproducibility. We get the same fairness through hashed tie-breaks (2.8). |
| **GAMA / GAML**: <https://gama-platform.org/wiki/DefiningActionsAndBehaviors> | Species have `reflex name when: <bool> { … }`, which runs each step if the condition is true, plus `init`, and `action`s with typed arguments | The closest existing model to `WHEN … THEN`. Reflexes are like our rules, `init` is like `on_birth`, and `action` is like our action vocabulary. |
| **StarLogo Nova / TNG** (MIT STEP): <https://wiki.slnova.org/wiki/On_Collision_With_Do>, <https://en.wikipedia.org/wiki/StarLogo> | Block programming. Each breed has its own program page, and an "on collision with <breed> do" hat block refers to the other agent as `collidee` | Event hats (`on_collision`) carry an implicit `other` reference. Rules are grouped per species. |
| **Scratch / Blockly**: <https://github.com/scratchfoundation/scratch-blocks> | Hat blocks for events ("when I start as a clone", "when I receive …"), C-blocks for control, reporter blocks with slot shapes that encode types. scratch-blocks 2.0 "depends on Blockly as a library" (Apache-2.0). | Encode types in slot shapes so that invalid programs cannot be built. Keep event hats separate from conditions. Typed reporter slots map to `Expr` types. |
| **Behavior trees**: Colledanchise and Ögren, "Behavior Trees in Robotics and AI", <https://arxiv.org/abs/1709.00084> | Control nodes Sequence, Fallback (Selector), Parallel and Decorator; leaves are Action and Condition; ticked each frame, returning Success, Failure or Running | Our priority-ordered rule list is equivalent to a single Fallback over Sequence(Condition, Action) children, which is easy to explain and visualize. Multi-tick "Running" actions such as move-to-target are better modelled as **intents with persistent targets** than as BT state. Rust crate if a real BT is ever needed: `bonsai-bt 0.14.0` (MIT; integration UNVERIFIED). |
| **Production rule engines**: CLIPS <https://www.csie.ntu.edu.tw/~sylee/courses/clips/bpg/node5.3.html>; Rete (Forgy 1982, *Artificial Intelligence* 19:17-37, doi:10.1016/0004-3702(82)90020-0) | Condition/action rules over working memory. A conflict-resolution agenda orders activations by salience, then by a strategy (depth, breadth, simplicity, complexity, LEX, MEA or random; the default is depth). Rete shares partial matches between rules. | **Salience = our `priority`.** We need an explicit, documented tie-break; ours is declaration order. We do not need Rete: each agent has few rules over local sensors, so a compiled linear scan with shared sensor caching per agent per tick is enough. |

### 2.2 Design principles

1. **One IR, two views.** The text DSL and the visual block editor both serialize to the same `RuleSet` IR, and it round-trips through RON and JSON (verified, 2.6). Text is a projection, so `format(parse(text))` is stable.
2. **Typed and total.** Every expression has a static type. Invalid trees are rejected at validation time. Runtime evaluation cannot fail: a missing target makes the action a no-op, never an error.
3. **Rules decide, the engine acts.** Rules emit intents. The resolver applies them in a canonical order (2.8).
4. **Bounded cost.** Sensors have a maximum radius, rules have no loops, and each agent's actions per tick are limited by slots.
5. **Deterministic.** There is no hidden randomness. `probability` uses keyed RNG streams.

### 2.3 Vocabulary

**Subjects:** `organism` (alias `self`), `other` (only in `on_collision` and `on_signal`), `cell` (the environment cell at the organism's position), `world` (globals such as `tick`, `season` and `temperature`).

**Properties:**
- Built-in read-only: `id`, `age`, `species`, `state`, `position`, `generation`.
- Built-in read-write: `energy`, `health`, `size`, `speed`.
- User-declared, per species: `property hunger: number = 0 in [0, 100]`.
- Types: `number` (f64 in the IR; can be stored as f32), `int`, `bool`, `kind`, `entity` (nullable), `state` (enum of declared states).

**Kinds**, for sensing and targets, are a closed set resolved at compile time into a bitset:
- species names: `"rabbit"`
- tags: `"food"`, `"predator"`, `"prey"`, `"threat"`, `"mate"`, `"plant"`, `"fungus"`, `"fire"`, `"water"`
- `same_species`
- `any`

Relationship-derived tags are computed by the relationship compiler (section 3). For example, `"food"` for species S means `{X | S consumes X}`.

**Comparators:** `< <= > >= == !=`, plus `is` (kind or state membership). Logic: `AND`, `OR`, `NOT`, parentheses. Arithmetic: `+ - * /`, with `min`, `max` and `clamp` functions.

**Sensors.** These are pure functions over the snapshot, cached per agent per tick:

| Sensor | Type | Notes |
|---|---|---|
| `nearby(kind, radius=r)` | int | count within r ≤ `MAX_SENSE_RADIUS` |
| `nearest(kind [, radius=r])` | entity? | `none` if nothing is found; ties go to the lowest hashed key (2.8) |
| `distance(entity)` | number | `inf` for `none` |
| `exists(entity)` | bool | |
| `signal(name [, radius])` | number | from the diffusion fields filled by `emit` |
| `random()` / `chance(p)` | number / bool | keyed RNG |
| `cell.<field>`, `world.<field>` | number | for example temperature, moisture, light |

**Events** (rule header; the default is `on_tick`): `on_tick`, `on_birth`, `on_death`, `on_collision [with kind]`, `on_signal(name)`, `every(n)`.

**Actions** and the slot each one occupies:

| Action | Slot | Arguments |
|---|---|---|
| `move_towards(target [, speed])`, `move_away(target [, speed])`, `wander([turn])`, `stay()` | MOVE | target is an entity or a position |
| `eat(target [, amount])`, `attack(target [, damage])`, `reproduce([probability, partner, offspring, energy_share])`, `transform_into(species_or_state)`, `die()` | ACT (exclusive) | |
| `emit(signal, amount [, radius])`, `spawn(kind [, count, radius])`, `set prop = expr`, `add prop += expr` / `-=` | EFFECT (any number) | `set` and `add` only on writable `organism.*` properties |

**Rule modifiers:**

| Modifier | Effect |
|---|---|
| `PRIORITY n` | higher fires first; int, default 0 |
| `PROBABILITY p` | rule-level chance |
| `COOLDOWN n` | ticks |
| `COST prop n` | resource cost, charged on success |
| `PRODUCES kind [xN]` | by-products, applied with the rule's ACT action |
| `ONCE` | fires at most once per lifetime |
| `GROUP name` | mutual exclusion beyond slots |

### 2.4 EBNF grammar

Text DSL v1. ISO-14977-style EBNF. Keywords are case-insensitive. `#` starts a comment that runs to the end of the line.

```ebnf
ruleset        = { species_block } ;
species_block  = "species" , ident , [ "extends" , ident ] , "{" , { decl } , "}" ;
decl           = property_decl | state_decl | rule ;
property_decl  = "property" , ident , ":" , type , [ "=" , literal ] , [ "in" , "[" , number , "," , number , "]" ] , [ "readonly" ] , ";" ;
state_decl     = "states" , ident , { "," , ident } , [ "initial" , ident ] , ";" ;
type           = "number" | "int" | "bool" | "kind" | "entity" | "state" ;

rule           = [ "rule" , ( ident | string ) , ":" ] ,
                 [ "ON" , event ] ,
                 [ "WHEN" , expr ] ,
                 "THEN" , action , { "," , action } ,
                 { modifier } , [ ";" ] ;          (* at least one of ON / WHEN; default event = on_tick *)

event          = "on_tick" | "on_birth" | "on_death"
               | "on_collision" , [ "with" , kind_lit ]
               | "on_signal" , "(" , string , ")"
               | "every" , "(" , int , ")" ;

modifier       = "PRIORITY" , int
               | "PROBABILITY" , number              (* 0 <= p <= 1 *)
               | "COOLDOWN" , int , [ "ticks" ]
               | "COST" , path , number
               | "PRODUCES" , kind_lit , [ "x" , int ]
               | "ONCE"
               | "GROUP" , ident ;

action         = assignment | call ;
assignment     = ( "set" , path , "=" , expr ) | ( "add" , path , ( "+=" | "-=" ) , expr ) ;

expr           = or_expr ;
or_expr        = and_expr , { "OR" , and_expr } ;
and_expr       = not_expr , { "AND" , not_expr } ;
not_expr       = "NOT" , not_expr | cmp_expr ;
cmp_expr       = sum , [ cmp_op , sum ] | sum , "is" , ( kind_lit | ident ) ;
cmp_op         = "<" | "<=" | ">" | ">=" | "==" | "!=" ;
sum            = term , { ( "+" | "-" ) , term } ;
term           = unary , { ( "*" | "/" ) , unary } ;
unary          = [ "-" ] , primary ;
primary        = number | string | "true" | "false" | "none"
               | call | path | "(" , expr , ")" ;
call           = ident , "(" , [ arg , { "," , arg } ] , ")" ;
arg            = [ ident , "=" ] , expr ;            (* positional args before named args *)
path           = ident , { "." , ident } ;           (* organism.energy, other.species, cell.temperature *)
kind_lit       = string | ident ;                    (* "food", same_species, rabbit *)

ident          = letter , { letter | digit | "_" } ;
number         = digit , { digit } , [ "." , digit , { digit } ] ;
int            = digit , { digit } ;
string         = '"' , { char - '"' } , '"' ;
```

Notes:
- `AND` binds tighter than `OR`, and `NOT` binds tightest of the three. Comparison does not chain (`a < b < c` is a parse error with a hint).
- A **sugar rule** handles action-level `probability=`: `reproduce(probability=0.15)` is the same as setting `PROBABILITY` on that action alone. The roll happens in the decision phase *before* the slot is claimed, so a failed roll leaves the ACT slot free for lower-priority rules.
- Both of the user's examples parse under this grammar as written. They have no `ON`, so the event defaults to `on_tick`.
- Implementation: a hand-written lexer plus a Pratt or recursive-descent parser that keeps a span on every node. `logos 0.16.1` (MIT OR Apache-2.0) is an option for the lexer and `chumsky 1.0.0-alpha.8` (MIT, still alpha) for parsing. Versions and licenses come from `cargo info`; the grammar has not been implemented yet, so its fit is **UNVERIFIED**.

Examples:

```text
species rabbit {
  property hunger: number = 0 in [0, 100];
  states kit, adult initial kit;

  rule forage:
    WHEN organism.energy < 20 AND nearby("food", radius=5) > 0
    THEN move_towards(nearest("food"))  PRIORITY 500

  rule graze:
    ON on_collision with "grass"
    THEN eat(other), add organism.energy += 15  PRIORITY 520 PRODUCES "dung"

  rule breed:
    WHEN organism.age > 10 AND organism.energy > 80 AND nearby("same_species", radius=2) >= 1
    THEN reproduce(probability=0.15)  PRIORITY 300 COOLDOWN 30 COST organism.energy 40

  rule flee:
    WHEN nearby("predator", radius=6) > 0
    THEN move_away(nearest("predator"), speed=organism.speed * 1.5), emit("alarm", 1.0, radius=4)  PRIORITY 900

  rule grow_up:  WHEN organism.state is kit AND organism.age >= 5  THEN transform_into(adult) ONCE
  rule starve:   WHEN organism.energy <= 0 THEN die()  PRIORITY 1000
  rule idle:     ON on_tick THEN wander()  PRIORITY 0
}
```

### 2.5 Serialized IR for the visual editor (RON and JSON)

serde enums use the default externally tagged form. `#[serde(tag=..)]`, `untagged` and `flatten` are avoided because ron 0.12.3's README says they have only "limited support" (`ron-0.12.3/README.md`, Limitations). The program `scratchpad/rhai-check/irshape` round-trips the IR below through `ron 0.12` and `serde_json 1` with `assert_eq!` (verified).

```rust
pub struct RuleSet { pub format: u32, pub species: String, pub rules: Vec<Rule> }
pub struct Rule {
    pub id: String,            // stable; used by the editor, cooldown slots and the RNG stream key
    pub name: String,
    pub event: Event,
    pub when: Option<Expr>,    // skip_serializing_if none
    pub then: Vec<ActionCall>,
    pub priority: i32,
    pub probability: Option<f32>,
    pub cooldown: Option<u32>,
    pub costs: Vec<Cost>,
    pub produces: Vec<Produce>,
    pub enabled: bool,         // default true
    pub origin: Option<Origin>,// Manual | Relationship { id, relation }
}
pub enum Event   { OnTick, OnBirth, OnDeath, OnCollision { with: Option<KindRef> }, OnSignal { name: String }, Every { ticks: u32 } }
pub enum KindRef { Species(String), Tag(String), SameSpecies, Any }
pub enum CmpOp   { Lt, Le, Gt, Ge, Eq, Ne }
pub enum Expr    { Num(f64), Bool(bool), Str(String), Prop { subject: Subject, name: String },
                   Cmp { op: CmpOp, lhs: Box<Expr>, rhs: Box<Expr> }, And(Vec<Expr>), Or(Vec<Expr>),
                   Not(Box<Expr>), Sensor { name: String, args: Vec<Arg> } }
pub enum Subject { Organism, Other, Cell, World }
pub struct Arg   { pub name: Option<String>, pub value: ArgValue }
pub enum ArgValue{ Expr(Expr), Kind(KindRef) }
pub struct ActionCall { pub action: String, pub args: Vec<Arg> }
```

Actual RON output for the user's second example (trimmed):

```ron
(
  id: "r-breed", name: "breed", event: OnTick,
  when: Some(And([
    Cmp(op: Gt, lhs: Prop(subject: Organism, name: "age"),    rhs: Num(10.0)),
    Cmp(op: Gt, lhs: Prop(subject: Organism, name: "energy"), rhs: Num(80.0)),
    Cmp(op: Ge, lhs: Sensor(name: "nearby", args: [(value: Kind(SameSpecies)), (name: Some("radius"), value: Expr(Num(2.0)))]), rhs: Num(1.0)),
  ])),
  then: [(action: "reproduce", args: [(name: Some("probability"), value: Expr(Num(0.15)))])],
  priority: 300, cooldown: Some(30), costs: [(property: "energy", amount: 40.0)], enabled: true,
)
```

The same data in JSON is externally tagged, for example `{"when":{"And":[{"Cmp":{"op":"Lt","lhs":{"Prop":{"subject":"Organism","name":"energy"}},"rhs":{"Num":20.0}}}, …]}}`.

Editor-only metadata, such as block positions and collapsed state, goes in a separate `EditorLayout { rules: BTreeMap<RuleId, BlockLayout> }`. Simulation files therefore do not change when a block is moved. Use `BTreeMap` rather than `HashMap` so that serialized output is stable.

After validation, this "authoring IR" is lowered to a **runtime IR**. In the runtime IR, property names become `PropId(u16)`, kinds become `KindMask(u128)` bitsets, sensors become an enum with resolved radii, and rules are pre-sorted. Lowering at runtime is allocation-free. Target cost per condition is tens of ns (**UNVERIFIED estimate**).

### 2.6 Validation and error messages

`Diagnostic { code, severity, message, primary: Anchor, labels: Vec<(Anchor, String)>, help: Option<String>, fix: Option<QuickFix> }`.
`Anchor` is `Span { line, col, len }` in text mode or `NodePath` in the visual editor, for example `rules["r-breed"].when.And[2].lhs.args[1]`. The editor uses the node path to outline the offending block and offer the quick-fix as a button.

| Code | Severity | Check | Example message |
|---|---|---|---|
| E001 | error | unknown property | `unknown property 'enrgy' on organism` · help: `did you mean 'energy'?` (Levenshtein ≤ 2) |
| E002 | error | type mismatch | `cannot compare number with kind: organism.energy < "food"` |
| E003 | error | unknown species, tag or state | `unknown kind "fod"` · help lists the tags defined by this species' relationships |
| E004 | error | bad arguments: unknown named argument, missing required, wrong type, positional after named | `move_towards() expects a target (entity or position), got number` |
| E005 | error | probability not in [0, 1] | `probability must be between 0 and 1 (got 1.5)` |
| E006 | error | radius ≤ 0 or > MAX_SENSE_RADIUS | `radius 50 exceeds the maximum sensing radius 16` |
| E007 | error | action not allowed for the event | `move_towards() is not allowed in on_death` |
| E008 | error | writing a read-only property | `'age' is read-only` |
| E009 | error | cost on a missing or non-numeric property | |
| E010 | error | two ACT-slot actions in one rule | `a rule can do only one of eat/attack/reproduce/transform_into/die per tick` |
| E011 | error | `transform_into` target is unknown | |
| E012 | error | cooldown or count is negative or not an integer | |
| E013 | error | `other` used outside `on_collision` / `on_signal` | |
| W101 | warning | rule can never fire: constant-false condition, or shadowed by a higher-priority unconditional rule that always takes the same slot | `rule 'idle' is shadowed by 'wander_always' (priority 10) for slot MOVE` |
| W102 | warning | duplicate rule | |
| W103 | warning | unconditional transform cycle A→B→A | |
| W104 | warning | `nearest(k)` used without a `nearby(k, r) > 0` guard (legal, it is a no-op) | |
| W105 | warning | `spawn` or `reproduce` without cost or cooldown (population explosion) | |
| W106 | info | high sensor cost: radius² × expected population over budget | |

Messages follow the Rust and Elm style:
- State what is wrong in user vocabulary, never IR names.
- Point to exactly one primary location.
- Add at most one `help:` line with a concrete fix.
- Never show a stack trace.
- Rhai errors are wrapped in the same `Diagnostic` structure (1.11).

### 2.7 Deterministic evaluation order (per tick t)

1. **Snapshot.** Freeze the agent state at the end of t−1. Build the spatial index with buckets in sorted order, keyed by `AgentId` (u64, never reused).
2. **Event queue.** Collect the events produced during t−1 resolution: births, collisions and signals. Sort them by `(agent_id, event_kind_order, other_id)`.
3. **Decision phase.** This phase can run in parallel with `rayon` `par_iter` over agents sorted by `AgentId`; `collect` on an indexed parallel iterator preserves order. For each agent:
   1. Evaluate its queued events first, then `on_tick`.
   2. Within each event, try the rules in `(priority desc, declaration index asc)` order. A rule is **eligible** if its event matches, its cooldown has expired, its costs are affordable (counting costs this agent has already reserved this tick), its condition is true, and its probability rolls succeed.
   3. An eligible rule claims its slots **atomically**. If any of its MOVE or ACT slots is already taken, the whole rule is skipped: no cost is charged and its cooldown does not start. EFFECT actions always accumulate.
   4. RNG: `ChaCha8Rng::seed_from_u64(mix(world_seed, tick, agent_id, rule_id_hash, action_idx))`. `mix` is a fixed splitmix64-style hash, not `std::hash::DefaultHasher`, whose algorithm is not guaranteed stable across Rust releases. As an alternative, rand_chacha 0.10's `set_stream(u64)` / `set_word_pos(u128)` give counter-based streams; both exist in `rand_chacha-0.10.0/src/chacha.rs`. Decisions therefore do not depend on thread count or evaluation order.
   5. Output: `Vec<Intent>` per agent, in rule order.
4. **Resolution phase.** Sequential. Fixed sub-phase order:
   1. Self EFFECTs (`set` / `add`), in (agent, rule) order.
   2. MOVE: compute desired positions. Occupancy conflicts go to the higher priority, then the lower `tiebreak = mix(seed, tick, agent_id)`. The hashed tie-break avoids systematic bias toward old or low-ID agents, which is the same problem NetLogo solves with random ask order.
   3. ACT interactions (`attack` before `eat`). Group them by target. Contested targets use the same priority-then-tiebreak rule. Losers get nothing, and their costs are not charged (charge on success).
   4. `reproduce` / `spawn` / `PRODUCES`. New `AgentId`s are allocated in `(parent_id, intent_index)` order.
   5. `transform_into`.
   6. `die`: explicit deaths, eaten agents, and agents with `energy <= 0`.
   7. Run `on_birth` and `on_death` in a **reaction sub-phase**. It allows only EFFECT actions plus `spawn`, has cascade depth 1, and handles agents in `AgentId` order.
   8. Detect collisions after movement and queue them as `on_collision` events for tick t+1.
5. **Environment.** Diffuse and decay the `emit` fields and update climate. Use fixed-order stencils. **Statistics** come last.

Floating-point arithmetic is deterministic for the same binary on the same platform. Cross-platform bit-exactness, for example x86 versus aarch64 FMA contraction, is **not** guaranteed and **UNVERIFIED**. If replays must be identical across machines, keep critical state in integers or fixed point.

### 2.8 Where Rhai fits

The DSL has two escape-hatch forms:
- `WHEN script("my_condition")`: a Rhai function `fn my_condition(org) -> bool`, compiled in the expression-only profile with `max_operations` around 2,000.
- `THEN script_action("my_action")`: a function that returns `Action` or an array of `Action`s.

These are validated at load with `ast.iter_functions()`, which checks the name and `params.len()`. They run in the decision phase through the same per-call tag and the same keyed RNG. Errors become W/E diagnostics. On a runtime budget error, the rule is disabled for the rest of the run and an editor notification is raised.

---

## 3. Relationship editor semantics

A relationship is a typed directed edge `subject --relation(params)--> object`. The object is a species, tag or resource. The relationship compiler turns edges into derived **tags**, such as `food`, `predator`, `prey`, `threat` and `mate`, and into **generated rules** with `origin: Relationship { id, relation }`. Generated rules are read-only in the rule list. "Detach" copies a generated rule into a manual one and removes the link. The compiler always produces the same rule ids from the same graph (`gen:{rel_id}:{n}`), so cooldown, RNG streams and saved diffs stay stable.

Priority bands:
- 1000: survival-critical (`die`)
- 900: flee
- 700: defend
- 500: feed
- 400: compete
- 300: reproduce
- 200: social / trade
- 100: produce
- 0: idle / wander

Users can shift a relationship within its band (±99).

| Relation | Meaning | Parameters | Derived tags | Generated rules (for the subject unless noted) |
|---|---|---|---|---|
| `consumes(A,B)` | A eats B | `gain`, `hunger_threshold`, `sense_radius`, `bite` | B ∈ `food(A)`; A ∈ `predator(B)` if B is mobile | `WHEN energy < hunger_threshold AND nearby(B, r) > 0 THEN move_towards(nearest(B))` P500. `ON on_collision with B WHEN energy < satiation THEN eat(other)` P520, `gain` added on success. For B (if mobile and `B avoids A` is not already set): offer to add an `avoids` edge, never added automatically. |
| `chases(A,B)` | A pursues B without eating it | `radius`, `speed_mult` | | `WHEN nearby(B, r) > 0 THEN move_towards(nearest(B), speed=…)` P450 |
| `avoids(A,B)` | A flees from B | `radius`, `speed_mult` | B ∈ `threat(A)` | `WHEN nearby(B, r) > 0 THEN move_away(nearest(B), speed=…)` P900 |
| `protects(A,B)` | A defends B from B's threats | `radius`, `damage` | | Let `T = threat(B) ∪ predator(B)`, computed after all edges. `WHEN nearby(B, r) > 0 AND nearby(T, r) > 0 THEN attack(nearest(T))` P700. Also `move_towards(nearest(B))` when `distance(nearest(B)) > r` at P150, to stay close. |
| `transforms(A→B)` | A becomes B, for example a caterpillar becoming a butterfly or a tree becoming ash | `condition` (Expr, required), `probability` | | `WHEN condition THEN transform_into(B)`, at P800 if death-like, otherwise P600 |
| `produces(A,X)` | A emits a resource, signal or entity X | `rate` (per tick probability) or `every`, `amount`, `cost` | X ∈ `product(A)` | X is a signal or field: `ON every(n) THEN emit(X, amount)`. X is an entity: `WHEN chance(rate) THEN spawn(X)` P100 with the cost |
| `reproduces_with(A,B)` | sexual reproduction; A = B means same species | `maturity_age`, `min_energy`, `radius`, `probability`, `offspring` (default A), `energy_share` | B ∈ `mate(A)` | `WHEN age > maturity AND energy > min_energy AND nearby(mate, r) >= 1 THEN reproduce(partner=nearest(mate), probability=p)` P300 `COOLDOWN` `COST`. Resolution needs a **reciprocal intent**: both partners must emit `reproduce` targeting each other in the same tick, or the edge is set to `consent=one_sided`. Pairs are matched in (min id, max id) order. |
| `competes_with(A,B)` | rivalry over shared resources; symmetric | `aggression` ∈ [0, 1], `radius` | | Let `R = food(A) ∩ food(B)`. If R = ∅, warn: competition without a shared resource. `WHEN nearby(B, r) > 0 AND nearby(R, r) > 0 THEN attack(nearest(B), probability=aggression)` P400. Resolution: in contested `eat` on R, a competitor's priority adds `aggression × 10`. |
| `trades_with(A,B)` | exchange between civilization agents | `give: (prop, qty)`, `receive: (prop, qty)`, `radius`, `surplus` | B ∈ `partner(A)` | `WHEN nearby(B, r) > 0 AND give.prop > surplus THEN trade(nearest(B), give, receive)` P200. `trade` is a **two-party intent** that is applied only when B has a matching complementary intent this tick, matched in (min id, max id) order. It is atomic: both property changes happen or neither does. |

Compilation algorithm:

1. Validate each edge: the endpoints exist and the parameters are in range.
2. Compute the derived tag sets until they stop changing. `threat` and `predator` are used by `protects`, so iterate a closure over the graph. It is a single pass because tags are not recursive except `protects`, which only reads them.
3. Generate rules per subject species in a stable order: edges sorted by `(relation_order, object, rel_id)`.
4. Merge them with manual rules. Report conflicts:
   - `A consumes B` and `A protects B` gives W201, "contradiction".
   - `A avoids B` and `A chases B` gives W202.
   - Over-full priority bands give W203.
   - A `transforms` cycle with no conditions gives W103.
5. Emit the `RuleSet` for each species. The rule list shows generated rules with a chain icon and a "from fox → rabbit (consumes)" label.

Example. Graph: `fox consumes rabbit`, `rabbit consumes grass`, `rabbit avoids fox`, `dog protects rabbit`, `cloud produces rain`.
- Derived: `food(rabbit) = {grass}`, `threat(rabbit) = {fox}`.
- Dog: `WHEN nearby("rabbit", 6) > 0 AND nearby({fox}, 6) > 0 THEN attack(nearest({fox}))` P700.
- Rabbit: flee P900 > graze P520 > forage-move P500 > breed P300 > wander P0.

The graphcheck program (section 4) renders this graph headless.

---

## 4. Graph visualization in egui 0.36 (bevy_egui 0.43.1)

Versions, licenses and egui requirements below come from `cargo info` and each crate's `Cargo.toml` in the registry (verified):

| Crate | Version | License | Depends on | Works with egui 0.36.2? | Notes |
|---|---|---|---|---|---|
| **egui_graphs** | 0.32.0 | MIT | `egui 0.36` (no default features, `persistence`), `petgraph 0.8` (`stable_graph`, `serde-1`, …), `rand 0.9`, `serde` | **Yes**, resolves and compiles | Built-in layouts: `LayoutForceDirected<FruchtermanReingold>` (plus `…WithCenterGravity` and `…WithExtras` with custom `ExtraForce`), `LayoutHierarchical`, `LayoutRandom`. `GraphView::<S, L>::new().with_id(..).show(ui, &mut graph) -> GraphViewResponse { response, changes }`. `fast_forward*` pre-settles the layout. Parallel edges are drawn curved (`order`, `curve_size`) and self-loops are supported (`draw/displays_default/edge.rs`). Labels: `add_node_with_label_and_location` and `add_edge_with_label`. |
| **egui-snarl** | 0.12.0 | MIT OR Apache-2.0 | `egui 0.36`, `rust-version 1.92` | **Yes**, resolves and compiles | Node-graph editor with pins and wires: `Snarl<T>`, `insert_node(pos, T) -> NodeId`, `connect(OutPinId, InPinId) -> bool`, `SnarlWidget::new().id_salt(..).show(&mut snarl, &mut viewer, ui)`. You implement the `SnarlViewer<T>` methods `title`, `inputs`, `outputs`, `show_input`, `show_output` (`-> impl SnarlPin`, e.g. `PinInfo::circle()`). Optional `serde`. |
| egui-graph-edit | 0.9.1 | MIT | `egui 0.35` | **No** | |
| egui_node_graph2 | 0.7.0 | MIT | `egui 0.29` | **No** | |
| fdg-sim | 0.9.1 | MIT | `petgraph 0.6`, `glam 0.21` | layout only, old dependencies | not recommended |
| force_graph | 0.4.0 | MIT | `petgraph 0.8.3` | layout only | an option if we want layout without the widget |

Verification:
1. `scratchpad/rhai-check/graphcheck` resolved `bevy_egui =0.43.1` + `egui_plot =0.37.0` + `egui_graphs =0.32.0` + `egui-snarl =0.12.0` + `rhai =1.27.0` (lockfile kept as `Cargo.lock.with-bevy-egui`). `cargo tree -i egui@0.36.2` shows a **single** `egui v0.36.2` used by bevy_egui, egui_plot, egui_graphs, egui-snarl and egui-scale. The only duplicate is `rand 0.9.5` (from egui_graphs) next to `rand 0.10.3`.
2. A headless egui 0.36.2 program, without Bevy so that it compiles quickly, ran 5 frames of `Context::run_ui`. Each frame drew an egui_graphs FR view, an egui-snarl rule graph (`connect` returned `true`) and a custom `Painter` force-directed graph (20 shapes per frame), and all of it tessellated. egui 0.36 gotcha: an unapplied `FullOutput::textures_delta` **panics on drop**, so headless tests must call `out.textures_delta.clear()`.

egui_graphs caveats (from source):
- Random initial placement uses `rand::rng()`, which is thread RNG. Use `add_node_with_location` (or the label variant) for reproducible screenshots.
- `show()` calls `ui.ctx().request_repaint()` every frame. That is fine inside Bevy's continuous loop, but it would defeat reactive or low-power UI modes.
- Layout state is stored in egui memory per `with_id(Some(..))`. Use distinct ids for multiple views.
- Graph data is a `petgraph::StableGraph` wrapped in `Graph<N, E, Ty, Ix, Dn, De>`, so indices stay stable when nodes are removed. Keep our own `RelationshipGraph` as the source of truth and rebuild or diff the view graph from it.

**Recommendation:**
- Relationship editor: an **egui_graphs** view with a FR layout, nodes coloured by trophic level, and edges coloured by relation type. Add a side panel to edit the selected edge's parameters, and a palette to drag new relations.
- Rule editor: a structured **Scratch-like block list** built from plain egui widgets. Each rule is a card containing an event hat, condition rows (`ComboBox` for property / comparator / `DragValue`) and action rows. This matches the IR one-to-one and makes invalid states impossible to build.
- **egui-snarl** offers an optional "graph mode" that shows conditions wired to actions.
- A **custom Painter FR layout** of about 100 lines (`graphcheck/src/main.rs`, `RelGraph::step`/`ui`) is the fallback if we need deterministic layout, typed multi-edge styling, or no `rand 0.9` dependency. Repulsion is O(n²), which is fine for ≤ 200 species nodes. Above that, use Barnes-Hut (**UNVERIFIED** need).

---

## 5. Verification artifacts

All paths are under `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/rhai-check/`.

| Path | What it checks | Result |
|---|---|---|
| `src/main.rs` (`cargo run`, `cargo run --release`) | sandbox engine; checks 1-7; timing | 29/29 PASS in both debug and release. Full log in `verification-output.txt`. |
| `examples/mapsize.rs` | where map, array and string size limits are enforced | found the map index-assignment gap (1.4) |
| `examples/exprmode.rs` | expression-only profile: no loops, no `fn`, no closures | verified |
| `examples/trycatch.rs` | budget errors cannot be caught | verified |
| `nosync/` | `Engine`/`AST` are not `Send + Sync` without `sync` | compile error, as expected |
| `unchecked/` | the limits API disappears under `unchecked` | compile error, as expected |
| `graphcheck/` | egui 0.36.2 + egui_graphs + egui-snarl + custom painter, headless | runs; resolution with bevy_egui confirmed |
| `irshape/` | rule IR round-trips through RON 0.12 and JSON | verified |

Check list from `src/main.rs`:
- 1: `loop {}` stopped by max_operations
- 1b: cancel flag → `ErrorTerminated`
- 1c: 50 ms deadline
- 2: recursion → `ErrorStackOverflow`
- 2b: 200 nested parens → `ExprTooDeep`
- 3: string doubling → `ErrorDataTooLarge`
- 3b: `pad(1e6)` → array limit
- 3c: map gap
- 3e: max_variables
- 4a: default engine imports from disk
- 4b/4d: `import` and `eval` disabled
- 4c: Dummy resolver
- 4e: `eval` is built in
- 4f: `sleep`, `exit`, `timestamp`, `parse_json`, file open not found
- 4g/h/i: `sleep` in `Engine::new()`; `disable_symbol` ineffective; override works
- 5: custom type + context functions return the right `Action`s
- 5b: host error with position
- 5c: `eval_ast` gotcha
- 6: parse and runtime diagnostics rendering
- 7: 8-thread shared engine

---

## 6. Risks and open questions

- **Rhai throughput.** About 0.85 µs per call means per-agent per-tick scripting is limited to roughly 10^4-10^5 agents. The fallback is the native IR (D1).
- **Map-size gap** (1.4). Rely on the ops budget and host-side checks of returned values. Re-check after a Rhai upgrade, since it may be fixed upstream (**UNVERIFIED** whether it is reported).
- **Feature unification.** If any workspace crate enables `rhai/unchecked` or `rhai/f32_float`, it applies to everyone. Pin features in a single `workspace.dependencies` entry. The `set_max_operations` call is the compile-time guard against `unchecked`.
- **Debug and release limit defaults differ** (8 versus 64 call levels). Always set limits explicitly.
- **`sleep` exposure** if anyone switches to `Engine::new()`. Add a unit test asserting `engine.run("sleep(0)")` returns `ErrorFunctionNotFound`.
- **egui_graphs** pulls in a second `rand` (0.9) and repaints continuously. Its API changed often across 0.2x releases (**UNVERIFIED** history), so pin `=0.32.0`.
- **Two-party actions** (`reproduces_with`, `trades_with`) need reciprocal-intent matching. That adds complexity to the resolver; specify it before implementation.
- **Cross-platform float determinism** is not guaranteed (2.7).
- **Grammar** (2.4) is designed but not yet implemented or fuzzed.

## 7. Sources

- Rhai 1.27.0 source (local registry): `src/engine.rs`, `src/api/{limits,events,mod,options,register,call_fn,compile,build_type}.rs`, `src/packages/{pkg_std,pkg_core,lang_core,time_basic}.rs`, `src/eval/{data_check,chaining}.rs`, `src/func/{call,func_call,native,hashing}.rs`, `src/types/{error,parse_error,position,token,scope}.rs`, `src/module/resolvers/{dummy,file}.rs`, `Cargo.toml.orig`. Upstream: <https://github.com/rhaiscript/rhai>. Book: <https://rhai.rs/book>.
- bevy_ecs 0.20.1 source: `src/resource.rs:87`, `src/component/mod.rs:530`.
- egui 0.36.2 source: `src/context.rs` (`run_ui`), `src/containers/panel.rs`.
- egui_graphs 0.32.0 source and examples: <https://github.com/blitzarx1/egui_graphs>, <https://docs.rs/egui_graphs/0.32.0>.
- egui-snarl 0.12.0: <https://github.com/zakarumych/egui-snarl>, <https://docs.rs/egui-snarl>.
- egui-graph-edit: <https://github.com/kamirr/egui-graph-edit>. egui_node_graph2: <https://github.com/trevyn/egui_node_graph2>.
- ron 0.12.3 README, Limitations section (local registry).
- rand_chacha 0.10.0 source: `src/chacha.rs` (`set_stream`, `set_word_pos`).
- NetLogo Dictionary: <https://docs.netlogo.org/dictionary.html> (`ask`: "agentset members are always read in a random order").
- GAMA, "Defining actions and behaviors": <https://gama-platform.org/wiki/DefiningActionsAndBehaviors>.
- StarLogo Nova wiki, "On Collision With Do": <https://wiki.slnova.org/wiki/On_Collision_With_Do>. StarLogo: <https://en.wikipedia.org/wiki/StarLogo>.
- scratch-blocks: <https://github.com/scratchfoundation/scratch-blocks>.
- M. Colledanchise and P. Ögren, "Behavior Trees in Robotics and AI: An Introduction", arXiv:1709.00084: <https://arxiv.org/abs/1709.00084>.
- CLIPS Basic Programming Guide §5.3, Conflict Resolution Strategies: <https://www.csie.ntu.edu.tw/~sylee/courses/clips/bpg/node5.3.html>.
- C. L. Forgy, "Rete: A fast algorithm for the many pattern/many object pattern match problem", *Artificial Intelligence* 19 (1982) 17-37, <https://doi.org/10.1016/0004-3702(82)90020-0>.
- Crate metadata via `cargo info`: bonsai-bt 0.14.0 (MIT), pest 2.9.5, lalrpop 0.23.2, chumsky 1.0.0-alpha.8, logos 0.16.1, ariadne 0.6.0, miette 7.6.0, codespan-reporting 0.13.1.

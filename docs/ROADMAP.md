# OtterLang v0.1 Roadmap

This roadmap captures the work required before the first public release, grouped by subsystem so each feature or fix can be driven to completion.

## Component Snapshot

| Component | Key Files | Status | Notes |
|-----------|-----------|--------|-------|
| Language front-end | `crates/otterc_*`, `docs/LANGUAGE_SPEC.md` | Ready | Parser/type checker implemented; needs async & pattern-guard decisions before freeze |
| Runtime + stdlib | `src/runtime/**/*`, `stdlib/otter/*` | Ready | Core features present; channels/tasks/gc need polish & docs |
| JIT & codegen | `src/codegen/llvm`, `src/runtime/jit/*` | Partial | Functional but hot-path optimizations stubbed; adaptive managers exist but rarely invoked |
| Tooling | `src/cli.rs`, `src/repl`, `crates/otterc_fmt`, `src/lsp`, `vscode-extension/` | Partial | Works end-to-end; REPL UX, CLI flags, formatter parity pending |
| Docs & release | `docs/*.md`, `.github/workflows` | Partial | Comprehensive docs exist; need install guide, GC/task write-ups, release automation |

## 1. Language & Type System

| Gap | Details | Relevant Files | Priority |
|-----|---------|----------------|----------|
| ~~`async` keyword unused~~ | Parser/lexer now treat `async` as a normal identifier until first-class async functions ship, removing the unused reserved keyword. | Parser, docs, tooling | Done |
| `await` limited to direct calls | Grammar forces `await` to wrap a call expression only (`crates/otterc_parser/src/grammar.rs:440-500`), so awaiting a stored handle fails. Either generalize the grammar or emit diagnostics. | Parser, `src/typecheck/checker.rs:2770-2805` | High |
| Traits/polymorphism (planned) | Contributor proposal introduces `trait` definitions, `impl` blocks, trait-qualified calls, and trait objects. Parser needs new productions/AST nodes (`crates/otterc_parser/src/grammar.rs`, `crates/otterc_ast`). Type checker must track trait tables, validate impls, and resolve trait bounds + trait objects (`src/typecheck/checker.rs`). Codegen/runtime need vtables and trait-object layouts for dynamic dispatch (`src/codegen/llvm`, `src/runtime/ffi`). Document the feature once implemented. | Parser, type checker, codegen/runtime, docs | High |
| Type checker modularity & diagnostics | `TypeCheckerWorkspace` now snapshots module exports/diagnostics and keeps per-module type maps so we can re-check just the files that changed and share symbol tables across modules. Next steps: wire the workspace into the CLI/LSP, expand `use` resolution, and surface cross-file diagnostics in the editor. | Type checker, LSP | Medium |
| Pattern matching lacks guards | `match_case` hard-codes `guard: None` (`crates/otterc_parser/src/grammar.rs:750-771`). Need to add `case Foo if cond` parsing/type checking or document it as unsupported. | Parser, `src/typecheck/checker.rs:1510-1600` | Medium |
| Loop/comprehension iteration story | `for` only handles ranges/lists/strings (`src/codegen/llvm/compiler/stmt.rs:296-371`), and comprehensions demand list iterables (`src/codegen/llvm/compiler/expr.rs:2227-2405`). Introduce an iterator trait or produce compile errors hinting at `range()` usage. | Codegen, docs | High |
| Range expressions allocate eagerly | `start..end` expands to `otter_builtin_range_*` and creates a list immediately (`src/runtime/stdlib/builtins.rs:472-510`). Document the memory impact or add a lazy iterator. | Stdlib, docs | Medium |
| ~~Type checker unaware of task handles~~ | `await` now peels `Task<T>`/`Future<T>` handles and produces the inner type so user code can reason about spawned task results without resorting to shared state plumbing. | Type checker, runtime | Done |

## 2. Codegen, JIT, and Runtime Execution

| Gap | Details | Relevant Files | Priority |
|-----|---------|----------------|----------|
| `await` drops values | `eval_await_expr` ignores any result (`src/codegen/llvm/compiler/expr.rs:20-35`); tasks must write into shared state. Decide whether to plumb return values through handles or clearly document the limitation (examples/docs). | Codegen, docs | High |
| `spawn` context management | Captured variables are serialized via `SPAWN_CONTEXTS` (`src/codegen/llvm/compiler/expr.rs:38-140`, `src/runtime/stdlib/task.rs:28-74`) but never freed if the task never consumes them. Need explicit drop logic or GC hooks. | Codegen, task runtime | High |
| Hot JIT plumbing incomplete | `JitExecutor::optimize_function` is a no-op (`src/runtime/jit/executor.rs:37-67`), adaptive concurrency/memory managers are instantiated but unused (`src/runtime/jit/engine.rs:28-76`), and the hot detector relies on thresholds without integration tests. Implement real recompilation paths or remove unused managers. | `src/runtime/jit/*` | High |
| ~~Task runtime feature gating~~ | CLI now errors when `--tasks`, `--tasks-debug`, or `--tasks-trace` are used without compiling with `--features task-runtime`, making the feature detection story explicit until the runtime ships by default (`src/cli.rs`). | Task stdlib, CLI | Done |
| ~~Channel performance & semantics~~ | `TaskChannel` uses a `VecDeque` ring buffer, deduplicates registered wakers, drains them on close, and carries backlog/waiter metrics with close-before-send tests guarding regressions (`src/runtime/task/channel.rs`). | Task channel | Done |
| Iterator coverage | No iterator abstraction for maps/custom structs; everything funnels through `__otter_iter_array/string` FFI helpers. Add runtime traits or helper APIs so user-defined collections can participate in `for` and comprehensions. | Runtime FFI, docs | Medium |
| Stdlib surface audit | Several stdlib modules share inconsistent naming/return conventions (`stdlib/otter/*.ot`, `src/runtime/stdlib/*`). `io`/`fs` mix strings and `Option` for error reporting, `http`/`net` each define their own response structs, `yaml/json` return naked strings instead of typed values, and `task` vs. `sync` expose incompatible handle types. We need a systematic pass that (a) defines shared error/result structs, (b) standardizes naming (e.g., `new`, `open`, `close`), (c) documents every exported symbol in `docs/API_REFERENCE.md`, and (d) wires those docs into the LSP/CLI so completions show accurate signatures. | Stdlib Otter files + `src/runtime/stdlib` | Medium |
| Deterministic/random APIs | `stdlib/otter/rand.ot` is a thin wrapper around `src/runtime/stdlib/rand.rs` and only exposes global functions (`seed`, `int`, `float`). There is no concept of independent RNG instances, cryptographic sources, or reproducible streams per task. Add RNG structs (e.g., `Random`), thread-safe seeding, ability to consume OS entropy, and docs/tests demonstrating deterministic replay vs. secure randomness. | `stdlib/otter/rand.ot`, `src/runtime/stdlib/rand.rs` | Medium |
| Time + scheduling utilities | `stdlib/otter/time.ot` offers formatting and `sleep_ms`, but lacks monotonic timers, duration arithmetic, or integration with the task runtime. Align `time` APIs with `src/runtime/stdlib/time.rs`, expose Duration/Instant types, and ensure `task` helpers (timeouts, scheduling) reuse the same primitives so users don’t reach into the runtime directly. | `stdlib/otter/time.ot`, `src/runtime/stdlib/time.rs`, task runtime | Medium |
| FFI surface expansion | The FFI layer currently exposes only hand-written helpers; complicated Rust crates require manual symbol plumbing (`src/runtime/ffi`, `crates/ffi/*`). To interop with more libraries we need a richer binding story (trait/object support, better metadata in `crates/ffi/src/rust_stubgen.rs`, runtime loaders with versioning, and docs showing how to register new modules). | FFI crates, runtime FFI, docs | High |
| Task runtime depth | The task runtime combines async-ish helpers (`stdlib/otter/task.ot`) with a scheduler backed by `src/runtime/task` and `src/runtime/jit/concurrency/*`, but only exposes spawn/join/sleep/channel APIs. We need: per-task cancellation, timeout-aware waits, structured concurrency (scoped tasks), better metrics/telemetry, and docs/examples that cover select, typed channels, and coordination with `sync` primitives. Implementation touches `src/runtime/task/channel.rs`, `src/runtime/task/mod.rs`, and the CLI flags that toggle task instrumentation. | Task runtime, stdlib/task, CLI/docs | High |

## 3. Memory Management & GC

| Gap | Details | Relevant Files | Priority |
|-----|---------|----------------|----------|
| ~~GC configuration hidden~~ | CLI now exposes `--gc-*` switches that feed directly into `OTTER_GC_*`, and the getting-started guide documents the available strategies/threshold knobs. | CLI, runtime config, docs | Done |
| ~~Arena/root APIs undocumented~~ | `docs/GC_GUIDE.md` now walks through GC tuning, root registration, and arena allocation with FFI examples so developers can safely manage memory. | Docs | Done |
| REPL recompiles everything | `ReplEngine` reparses and rebuilds LLVM modules on every input (`src/repl/engine.rs:73-150`), creating temp libraries and GC allocations repeatedly. Implement incremental compilation or trimming so long sessions do not leak memory. | REPL, JIT | Medium |
| GC strategy implementation gaps | `src/runtime/memory/gc.rs` defines reference counting, mark-sweep, and generational collectors, but only mark-sweep sees regular exercise. Reference-counting lacks cycle detection, generational GC never promotes objects, and `GcStrategy::None` is untested. We need test coverage for each strategy, runtime metrics to surface collector stats, and a way to hot-swap strategies based on `GcConfig`. | Runtime memory subsystem | High |
| Arena allocator integration | The arena API (`src/runtime/memory/arena.rs`, `src/runtime/stdlib/gc.rs`) is disconnected from the rest of the allocator story—Otter programs cannot tie arenas to lifetimes or automatically reset them. Add high-level Otter APIs (`stdlib/otter/gc.ot`), ensure arenas respect GC roots, and document how arenas interact with FFI allocations. | Runtime memory + stdlib | Medium |
| Memory profiler visibility | `src/runtime/memory/profiler.rs` can record allocations, but no user-facing tool surfaces the data. Integrate it with `otter profile` and expose hooks in the CLI/LSP so developers can inspect object counts, GC pauses, and arena usage. | Memory profiler, CLI, tooling | Medium |

## 4. Tooling & Developer Experience

| Gap | Details | Relevant Files | Priority |
|-----|---------|----------------|----------|
| REPL UX | No multiline editing/history, results are not printed, and every snippet wraps a fake `__repl_expr` function (`src/repl/engine.rs:73-150`). Implement a persistent evaluator that reuses compiled code, prints expression results, and exposes commands (import, reset, inspect). | REPL | High |
| CLI flag validation | Flags such as `--tasks`, `--tasks_debug`, and GC env vars have no validation or help text beyond clap defaults (`src/cli.rs:16-120`). Add cohesive `--gc-*`/`--task-*` behavior and update `INSTALL.md`/`README.md`. | CLI, docs | Medium |
| LSP capability gaps | The LSP hardcodes completions/snippets (`src/lsp/mod.rs:15-90`), rebuilds a fresh `TypeChecker` per request, and only sees the active document. Add a module-aware index, workspace symbol table, semantic tokens, and go-to-definition/reference support across files so the VS Code extension reflects real project structure. | LSP, type checker | Medium |
| Formatter/LSP parity | `crates/otterc_fmt` and VS Code grammar need to stay in sync with the current spec (spawn/await spacing, comprehension layout). Audit formatter output and update `vscode-extension/syntaxes/otterlang.tmLanguage.json`. | Formatter, VS Code extension | Medium |
| Profiler/test docs | `otter profile` and `otter test` expose powerful tools, but there are no docs or sample outputs. Extend `docs/TUTORIALS.md` and `docs/EXAMPLES.md` to explain profiler metrics, snapshot testing, and CI usage. | Docs, tooling | Low |

## 5. Documentation, Examples, and Release Engineering

| Gap | Details | Relevant Files | Priority |
|-----|---------|----------------|----------|
| Missing task/sync tutorials | Concurrency modules (`stdlib/otter/task.ot`, `stdlib/otter/sync.ot`) lack corresponding tutorials/examples. Add runnable demos under `examples/` (typed channels, wait groups, select) and reference them from `docs/TUTORIALS.md`. | Examples, docs | Medium |
| GC & env var docs | Neither `docs/LANGUAGE_SPEC.md` nor `docs/FFI_GUIDE.md` mention `OTTER_GC_STRATEGY`, arenas, or GC hooks. Add a dedicated GC section plus environment variable reference. | Docs | High |
| FFI exception bridge | The runtime exposes exception helpers (`src/runtime/stdlib/exceptions.rs`), but FFI docs never describe how to use them. Update `docs/FFI_GUIDE.md`/`docs/FFI_TRANSPARENT.md` with examples. | Docs | Medium |
| Install & packaging story | There is no documented process for installing binaries, bundling the stdlib, or setting env vars once a release is cut. Expand `INSTALL.md` and `README.md` with platform-specific steps, and ensure CI publishes archives. | Docs, CI | High |
| Release automation | `.github/workflows/ci.yml` only runs tests (`.github/workflows/ci.yml:1-120`). Add workflows for tagging releases, building macOS/Linux/Windows artifacts, bundling stdlib, and publishing the VS Code extension. | CI, release | High |
| Roadmap visibility | Once blockers start closing, mirror a short status summary in `README.md` so contributors know what’s left for v0.1. | README | Low |

## 6. Tracking Table

| Area | Owners | Deliverables |
|------|--------|--------------|
| Language/spec | Language WG | Remove/implement `async`, expand iterator support, finalize spec + regression tests |
| Runtime/JIT | Runtime WG | Task handle semantics, spawn context cleanup, enable hot-function recompilation |
| Memory/GC | Runtime WG | CLI/ENV integration, GC/arena documentation, REPL memory plan |
| Tooling/DX | DX WG | REPL redesign, formatter/LSP audit, CLI flag UX |
| Docs/Release | DevRel + Release | Tutorials/examples, install guide, release automation, roadmap visibility |

Update this document whenever a gap is closed or a new blocker emerges so the path to v0.1 stays aligned with the codebase.

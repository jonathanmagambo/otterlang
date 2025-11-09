## OtterLang Roadmap

This roadmap outlines major areas of investment and their intended scope. Items are prioritized by immediate impact and developer needs, with core language features and essential tooling coming first. Timelines are indicative and may evolve based on feedback.

### Guiding Principles
- Simplicity first: syntax and tooling should be easy to learn and hard to misuse
- Performance by default: native codegen, low overhead abstractions
- Safety with explicit control: predictable resource and error handling
- Batteries included: great stdlib, formatter, profiler, and REPL

---

### Milestone 1: Core Language Features (HIGH PRIORITY)

Essential language features needed for practical development.

- **Error Handling**
  - Add `Result<T, E>` and `Option<T>` as first-class types
  - `?` operator to early-return on error from `Result<T, E>`
  - Standard library helpers for mapping/combining results
  - `panic`/`raise` for unrecoverable states with backtrace hooks
  
- **Pattern Matching**
  - Exhaustive `match` on `Result`/`Option` and algebraic data types
  - Destructuring and guards in `match` arms
  - Type-safe pattern matching with compile-time exhaustiveness checking
  
- **Type System Improvements**
  - Better type inference (reduce need for explicit type annotations)
  - Strong type aliases (newtype pattern): `type UserId = int`
  - Enhanced type error messages with suggestions

### Milestone 2: Essential Developer Experience (HIGH PRIORITY)

Critical tooling needed for daily development workflow.

- **Testing Framework**
  - Built-in test runner (`otter test`) and assertion library
  - Test discovery and execution
  - Snapshot testing hooks and timing output
  - Clear error reporting with diffs and spans
  
- **Improved Diagnostics**
  - Rich error messages with spans and suggestions
  - Better type error reporting
  - Compiler suggestions for common mistakes
  
- **Formatter and LSP Basics**
  - Incremental formatter improvements
  - First-party LSP features (hover, go-to-def, diagnostics)
  - Syntax highlighting and basic autocomplete

### Milestone 3: Package Management and Ecosystem (HIGH PRIORITY)

Necessary for building real-world projects and sharing code.

- **Package Manager (otterpkg)**
  - Dependency resolution and version management
  - Local and remote package registry support
  - Lockfile generation and reproducible builds
  - Integration with `otter build` and module system
  - Project manifest (`Otter.toml`) for package metadata, dependencies, scripts, and targets
  
- **Standard Library Expansion**
  - Collections improvements (more methods, better ergonomics)
  - Iterators and iterator adapters
  - IO/FS/Net ergonomics and safety improvements
  - Async utilities and better concurrency primitives

### Milestone 4: Performance Optimizations (MEDIUM PRIORITY)

Important optimizations that improve runtime performance.

- **Static Dispatch Optimization**
  - Compile-time method resolution for known types (no runtime vtable lookup)
  - Operator overloading resolved statically based on static types
  - Direct function calls instead of dynamic dispatch when types are known
  - Opt-in dynamic dispatch via `dynamic` type when needed
  
- **Partial Evaluation and Constant Propagation**
  - Automatic identification of compile-time evaluable expressions
  - Eager evaluation of all statically known expressions during compilation
  - Guaranteed constant folding for statically known values
  - Eliminates runtime overhead for operations on compile-time constants
  
- **Module Freezing and Static Optimization**
  - Freeze global constants and module-level data after initialization phase
  - Aggressive constant propagation and dead code elimination
  - Static module resolution: all imports resolved at compile time
  - Immutable module/class hierarchy after initialization enables better optimization
  
- **Enhanced Type System Soundness**
  - Runtime type checks for soundness guarantees
  - Type-checked code cannot have runtime type errors (mathematical guarantee)
  - Better integration between static and runtime type checking
  - Predictable performance: no "performance cliffs" from subtle code changes

### Milestone 5: Advanced Language Features (MEDIUM PRIORITY)

Powerful features that enable advanced use cases.

- **Compile-Time Evaluation System**
  - `@compile` function annotations for compile-time execution with zero runtime cost
  - Compile-time metaprogramming in OtterLang itself (not just Rust macros)
  - Generics implemented as compile-time functions (e.g., `List[T]` expands at compile time)
  - Zero-cost abstractions: decorators, property descriptors, custom operators
  
- **Resource Management**
  - `defer` statement for guaranteed cleanup (file handles, database connections, locks)
  - Multiple defer statements execute in reverse order (LIFO)
  - Complements error handling by guaranteeing cleanup even on early returns

### Milestone 6: Advanced Runtime Features (LOW PRIORITY)

Specialized runtime capabilities for specific use cases.

- **Per-function JIT**
  - Julia-style per-function JIT for hot paths
  - Tiered execution (interp/JIT/native) with profiling feedback
  - Manual JIT control for predictable AOT performance with optional dynamic compilation
  - Explicit `jit` annotation for functions requiring JIT compilation
  
- **Live Module Loading**
  - Reloadable modules for REPL and long-lived processes
  - Stable ABI for safe hot-swapping within a process

### Milestone 7: Embedded and Specialized Features (LOW PRIORITY)

Features for specific domains and use cases.

- **Embedded Development Features**
  - Memory section control: explicit variable placement in memory sections (`.rodata`, `.data`, `.bss`)
  - Cleaner bit manipulation syntax (set, clear, toggle, test bits)
  - Type-safe bit field operations with compile-time validation
  - Critical section blocks for thread/coroutine-safe code sections
  
- **Embeddable Runtime and Sandbox**
  - Bytecode interpreter and VM for embedded use cases
  - Sandboxed execution environment with capability-based security
  - Resource limits (memory, CPU, execution time)
  - Rust host integration with safe API for creating and managing VM instances
  - Game engine integration and plugin system

### Milestone 8: Documentation and Community (ONGOING)

Essential for adoption and community growth.

- **Documentation and Website**
  - Official documentation site with search and examples
  - Interactive API reference with live code samples
  - Tutorials and guides for common use cases
  - Package registry browser and search
  
- **Observability**
  - Built-in tracing and structured logs
  - Performance profiling tools

---

### Acceptance Criteria (Examples)

**Milestone 1: Core Language Features**
- Error handling: A function returning `Result<T, E>` can be composed with `?`
- Pattern matching: `match` on `Result`/`Option` is exhaustive and type-checked
- Type system: Strong type aliases prevent type confusion at compile time (e.g., `UserId` ≠ `ProductId` even if both are `int`)

**Milestone 2: Essential Developer Experience**
- Testing: `otter test` discovers and runs tests, returning non-zero on failure
- Testing: Assertions report clear diffs and spans
- Diagnostics: Error messages include spans, suggestions, and context

**Milestone 3: Package Management and Ecosystem**
- Package manager: `otterpkg init` creates a new project with dependencies
- Package manager: `otterpkg add <package>` resolves and installs dependencies
- Package manager: `otterpkg build` uses lockfile for reproducible builds
- Package manager: `Otter.toml` is parsed and validated (name, version, deps, scripts, targets)
- Package manager: `otterpkg run <script>` executes scripts defined in `Otter.toml`

**Milestone 4: Performance Optimizations**
- Static dispatch: Method calls resolved at compile time for known types produce direct function calls
- Constant propagation: Compile-time evaluable expressions are fully evaluated during compilation
- Module freezing: Globals cannot be mutated after initialization phase
- Type soundness: Type-checked code has runtime type safety guarantees

**Milestone 5: Advanced Language Features**
- Compile-time evaluation: `@compile` functions execute during compilation and produce no runtime code
- Compile-time evaluation: Generics work as compile-time functions (e.g., `List[T]` expands at compile time)
- Resource management: `defer` statements guarantee resource cleanup even on error paths

**Milestone 6: Advanced Runtime Features**
- JIT: A hot function receives JIT compilation with measurable speedup
- Live module loading: Module swapping retains state isolation guarantees

**Milestone 7: Embedded and Specialized Features**
- Embedded: Memory section control allows explicit placement of data in read-only sections
- Embedded: Bit manipulation syntax is readable and type-safe (no more `|= (1 << 5)` patterns)
- Embeddable runtime: VM can be instantiated from Rust with configurable sandbox permissions
- Embeddable runtime: OtterLang code runs in isolated environment with resource limits

---

## Priority Summary

**Immediate Focus (Milestones 1-3):**
- Core language features (error handling, pattern matching, type system)
- Essential developer tooling (testing, diagnostics, LSP)
- Package management and ecosystem basics

These milestones provide the foundation for practical OtterLang development and enable real-world projects.

**Near-term (Milestones 4-5):**
- Performance optimizations for better runtime speed
- Advanced language features for power users

**Long-term (Milestones 6-8):**
- Specialized runtime features (JIT, live reloading)
- Domain-specific features (embedded development, sandboxing)
- Documentation and community resources

---

### Out of Scope (for now)
- Distributed runtime
- Full interpreter mode (AOT compilation is primary; JIT/interpreter are for specific use cases)

### Feedback
Feedback is welcome via issues and discussions. Priorities will be adjusted based on real-world usage.



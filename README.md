<h1 align="center">OtterLang</h1>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/jonathanmagambo/otterlang/blob/main/image.png?raw=true" width="400">
    <img src="https://github.com/jonathanmagambo/otterlang/blob/main/image.png?raw=true" width="400" alt="OtterLang Logo" />
  </picture>
  <br>
  <strong>Simple syntax, native performance, transparent Rust FFI.</strong>
  <br><br>
  
  [![Build Status](https://github.com/jonathanmagambo/otterlang/workflows/CI/badge.svg)](https://github.com/jonathanmagambo/otterlang/actions)
  [![Discord](https://img.shields.io/badge/Discord-Join%20Server-5865F2?style=flat&logo=discord&logoColor=white)](https://discord.gg/y3b4QuvyFk)
  
  <br><br>
  An indentation-sensitive programming language with an LLVM backend. OtterLang compiles to native binaries with a focus on simplicity and performance.
</div>

<h3 align="center">
  <a href="docs/ROADMAP.md"><b>Roadmap</b></a>
  &nbsp;&#183;&nbsp;
  <a href="docs/GETTING_STARTED.md"><b>Getting Started</b></a>
  &nbsp;&#183;&nbsp;
  <a href="docs/EXAMPLES.md"><b>Examples</b></a>
</h3>

<h1 align="center">Quick Start</h1>

Get started with OtterLang in just a few commands:

```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Environment setup (requires Rust nightly + LLVM 18)
nix develop            # recommended shell with all dependencies
cargo +nightly build --release

# Create and run your first program
cat > hello.ot << 'EOF'
fn main():
    println("Hello from OtterLang!")
EOF

./target/release/otter run hello.ot
```

See the [Getting Started Guide](docs/GETTING_STARTED.md) for detailed installation and usage instructions.

<h1 align="center">Getting Started</h1>

1. Install Rust nightly (`rustup toolchain install nightly`) and ensure LLVM 18 is available (or simply run `nix develop`).
2. Build the toolchain: `cargo +nightly build --release`.
3. Run code with `otter run file.ot` or emit binaries with `otter build file.ot -o my_app`.

The [Getting Started Guide](docs/GETTING_STARTED.md) expands on editor setup, cache usage, the REPL, and snapshot testing.

<h1 align="center">Language Features</h1>

OtterLang pairs indentation-aware syntax with modern language constructs:

1. **Whitespace-driven grammar** – no braces or semicolons, just `fn`, blocks, and meaningful indentation.
2. **Static typing with inference** – optional annotations, tuples, enums, and generics.
3. **Structured error handling** – `Result<T, E>` enum with pattern matching plus `panic`/`recover` utilities.
4. **Async task runtime** – `spawn` and `await` built on the Otter task scheduler.
5. **Generational GC** – explicit root APIs (`gc.alloc`, `gc.add_root`, `gc.remove_root`) keep FFI safe.
6. **Transparent Rust FFI** – import any crate with `use rust:crate_name` and call it directly.

### Transparent Rust FFI

The compiler shells out to `cargo`/rustdoc, normalizes the crate’s public API, and generates a native bridge automatically. Import crates directly from OtterLang (`use rust:serde::{json}`) without touching build scripts. See the [FFI Guide](docs/FFI_GUIDE.md) for configuration, caching, and troubleshooting tips.

### Standard Library

Built-in modules cover IO, math, JSON, tasks, runtime helpers, strings, networking, testing, and more. The [API Reference](docs/API_REFERENCE.md) documents every exported function.

Only the true language primitives (enums, `Option`/`Result`, `panic`, `print`, `len`, strings, lists, maps, and arithmetic) live in the implicit prelude. All other stdlib functionality now follows a Python-style import model—`use http`, `use yaml`, `use task`, etc.—and nothing outside the prelude is visible until you import the module you need.


<h1 align="center">Command Line Interface</h1>

The `otter` binary drives every workflow:

```bash
otter run program.ot            # Compile + execute
otter build program.ot -o app   # Emit native binary
otter fmt                       # Format .ot files
otter repl                      # Interactive REPL
otter test path/to/tests        # Run snapshot-style tests
```

Cross-compilation targets (including WebAssembly) are described in [docs/WEBASSEMBLY.md](docs/WEBASSEMBLY.md).

<h1 align="center">Examples</h1>

Browse [docs/EXAMPLES.md](docs/EXAMPLES.md) and the `examples/` tree for runnable snippets that stress the parser, runtime, and FFI bridge.

<h1 align="center">VSCode Extension</h1>

We ship a VS Code extension with syntax highlighting, snippets, diagnostics, and an LSP server. Installation and release notes live in [vscode-extension/README.md](vscode-extension/README.md).

<h1 align="center">Documentation</h1>

<div align="center">

**[Roadmap](docs/ROADMAP.md)** – project roadmap and future plans.

**[Getting Started](docs/GETTING_STARTED.md)** – installation, CLI walkthrough, first project.

**[Examples](docs/EXAMPLES.md)** – curated sample programs.

**[Tutorials](docs/TUTORIALS.md)** – guided walkthroughs for specific topics.

**[API Reference](docs/API_REFERENCE.md)** – stdlib module documentation.

**[WebAssembly](docs/WEBASSEMBLY.md)** – compiling OtterLang programs to WASM targets.

</div>

<h1 align="center">Project Status</h1>

**Early Access (v0.1.0)** – experimental tooling, expect sharp edges.

### Known Limitations

- `match` arms do not support guard clauses (`case value if ...`) yet; only direct patterns are accepted.
- Transparent Rust FFI exposes functions and methods, but macros/proc-macros are ignored and structs/enums cross the boundary as opaque handles (see [FFI Guide](docs/FFI_GUIDE.md#limitations)).
- WebAssembly builds run without filesystem access or full FFI support, so many stdlib modules (`io`, `net`, `task`) are unavailable in that target ([details](docs/WEBASSEMBLY.md#limitations)).
- Building the toolchain currently requires LLVM 18 and Rust nightly because the bridge generator depends on nightly-only rustdoc features.

<h1 align="center">Contributing and License</h1>

> [!IMPORTANT]
> OtterLang is currently in **Early Access (v0.1.0)** and is being actively developed. The project is experimental, and you may encounter sharp edges. **If you plan to contribute to the project, now is the time to provide a helping hand for the hardworking team**. Your contributions during this early stage will help shape the future of OtterLang.

OtterLang is free and open source, released under the **BSD-3-Clause License**. The license allows you to:

- View the source code and learn from it
- Use it for any purpose, including commercial use
- Modify and distribute the code
- Use it in proprietary software

Contributions are welcome with open arms as we look to foster a community. Please proceed to take a look at [CONTRIBUTING.md](./CONTRIBUTING.md) for more information on how to get started, as well as the codebase to learn from it. We sincerely and deeply are grateful and thankful for your efforts.

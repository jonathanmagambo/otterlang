<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/Ottrlang/otterlang/blob/main/image.png?raw=true" width="400">
    <img src="https://github.com/Ottrlang/otterlang/blob/main/image.png?raw=true" width="400" alt="OtterLang Logo" />
  </picture>
</div>

<h1 align="center">OtterLang</h1>

<h3 align="center">
  <a href="docs/ROADMAP.md"><b>Roadmap</b></a>
  &nbsp;&#183;&nbsp;
  <a href="docs/GETTING_STARTED.md"><b>Getting Started</b></a>
  &nbsp;&#183;&nbsp;
  <a href="docs/EXAMPLES.md"><b>Examples</b></a>
</h3>

<div align="center">
  <strong>Simple syntax, native performance, transparent Rust FFI.</strong>
  <br><br>
  
  [![Build Status](https://github.com/jonathanmagambo/otterlang/workflows/CI/badge.svg)](https://github.com/jonathanmagambo/otterlang/actions)
  [![Discord](https://img.shields.io/badge/Discord-Join%20Server-5865F2?style=flat&logo=discord&logoColor=white)](https://discord.gg/y3b4QuvyFk)
</div>

<h1 align="center">What is OtterLang?</h1>

OtterLang is an indentation-sensitive programming language with an LLVM backend that compiles to native machine code. It combines Python-like simplicity with Rust-like performance and seamless interoperability with the Rust ecosystem.

**Think of OtterLang as a modern language that bridges the gap between high-level expressiveness and low-level performance, with transparent access to the entire Rust ecosystem.**

<h1 align="center">Goals</h1>

- 🎯 **Simple syntax** – Indentation-driven, no braces or semicolons
- ⚡ **Native performance** – Compiles to native binaries with LLVM
- 🔗 **Transparent Rust FFI** – Import any Rust crate with `use rust:crate_name`
- 🛡️ **Memory safety** – Generational garbage collection with explicit root APIs
- 🧵 **Concurrency** – Built-in async task runtime
- 📦 **Rich standard library** – Comprehensive stdlib covering IO, networking, JSON, and more

<h1 align="center">Quick Start</h1>

```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang
nix develop
cargo build --release
./target/release/otter run hello.ot
```

See the [Getting Started Guide](docs/GETTING_STARTED.md) for detailed instructions.

<h1 align="center">Documentation</h1>

<div align="center">

**[Roadmap](docs/ROADMAP.md)** – project roadmap and future plans.

**[Getting Started](docs/GETTING_STARTED.md)** – installation, CLI walkthrough, first project.

**[Examples](docs/EXAMPLES.md)** – curated sample programs.

**[Tutorials](docs/TUTORIALS.md)** – guided walkthroughs for specific topics.

**[API Reference](docs/API_REFERENCE.md)** – stdlib module documentation.

</div>

<h1 align="center">Contributing and License</h1>

> [!IMPORTANT]
> OtterLang is currently in **Early Access (v0.1.0)** and is being actively developed. **If you plan to contribute, now is the time to provide a helping hand for the hardworking team**.

OtterLang is free and open source, released under the **BSD-3-Clause License**. Contributions are welcome! See [CONTRIBUTING.md](./CONTRIBUTING.md) for more information.

<h1 align="center">Contributors</h1>

<div align="center">

<a href="https://github.com/jonathanmagambo/otterlang/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=jonathanmagambo/otterlang" />
</a>

Thank you so much to all our contributors! ❤️

![Alt](https://repobeats.axiom.co/api/embed/311d7c69bcf9daf0ef09f03cb6e369994f71b519.svg "Repobeats analytics image")

</div>

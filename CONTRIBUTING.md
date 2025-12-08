# Contributing to OtterLang

Thank you for your interest in contributing to OtterLang! This guide will help you get started with development setup, workflow expectations, and contribution guidelines.

## Development Setup

### Getting the Source Code

First, clone the OtterLang repository:

```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang
```

### Installing Nix

If you plan to use Nix (recommended), you'll need to install it first. Nix is a package manager that provides a reproducible development environment.

#### macOS and Linux

The easiest way to install Nix is using the official installer:

```bash
# Run the Nix installer
sh <(curl -L https://nixos.org/nix/install) --daemon
```

After installation, restart your terminal or run:

```bash
. ~/.nix-profile/etc/profile.d/nix.sh
```

#### Windows

On Windows, you can use Nix through WSL2 (Windows Subsystem for Linux) or use NixOS in a virtual machine. Alternatively, you can follow the manual installation instructions below.

For more detailed installation instructions, visit the [official Nix installation guide](https://nixos.org/download.html).

#### Verifying Nix Installation

After installing Nix, verify it's working:

```bash
nix --version
```

You should see the Nix version number. If you encounter any issues, refer to the [Nix troubleshooting guide](https://nixos.org/manual/nix/stable/installation/installing-binary.html#troubleshooting).

### Option 1: Nix Development Environment (Recommended)

The easiest way to get started is using the Nix development environment:

```bash
# Enter the development environment
nix develop

# Build the compiler
cargo +nightly build --release
```

### Option 2: Manual Toolchain Setup

If you prefer to set up the toolchain manually, follow these steps:

#### Prerequisites

1. Install Rust nightly:
   ```bash
   rustup toolchain install nightly
   ```

2. Install LLVM 18 and set the `LLVM_SYS_181_PREFIX` environment variable

#### Platform-Specific Instructions

**macOS**

```bash
# Clone the repository (if you haven't already)
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Install LLVM 18
brew install llvm@18

# Set environment variables
export LLVM_SYS_181_PREFIX=$(brew --prefix llvm@18)
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX
export PATH="$LLVM_SYS_181_PREFIX/bin:$PATH"

# Install Rust nightly
rustup toolchain install nightly

# Build OtterLang
cargo +nightly build --release
```

**Ubuntu / Debian**

```bash
# Clone the repository (if you haven't already)
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Install LLVM 18
sudo apt-get update
sudo apt-get install -y llvm-18 llvm-18-dev clang-18

# Set environment variables
export LLVM_SYS_181_PREFIX=/usr/lib/llvm-18
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX

# Install Rust nightly
rustup toolchain install nightly

# Build OtterLang
cargo +nightly build --release
```

**Fedora/RHEL**

```bash
# Clone the repository (if you haven't already)
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Install LLVM 18
sudo dnf -y install llvm18 llvm18-devel clang18

# Set environment variables
export LLVM_SYS_181_PREFIX=/usr/lib64/llvm18
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX

# Install Rust nightly
rustup toolchain install nightly

# Build OtterLang
cargo +nightly build --release
```

**Windows**

Use the **x64 Native Tools Command Prompt for VS 2022** (or any Visual Studio Developer Command Prompt) so the MSVC tooling is available. From there, run the bundled setup script which mirrors the process described in `docs/GETTING_STARTED.md`:

The included PowerShell script `setup.ps1` prepares your environment to build OtterLang by:

1. Initializing the Visual Studio developer shell (sets the required environment variables).
2. Downloading LLVM 18.
3. Downloading libxml2.
4. Updating `PATH` and other env vars so the compiler can find LLVM and libxml2.

The script installs the dependencies inside a `contrib` folder. Because the environment tweaks are session-specific, run the script in every new shell before building.

```powershell
# Clone the repository (if you haven't already)
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Install Rust nightly
rustup toolchain install nightly

# Run setup script
./setup.ps1

# Build OtterLang
cargo +nightly build --release
```

## Building & Testing

Build the project:

```bash
cargo +nightly build --release
```

Run the test suite:

```bash
cargo test
```

## Workflow Expectations

### Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` (nightly) and resolve warnings
- Favor expressive names and small helpers over heavy commenting

### Commit Messages

Use concise prefixes following conventional commit format:

```
feat: add array indexing support
fix: resolve type inference bug
docs: update FFI documentation
refactor: simplify lexer tokenization
```

### Pull Requests

When submitting a pull request:

1. Keep PRs scoped to a single change
2. Add tests/docs alongside behavioral changes
3. Ensure CI is green before requesting review
4. Reference related issues when possible (e.g., `Fixes #123`)

## Areas for Contribution

We welcome contributions in the following areas:

- **Language Front-End**: Parser, type checker, diagnostics
- **Standard Library**: Modules and runtime utilities
- **FFI Bridge**: Type mapping, cache tooling, `bridge.yaml` configs
- **Tooling**: VS Code extension, LSP, formatter, REPL/TUI
- **Documentation**: Runnable examples (`examples/basic`, `examples/ffi`, `examples/benchmarks`)
- **Performance**: Profiling and error-message polish

### Custom FFI Bridges

Most crates work via automatic rustdoc extraction. For bespoke setups:

1. Create `ffi/<crate>/bridge.yaml`
2. Describe the dependency metadata and any overrides
3. Use `ffi/rand/bridge.yaml` as a reference

## Reporting Issues

When reporting issues, please include:

- Clear reproduction steps or a minimal failing example
- Expected vs actual behavior
- Environment details (OS, LLVM/clang version, Rust toolchain, `nix develop` vs manual)

## License

By contributing, you agree that your work will be released under the BSD-3-Clause License.

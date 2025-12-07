# Getting Started Guide

## Table of Contents

- [Getting the Source Code](#getting-the-source-code)
- [Installing Nix](#installing-nix)
- [Quick Start with Nix](#quick-start-with-nix-recommended)
- [Installation](#installation)
  - [Using Nix (Recommended)](#using-nix-recommended)
  - [Manual Installation](#manual-installation)
  - [After Building](#after-building)
    - [WebAssembly Targets](#webassembly-targets)
  - [Troubleshooting](#troubleshooting)
- [Command Line Interface](#command-line-interface)
  - [Core Commands](#core-commands)
  - [Advanced Commands](#advanced-commands)
  - [Project Management](#project-management)
  - [Global Options](#global-options)
  - [Environment Variables](#environment-variables)

This guide covers installing OtterLang on various platforms. The easiest way is using Nix, but manual installation is also supported.

## Getting the Source Code

First, clone the OtterLang repository:

```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang
```

Now you're ready to build OtterLang. Choose one of the installation methods below.

## Installing Nix

If you plan to use Nix (recommended), you'll need to install it first. Nix is a package manager that provides a reproducible development environment.

### macOS and Linux

The easiest way to install Nix is using the official installer:

```bash
# Run the Nix installer
sh <(curl -L https://nixos.org/nix/install) --daemon
```

After installation, restart your terminal or run:

```bash
. ~/.nix-profile/etc/profile.d/nix.sh
```

### Windows

On Windows, you can use Nix through WSL2 (Windows Subsystem for Linux) or use NixOS in a virtual machine. Alternatively, you can follow the manual installation instructions below.

For more detailed installation instructions, visit the [official Nix installation guide](https://nixos.org/download.html).

### Verifying Nix Installation

After installing Nix, verify it's working:

```bash
nix --version
```

You should see the Nix version number. If you encounter any issues, refer to the [Nix troubleshooting guide](https://nixos.org/manual/nix/stable/installation/installing-binary.html#troubleshooting).

## Quick Start with Nix (Recommended)

Nix provides a reproducible development environment with all dependencies:

```bash
# Enter the development environment
nix develop

# Build the compiler (nightly toolchain is set as default in Nix)
cargo build --release
```

The Nix flake automatically provides:
- Rust nightly toolchain
- LLVM 18 with WebAssembly support
- All required system dependencies

This is the recommended approach for development and ensures consistent builds across different systems.

## Manual Installation

If you prefer not to use Nix, you can install dependencies manually.

### Prerequisites

- **Rust nightly**: Required for FFI features
- **LLVM 18**: For code generation and WebAssembly support

### macOS

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

### Ubuntu/Debian

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

### Fedora/RHEL

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

### Windows

**Important:** You must use the **x64 Native Tools Command Prompt for VS 2022** (or Visual Studio Developer Command Prompt) to build. The MSVC linker requires environment variables that are only set in the Developer Command Prompt.

```powershell
# Clone the repository (if you haven't already)
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Method 1: Using winget (recommended)
winget install -e --id LLVM.LLVM -v "18.1.8" --silent --accept-package-agreements --accept-source-agreements
$env:LLVM_SYS_181_PREFIX = "C:\\Program Files\\LLVM"
$env:LLVM_SYS_180_PREFIX = $env:LLVM_SYS_181_PREFIX
$env:Path = "$env:LLVM_SYS_181_PREFIX\\bin;$env:Path"

# Install Rust nightly
rustup toolchain install nightly
rustup default nightly

# Build OtterLang
cargo +nightly build --release
```

**Note:** The LLVM installation path may vary depending on the installation method. A typical location is `C:\\Program Files\\LLVM`.

## After Building

Once the build completes successfully, the `otter` binary will be available at `target/release/otter`.

### Running Programs

```bash
# Run a program directly
./target/release/otter run examples/basic/hello.ot

# Or using cargo
cargo +nightly run --release --bin otter -- run examples/basic/hello.ot
```

### Building Executables

```bash
# Build a standalone executable
./target/release/otter build examples/basic/hello.ot -o hello
```

#### WebAssembly Targets

OtterLang can compile programs to WebAssembly (WASM) for running in web browsers or other WASM runtimes.

**Basic WebAssembly (no WASI):**
```bash
otter build program.ot --target wasm32-unknown-unknown -o program.wasm
```

**WebAssembly with WASI support:**
```bash
otter build program.ot --target wasm32-wasi -o program.wasm
```

**Target Differences:**

- **`wasm32-wasi`** - WebAssembly System Interface
  - Best for: Server-side WASM, command-line tools, WASI-compatible runtimes
  - Full WASI support: Direct access to stdio, filesystem, networking, and other system APIs
  - Runtime compatibility: Works in any WASI-compatible runtime (wasmtime, WasmEdge, etc.)

- **`wasm32-unknown-unknown`** - Bare WebAssembly
  - Best for: Web browsers, embedded systems, custom host environments
  - Minimal imports: Only requires a few host functions for I/O
  - Smaller binaries: No WASI runtime overhead
  - Browser compatible: Can run directly in web browsers with appropriate host

**Requirements:**
- LLVM 18 with WebAssembly target support
- `clang` and `wasm-ld` in your PATH (included with LLVM installations)

**Limitations:**
- Garbage Collection: WASM modules use OtterLang's built-in GC, which may have different performance characteristics than native execution
- FFI: Foreign function interface is limited in WASM environments
- File I/O: Direct filesystem access requires WASI or host-provided APIs
- Concurrency: Task spawning works but may have limitations in constrained environments
- Many stdlib modules (`io`, `net`, `task`) are unavailable in WASM targets

**Examples:**
```bash
# Build examples for WebAssembly
otter build examples/basic/hello.ot --target wasm32-unknown-unknown -o hello.wasm
otter build examples/basic/fibonacci.ot --target wasm32-wasi -o fibonacci.wasm

# Run with wasmtime (WASI)
wasmtime fibonacci.wasm
```

### Testing

```bash
# Run the test suite
cargo +nightly test --release

# Run specific tests
cargo +nightly test --release test_name
```

### Troubleshooting

**Nix builds with libffi errors:**
If you get "libffi.so.8" errors when running outside Nix, use the Nix environment:
```bash
nix develop
./target/release/otter run program.ot
```

**Windows path issues:**
Ensure you're using the Visual Studio Developer Command Prompt with proper MSVC environment variables.

## Command Line Interface

OtterLang provides a comprehensive CLI tool for running, building, and managing OtterLang programs.

### Usage

```bash
otter [COMMAND] [OPTIONS] [FILE]
```

### Core Commands

#### `run` - Execute a Program

Run an OtterLang program directly.

```bash
otter run program.ot [options]
```

**Options:**
- `--debug` - Enable debug mode with additional logging
- `--quiet` - Suppress non-error output
- `--lib-path <PATH>` - Add directory to library search path

**Examples:**
```bash
otter run hello.ot
otter run --debug myprogram.ot
```

#### `build` - Compile to Executable

Compile an OtterLang program to a native executable or WebAssembly.

```bash
otter build program.ot [options]
```

**Options:**
- `-o, --output <FILE>` - Output file path
- `--target <TARGET>` - Compilation target (`native`, `wasm32-unknown-unknown`, `wasm32-wasi`)
- `--release` - Enable release optimizations

**Examples:**
```bash
otter build hello.ot
otter build program.ot -o myapp
otter build app.ot --target wasm32-unknown-unknown -o app.wasm
```

#### `fmt` - Format Code

Format OtterLang source code according to standard style guidelines.

```bash
otter fmt [files...] [options]
```

**Options:**
- `--check` - Check if files are formatted correctly (don't modify)

**Examples:**
```bash
otter fmt source.ot
otter fmt --check *.ot
```

#### `repl` - Interactive REPL

Start an interactive Read-Eval-Print Loop for experimenting with OtterLang code.

```bash
otter repl [options]
```

### Advanced Commands

#### `test` - Run Tests

Execute unit tests and integration tests.

```bash
otter test [options] [pattern]
```

#### `profile` - Performance Profiling

Profile program execution for performance analysis.

```bash
otter profile <SUBCOMMAND> program.ot [options]
```

**Subcommands:** `memory`, `cpu`, `alloc`

#### `lsp` - Language Server

Start the OtterLang Language Server Protocol (LSP) server for editor integration.

```bash
otter lsp [options]
```

### Project Management

#### `new` - Create New Project

Create a new OtterLang project with standard directory structure.

```bash
otter new <NAME> [options]
```

#### `check` - Type Check

Check program for type errors without running it.

```bash
otter check program.ot [options]
```

### Global Options

These options can be used with any command:

- `-h, --help` - Show help information
- `-V, --version` - Show version information
- `--verbose` - Enable verbose output
- `--quiet` - Suppress informational output

### Environment Variables

- `OTTER_LOG` - Set logging level
- `OTTER_FFI_CACHE` - FFI bridge cache directory
- `OTTER_LIB_PATH` - Additional library search paths

**LLVM not found:**
Verify that `LLVM_SYS_181_PREFIX` points to the correct LLVM installation directory and that LLVM binaries are in your PATH.

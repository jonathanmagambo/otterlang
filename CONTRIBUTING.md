# Contributing to OtterLang

## Development Setup

### Option 1: Using Nix (Recommended)

```bash
nix develop
cargo build --release
```

### Option 2: Manual Setup

1. **Install prerequisites**:
   - Rust (via rustup) - nightly required for FFI features
   - LLVM 18

2. **Setup**:

   **macOS:**
   ```bash
   brew install llvm@18
   export LLVM_SYS_180_PREFIX=$(brew --prefix llvm@18)
   export PATH="$LLVM_SYS_180_PREFIX/bin:$PATH"
   rustup toolchain install nightly
   rustup default nightly
   cargo build --release
   ```

   **Ubuntu/Debian:**
   ```bash
   sudo apt-get install -y llvm-18 llvm-18-dev clang-18
   export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
   rustup toolchain install nightly
   rustup default nightly
   cargo build --release
   ```

   **Windows:**
   ```powershell
   # Install LLVM 18 using winget (recommended) or Chocolatey
   winget install --id LLVM.LLVM --version 18.1.0 --silent --accept-package-agreements --accept-source-agreements
   # Or using Chocolatey:
   # choco install llvm -y

   # Set environment variables (adjust path if LLVM is installed elsewhere)
   $env:LLVM_SYS_180_PREFIX = "C:\Program Files\LLVM"
   $env:Path = "$env:LLVM_SYS_180_PREFIX\bin;$env:Path"

   # Install Rust nightly
   rustup toolchain install nightly
   rustup default nightly

   # Build
   cargo build --release
   ```

### Building

```bash
cargo build --release
```

### Testing

```bash
cargo test
```

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` to check for issues
- Write clear, self-documenting code

## Commit Messages

Use clear, descriptive messages:

```
feat: Add support for array indexing
fix: Resolve type inference bug
docs: Update FFI documentation
refactor: Simplify lexer tokenization
```

## Pull Requests

1. Keep PRs focused: one feature or fix per PR
2. Add tests for new functionality
3. Update documentation as needed
4. Ensure CI passes before requesting review

## Areas for Contribution

- Language features (Pythonic style preferred)
- Standard library modules
- FFI bridges (transparent FFI auto-extracts from rustdoc, or use bridge.yaml for custom config)
- Documentation improvements
- Examples (organized in `examples/basic/`, `examples/ffi/`, `examples/benchmarks/`)
- Performance optimizations
- Error message improvements

## FFI Development

Transparent FFI automatically extracts APIs from Rust crates via rustdoc. No `bridge.yaml` needed unless you want custom configuration.

To add custom bridge configuration:
1. Create `ffi/<crate-name>/bridge.yaml`
2. See `ffi/rand/bridge.yaml` for examples

## Reporting Issues

Include:
- Clear description and reproduction steps
- Expected vs actual behavior
- Environment: OS, LLVM version, Rust version

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

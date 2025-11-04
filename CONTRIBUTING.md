# Contributing to OtterLang

Thank you for your interest in contributing to OtterLang! This document provides guidelines and information for contributors.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/your-username/otterlang.git
   cd otterlang
   ```
3. **Create a branch** for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   ```

## Development Setup

1. **Install prerequisites**:
   - Rust (latest stable version)
   - LLVM 15 (see [README.md](README.md) for installation instructions)

2. **Build the project**:
   ```bash
   cargo build --release
   ```

3. **Run tests**:
   ```bash
   cargo test
   ```

4. **Install locally**:
   ```bash
   ./setup.sh
   ```

## Making Changes

### Code Style

- Follow Rust's standard formatting conventions (`cargo fmt`)
- Run `cargo clippy` to check for common issues
- Write clear, self-documenting code
- Add comments for complex logic

### Commit Messages

Use clear, descriptive commit messages:

```
feat: Add support for array indexing
fix: Resolve type inference bug in nested functions
docs: Update FFI documentation
refactor: Simplify lexer tokenization logic
```

### Pull Requests

1. **Keep PRs focused**: One feature or fix per PR
2. **Write tests**: Add tests for new features or bug fixes
3. **Update documentation**: Update relevant docs (README, API docs, etc.)
4. **Check CI**: Ensure all tests pass before requesting review

### Testing

- Add unit tests for new functionality
- Add integration tests for language features
- Run existing tests to ensure nothing breaks:
  ```bash
  cargo test
  ```

## Areas for Contribution

- **Language features**: Syntax improvements, new constructs (Pythonic style preferred)
- **Standard library**: Additional modules and functions
- **FFI bridges**: Create bridge.yaml files for Rust crates
- **Documentation**: Improve docs, add tutorials
- **Examples**: Add example programs (organized in `examples/basic/`, `examples/ffi/`, `examples/benchmarks/`)
- **Performance**: Optimize compilation or runtime
- **Error messages**: Improve error reporting

## FFI Bridge Development

To add support for a new Rust crate:

1. Create `ffi/<crate-name>/bridge.yaml`
2. Define function signatures and call expressions
3. See existing bridges in `ffi/` for examples
4. Test your bridge with example code

## Reporting Issues

Use GitHub Issues to report bugs or request features. Include:

- **Description**: Clear description of the issue
- **Reproduction**: Steps to reproduce
- **Expected behavior**: What should happen
- **Actual behavior**: What actually happens
- **Environment**: OS, LLVM version, Rust version

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

## Questions?

Feel free to open a GitHub Discussion or issue if you have questions about contributing.


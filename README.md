# OtterLang 🦦

<p>
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://github.com/jonathanmagambo/otterlang/blob/main/image.png?raw=true" width="400">
      <img src="https://github.com/jonathanmagambo/otterlang/blob/main/image.png?raw=true" width="400" alt="OtterLang Logo" />
    </picture>
    <br>
    <strong>Simple syntax, native performance, transparent Rust FFI.</strong>
</p>

[![Build Status](https://github.com/jonathanmagambo/otterlang/workflows/CI/badge.svg)](https://github.com/jonathanmagambo/otterlang/actions)
[![Discord](https://img.shields.io/badge/Discord-Join%20Server-5865F2?style=flat&logo=discord&logoColor=white)](https://discord.gg/y3b4QuvyFk)

An indentation-sensitive programming language with an LLVM backend. OtterLang compiles to native binaries with a focus on simplicity and performance.

## Quick Start

```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang

# Using Nix (recommended)
nix develop
cargo build --release

# Create and run your first program
cat > hello.ot << 'EOF'
use otter:fmt

fn main():
    fmt.println("Hello from OtterLang!")
EOF

otter run hello.ot
```

## Installation

### Using Nix (Recommended)

```bash
nix develop
cargo build --release
```

The Nix flake automatically provides Rust nightly, LLVM 18, and all dependencies.

### Manual Setup

**Prerequisites:**
- Rust (via rustup) - nightly required for FFI features
- LLVM 18

**macOS:**
```bash
brew install llvm@18
export LLVM_SYS_181_PREFIX=$(brew --prefix llvm@18)
export PATH="$LLVM_SYS_181_PREFIX/bin:$PATH"
rustup toolchain install nightly
rustup default nightly
cargo build --release
```

**Ubuntu/Debian:**
```bash
sudo apt-get install -y llvm-18 llvm-18-dev clang-18
export LLVM_SYS_181_PREFIX=/usr/lib/llvm-18
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
$env:LLVM_SYS_181_PREFIX = "C:\Program Files\LLVM"
$env:Path = "$env:LLVM_SYS_181_PREFIX\bin;$env:Path"

# Install Rust nightly
rustup toolchain install nightly
rustup default nightly

# Build
cargo build --release
```

**Note for Windows:** If LLVM is installed in a different location, update `LLVM_SYS_181_PREFIX` accordingly. Common locations:
- `C:\Program Files\LLVM`
- `C:\Program Files (x86)\LLVM`

## Language Features

### Syntax

Clean indentation-based syntax with modern features:

```otter
use otter:fmt
use otter:math

fn greet(name: str) -> str:
    return "Hello, " + name + "!"

struct Point:
    x: float
    y: float

    fn distance(self) -> float:
        return math.sqrt(self.x * self.x + self.y * self.y)

fn main():
    let message = greet("World")
    fmt.println(message)

    let p = Point(x=3.0, y=4.0)
    let dist = p.distance()
    fmt.println("Point: (" + stringify(p.x) + ", " + stringify(p.y) + "), distance: " + stringify(dist))

    if len(message) > 10:
        fmt.println("Long message")

    for i in 0..10:
        fmt.println(stringify(i))
```

### Transparent Rust FFI

Automatically use any Rust crate without manual configuration:

```otter
use rust:rand
use otter:fmt

fn main():
    let random = rand.random_f64()
    fmt.println("Random: " + stringify(random))
```

**Key advantages:**
- ✅ No manual bindings needed
- ✅ Automatic API extraction via rustdoc (requires Rust nightly)
- ✅ Memory management handled automatically
- ✅ Async/await support for Rust Futures
- ✅ Type checking integrated

See [docs/FFI_TRANSPARENT.md](docs/FFI_TRANSPARENT.md) for details.

### Standard Library

Built-in modules:
- `otter:math` - Mathematical functions
- `otter:io` - File I/O
- `otter:time` - Time utilities
- `otter:task` - Task-based concurrency
- `otter:rand` - Random numbers
- `otter:json` - JSON parsing
- `otter:net` - Networking
- `otter:http` - HTTP client/server

### Exception Handling

Modern exception handling with zero-cost success path:

```otter
use otter:fmt

fn divide(x: int, y: int) -> int:
    if y == 0:
        raise "Division by zero"
    return x / y

fn safe_operation():
    try:
        let result = divide(10, 0)
        fmt.println("Result: " + stringify(result))
    except Error as e:
        fmt.println("Caught error: " + stringify(e))
    else:
        fmt.println("No errors occurred")
    finally:
        fmt.println("Cleanup always runs")

fn nested_exceptions():
    try:
        try:
            raise "Inner error"
        except Error:
            fmt.println("Handled inner error")
            raise "Outer error"
    except Error:
        fmt.println("Handled outer error")
```

**Features:**
- `try/except/else/finally` blocks
- Exception propagation with automatic cleanup
- Zero-cost abstractions (no overhead on success path)
- Type-safe error handling at compile time

## Performance

**Note:** Benchmarks are currently being retested and properly specified. Comprehensive performance metrics will be available in a future update. OtterLang compiles to native code via LLVM and is designed for high performance, with automatic memory management and zero-cost abstractions.

## CLI Commands

```bash
otterlang run program.ot          # Run program
otterlang build program.ot -o out # Build executable
otterlang fmt                      # Format code
otterlang repl                     # Start REPL
otterlang profile memory program.ot # Profile memory
```

## Examples

**Basic Programs:**
- `examples/basic/hello.ot` - Hello world
- `examples/basic/exception_basics.ot` - Exception handling basics
- `examples/basic/exception_advanced.ot` - Advanced exceptions
- `examples/basic/exception_resource.ot` - Resource management
- `examples/basic/exception_validation.ot` - Data validation
- `examples/basic/struct_methods_demo.ot` - Struct methods
- `examples/basic/struct_demo.ot` - Struct usage
- `examples/basic/advanced_pipeline.ot` - Complex computation
- `examples/basic/task_benchmark.ot` - Task benchmarks
- `examples/basic/fibonacci.ot` - Fibonacci sequence
- `examples/basic/pythonic_demo.ot` - Pythonic style
- `examples/basic/multiline_test.ot` - Multi-line strings

**FFI Examples:**
- `examples/ffi/ffi_rand_demo.ot` - Random number generation
- `examples/ffi/ffi_rand_advanced.ot` - Advanced FFI usage

### Unit Testing (Examples)

Below are illustrative examples of how unit tests will look in OtterLang. A built-in `otter test` runner is planned (see `roadmap.md`).

```otter
use otter:fmt

struct User:
    id: int
    name: str

fn make_users() -> list<User>:
    return [User(id=1, name="Ana"), User(id=2, name="Bo")] 

fn to_map(users: list<User>) -> dict<int, str>:
    let m = { }
    for u in users:
        m[u.id] = u.name
    return m

fn test_user_list_basic():
    let xs = make_users()
    assert len(xs) == 2
    assert xs[0].name == "Ana"

fn test_dict_building():
    let m = to_map(make_users())
    assert m[1] == "Ana"
    assert m.get(3, default="none") == "none"

fn test_nested_structs_and_lists():
    struct Team:
        name: str
        members: list<User>

    let team = Team(name="core", members=make_users())
    assert team.members[1].id == 2
```

The test runner will discover functions prefixed with `test_` and report pass/fail results with spans and diffs.

## Status

**Early Access (v0.1.0)** - Experimental, not production-ready.

### Known Limitations

- Type inference is limited (explicit types recommended)
- Module system has some limitations
- Requires LLVM 18 and Rust nightly (for FFI features)

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT License - see [LICENSE](LICENSE).

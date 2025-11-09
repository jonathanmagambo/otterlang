# OtterLang

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
cargo +nightly build --release

# Create and run your first program
cat > hello.ot << 'EOF'
def main():
    print("Hello from OtterLang!")
EOF

otter run hello.ot
```

## Installation

### Using Nix (Recommended)

```bash
nix develop
cargo +nightly build --release
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
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX
export PATH="$LLVM_SYS_181_PREFIX/bin:$PATH"
rustup toolchain install nightly
cargo +nightly build --release
```

**Ubuntu/Debian:**
```bash
sudo apt-get install -y llvm-18 llvm-18-dev clang-18
export LLVM_SYS_181_PREFIX=/usr/lib/llvm-18
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX
rustup toolchain install nightly
cargo +nightly build --release
```

**Fedora 43:**
```bash
sudo dnf -y install llvm18 llvm18-devel clang18
export LLVM_SYS_181_PREFIX=/usr/lib64/llvm18
export LLVM_SYS_180_PREFIX=$LLVM_SYS_181_PREFIX
rustup toolchain install nightly
cargo +nightly build --release
```

**Windows:**
```powershell
# Install LLVM 18.1 using llvmenv (recommended)
cargo install llvmenv --locked
llvmenv install 18.1
llvmenv global 18.1

# Set environment variables
$llvmPath = llvmenv prefix
$env:LLVM_SYS_181_PREFIX = $llvmPath
$env:LLVM_SYS_180_PREFIX = $llvmPath
$env:Path = "$llvmPath\bin;$env:Path"

# Alternative: Install using winget or Chocolatey
# winget install --id LLVM.LLVM --silent --accept-package-agreements --accept-source-agreements
# choco install llvm -y
# $env:LLVM_SYS_181_PREFIX = "C:\Program Files\LLVM"
# $env:LLVM_SYS_180_PREFIX = $env:LLVM_SYS_181_PREFIX
# $env:Path = "$env:LLVM_SYS_181_PREFIX\bin;$env:Path"

# Install Rust nightly
rustup toolchain install nightly
rustup default nightly

# Build
cargo +nightly build --release
```

**Note for Windows:** If using winget/Chocolatey, LLVM may be installed in `C:\Program Files\LLVM` or `C:\Program Files (x86)\LLVM`.

**Important:** On Windows, you must use the **x64 Native Tools Command Prompt for VS 2022** to build. The MSVC linker requires environment variables that are automatically set in the Developer Command Prompt. Open it from the Start menu, then navigate to your project directory and run the build commands. Regular PowerShell/CMD will not have the MSVC environment configured.

## After Building

Once the build completes successfully, you can:

**Run a program:**
```bash
cargo +nightly run --release --bin otterlang -- run examples/basic/hello.ot
```

**Build an executable:**
```bash
cargo +nightly run --release --bin otterlang -- build examples/basic/hello.ot -o hello
```

**Run tests:**
```bash
cargo +nightly test --release
```

**Use the compiler directly:**
```bash
# The binary is located at:
# target/release/otterlang (or target/release/otterlang.exe on Windows)
./target/release/otterlang run program.ot
# Or on Windows:
# target\release\otterlang.exe run program.ot
```

## Language Features

### Syntax

Clean indentation-based syntax with modern features:

```otter
use math

def greet(name: str) -> str:
    return f"Hello, {name}!"

class Point:
    x: float
    y: float

    def distance(self) -> float:
        return math.sqrt(self.x * self.x + self.y * self.y)

def main():
    let message = greet("World")
    print(message)

    let p = Point(x=3.0, y=4.0)
    let dist = p.distance()
    print(f"Point: ({p.x}, {p.y}), distance: {dist}")

    if len(message) > 10:
        print("Long message")

    for i in 0..10:
        print(f"{i}")
```

`str()` is now a builtin for general conversions (the old `stringify()` helper remains as a compatibility alias).

### Enums

Use `enum` to define tagged unions with typed payloads. Variants accept tuple-style data, and you can destructure them with the existing `match` expression:

```otter
pub enum Option<T>:
    Some: (T)
    None

use core

def divide(x: float, y: float) -> Result<float, str>:
    if y == 0:
        return Result.Err("division by zero")
    return Result.Ok(x / y)

def main():
    let outcome = divide(10.0, 2.0)
    let value = match outcome:
        case Result.Ok(val):
            val
        case Result.Err(msg):
            print(f"error: {msg}")
            0.0

    let wrapped = Option.Some(value)
    match wrapped:
        case Option.Some(v):
            print(f"value: {v}")
        case Option.None:
            print("no value")
```

The standard library now ships `Option` and `Result` as regular enums in `core`, so you can `use core` (or explicitly `use otter:core`) to pull them in instead of relying on hard-coded compiler types.

### Transparent Rust FFI

Automatically use any Rust crate without manual configuration:

```otter
use rust:rand

def main():
    let random = rand.random_f64()
    print(f"Random: {random}")
```

**Key advantages:**
- No manual bindings needed
- Automatic API extraction via rustdoc (requires Rust nightly)
- Memory management handled automatically
- Async/await support for Rust Futures
- Type checking integrated

See [docs/FFI_TRANSPARENT.md](docs/FFI_TRANSPARENT.md) for details.

### Standard Library

Built-in modules (import without the `otter:` prefix):
- `fmt` - Formatting and printing helpers
- `math` - Mathematical functions
- `io` - File I/O
- `time` - Time utilities
- `task` - Task-based concurrency
- `rand` - Random numbers
- `json` - JSON parsing
- `net` - Networking
- `http` - HTTP client/server

You can import multiple modules at once using commas, and the old `otter:` prefix remains available when you want to be explicit:

```otter
use fmt, math, time
use rust:rand
use otter:json  # still supported
```

### Exception Handling

Modern exception handling with zero-cost success path:

```otter
def divide(x: int, y: int) -> int:
    if y == 0:
        raise "Division by zero"
    return x / y

def safe_operation():
    try:
        let result = divide(10, 0)
        print("Result: " + str(result))
    except Error as e:
        print("Caught error: " + str(e))
    else:
        print("No errors occurred")
    finally:
        print("Cleanup always runs")

def nested_exceptions():
    try:
        try:
            raise "Inner error"
        except Error:
            print("Handled inner error")
            raise "Outer error"
    except Error:
        print("Handled outer error")
```

**Features:**
- `try/except/else/finally` blocks
- Exception propagation with automatic cleanup
- Zero-cost abstractions (no overhead on success path)
- Type-safe error handling at compile time

## CLI Commands

```bash
otterlang run program.ot          # Run program
otterlang build program.ot -o out # Build executable
otterlang build program.ot --target wasm32-unknown-unknown -o out.wasm # Build to WebAssembly
otterlang fmt                      # Format code
otterlang repl                     # Start REPL
otterlang profile memory program.ot # Profile memory
```

### WebAssembly Support

OtterLang can compile to WebAssembly! Use the `--target` flag:

```bash
# Compile to WebAssembly (wasm32-unknown-unknown)
otterlang build program.ot --target wasm32-unknown-unknown -o program.wasm

# Compile to WebAssembly System Interface (wasm32-wasi)
otterlang build program.ot --target wasm32-wasi -o program.wasm
```

**Requirements:**
- LLVM 18 with WebAssembly target support
- `clang` and `wasm-ld` in your PATH (usually included with LLVM)

When targeting `wasm32-wasi` the generated binary can talk directly to WASI's
stdio and wall-clock APIs. For the more barebones `wasm32-unknown-unknown`
target we import a minimal host surface so you can decide how to surface
output:

- `env.otter_write_stdout(ptr: i32, len: i32)` – write UTF-8 data to stdout
- `env.otter_write_stderr(ptr: i32, len: i32)` – write UTF-8 data to stderr
- `env.otter_time_now_ms() -> i64` – optional wall-clock timestamp in ms

A tiny JavaScript host that wires these up under Node.js looks like:

```js
import fs from 'node:fs';

const memory = new WebAssembly.Memory({ initial: 8 });
const decoder = new TextDecoder();

const env = {
  memory,
  otter_write_stdout(ptr, len) {
    const bytes = new Uint8Array(memory.buffer, ptr, len);
    process.stdout.write(decoder.decode(bytes));
  },
  otter_write_stderr(ptr, len) {
    const bytes = new Uint8Array(memory.buffer, ptr, len);
    process.stderr.write(decoder.decode(bytes));
  },
  otter_time_now_ms() {
    return BigInt(Date.now());
  },
};

const { instance } = await WebAssembly.instantiate(fs.readFileSync('program.wasm'), { env });
instance.exports.main?.();
```

The generated `.wasm` file can be run in any WebAssembly runtime (Node.js, browsers, wasmtime, etc.).

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
class User:
    id: int
    name: str

def make_users() -> list<User>:
    return [User(id=1, name="Ana"), User(id=2, name="Bo")] 

def to_map(users: list<User>) -> dict<int, str>:
    let m = { }
    for u in users:
        m[u.id] = u.name
    return m

def test_user_list_basic():
    let xs = make_users()
    assert len(xs) == 2
    assert xs[0].name == "Ana"

def test_dict_building():
    let m = to_map(make_users())
    assert m[1] == "Ana"
    assert m.get(3, default="none") == "none"

def test_nested_structs_and_lists():
    class Team:
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

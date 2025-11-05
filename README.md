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

## Why OtterLang?

| Feature | OtterLang | Nim |
|---------|-----------|-----|
| **Transparent Rust FFI** | ✅ Auto-extracts entire public API | ❌ Manual bindings required |
| **Indentation-based Syntax** | ✅ Clean and readable | ✅ Similar |
| **LLVM Backend** | ✅ Native code generation | ✅ Also uses LLVM |
| **Memory Management** | ✅ Automatic GC + profiling | ✅ GC available |
| **Zero-cost Abstractions** | ✅ Direct LLVM compilation | ✅ Good |
| **Type Inference** | ✅ Full inference | ✅ Advanced |
| **Async/Await** | ✅ Built-in task system | ✅ Async/await |
| **Cross-compilation** | ✅ WASM, embedded targets | ✅ Good support |
| **REPL** | ✅ Interactive development | ❌ Limited |
| **Code Formatting** | ✅ Built-in formatter | ✅ Built-in |
| **Memory Profiling** | ✅ Built-in profiler | ❌ External tools |
| **Rust Ecosystem Access** | ✅ Transparent bridging | ❌ Manual bindings |
| **Compilation Speed** | ✅ Fast (LLVM) | ✅ Fast |
| **Runtime Performance** | ✅ Near C speed (1.28x) | ✅ Near C speed |

## Quick Start

```bash
# Clone and setup
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang
./setup.sh

# Create your first program
cat > hello.otter << 'EOF'
use otter:fmt

fn main():
    fmt.println("Hello from OtterLang!")
EOF

# Run it
otter run hello.otter
```

## Installation

### Prerequisites

Requires **LLVM 15**.

**macOS:**
```bash
brew install llvm@15
export LLVM_SYS_150_PREFIX=$(brew --prefix llvm@15)
export PATH="$LLVM_SYS_150_PREFIX/bin:$PATH"
```

**Ubuntu/Debian:**
```bash
sudo apt-get install -y llvm-15 llvm-15-dev clang-15
export LLVM_SYS_150_PREFIX=/usr/lib/llvm-15
```

**Manual Build:**
```bash
git clone https://github.com/jonathanmagambo/otterlang.git
cd otterlang
cargo build --release
cargo install --path . --bin otter
```

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

    # Struct initialization and methods
    let p = Point(x=3.0, y=4.0)
    let dist = p.distance()
    fmt.println("Point: (" + stringify(p.x) + ", " + stringify(p.y) + "), distance: " + stringify(dist))

    # Control flow
    if len(message) > 10:
        fmt.println("Long message")

    # Loops
    for i in 0..10:
        fmt.println(stringify(i))
```

### Transparent Rust FFI

Automatically use any Rust crate without manual configuration:

```otter
use rust:rand
use otter:fmt

fn main():
    # Auto-extracted from rustdoc JSON
    let random = rand.random_f64()
    fmt.println("Random: " + stringify(random))
```

**Key advantages:**
- ✅ No manual bindings needed
- ✅ Automatic API extraction via rustdoc
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

Benchmarked against C and Rust (100M iterations):

| Language | Time | Relative to C |
|----------|------|---------------|
| C | 0.070s | 1.00x |
| Rust | 0.080s | 1.14x |
| **OtterLang** | **0.090s** | **1.28x** |

Run `examples/benchmarks/benchmark.sh` to test yourself.

## CLI Commands

```bash
otterlang run program.otter          # Run program
otterlang build program.otter -o out # Build executable
otterlang fmt                        # Format code
otterlang repl                       # Start REPL
otterlang profile memory program.otter # Profile memory
```

## Examples

**Basic Programs:**
- `examples/basic/hello.otter` - Simple hello world example
- `examples/basic/exception_basics.otter` - Basic exception handling
- `examples/basic/exception_advanced.otter` - Advanced try/except/else/finally
- `examples/basic/exception_resource.otter` - Resource management patterns
- `examples/basic/exception_validation.otter` - Data validation with exceptions
- `examples/basic/struct_methods_demo.otter` - Factorial computation
- `examples/basic/struct_demo.otter` - Distance calculation
- `examples/basic/advanced_pipeline.otter` - Complex computation pipeline
- `examples/basic/task_benchmark.otter` - Series summation
- `examples/basic/arithmetic.otter` - Basic arithmetic
- `examples/basic/fibonacci.otter` - Fibonacci sequence
- `examples/basic/pythonic_demo.otter` - Power function

**FFI Examples:**
- `examples/ffi/ffi_rand_demo.otter` - Algorithm examples
- `examples/ffi/ffi_rand_advanced.otter` - Complex algorithms

**Benchmarks:**
- `examples/benchmarks/pi_leibniz.otter` - Performance comparison
- `examples/benchmarks/benchmark.sh` - Run benchmarks

## Status

**Early Access (v0.1.0)** - Experimental, not production-ready.

### Known Limitations

- Type inference is limited (explicit types recommended)
- Module system has some limitations
- Windows support is experimental
- Requires LLVM 15 specifically

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT License - see [LICENSE](LICENSE).

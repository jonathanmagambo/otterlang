# Examples

This directory contains runnable samples that exercise the language surface area and standard library. Use them as quick references for syntax, runtime APIs, and integration patterns.

## Basic Language Features

### Core Syntax and Types

**`examples/basic/hello.ot`**
- **Purpose**: Basic "Hello, World!" program
- **Demonstrates**: Top-level `main`, printing, string literals
- **Run**: `otter run examples/basic/hello.ot`

**`examples/basic/enum_demo.ot`**
- **Purpose**: Enum creation and pattern matching
- **Demonstrates**: Tagged unions, exhaustive `match` expressions, control flow
- **Run**: `otter run examples/basic/enum_demo.ot`

**`examples/advanced_syntax_test.ot`**
- **Purpose**: Covers most grammar constructs in one file
- **Demonstrates**: Advanced syntax that powers parser regression tests
- **Run**: `otter run examples/advanced_syntax_test.ot`

### Data Structures

**`examples/basic/struct_demo.ot`**
- **Purpose**: Basic struct usage and manipulation
- **Demonstrates**: Struct definition, instantiation, field access
- **Run**: `otter run examples/basic/struct_demo.ot`

**`examples/basic/struct_methods_demo.ot`**
- **Purpose**: Struct methods and object-oriented patterns
- **Demonstrates**: Method definitions, `self` handling, encapsulation
- **Run**: `otter run examples/basic/struct_methods_demo.ot`

**`examples/basic/generic_struct_test.ot`**
- **Purpose**: Generic struct definitions
- **Demonstrates**: Type parameters, specialized instantiations, generic helpers
- **Run**: `otter run examples/basic/generic_struct_test.ot`

### Control Flow and Algorithms

**`examples/basic/fibonacci.ot`**
- **Purpose**: Classic recursive algorithm implementation
- **Demonstrates**: Recursion, branching, numeric operations
- **Run**: `otter run examples/basic/fibonacci.ot`

**`examples/basic/advanced_pipeline.ot`**
- **Purpose**: Complex data processing pipeline
- **Demonstrates**: Higher-order functions, composition, iterables
- **Run**: `otter run examples/basic/advanced_pipeline.ot`

**`examples/basic/http_request.ot`**
- **Purpose**: Fetching remote data
- **Demonstrates**: Stdlib HTTP client, response parsing, error surfaces
- **Run**: `otter run examples/basic/http_request.ot`

### Error Handling

**`examples/basic/error_handling_basics.ot`**
- **Purpose**: Fundamental `Result`-based error handling
- **Demonstrates**: Returning `Result`, pattern matching, recovery
- **Run**: `otter run examples/basic/error_handling_basics.ot`

**`examples/basic/error_handling_advanced.ot`**
- **Purpose**: Composing richer error scenarios
- **Demonstrates**: Nested results, propagation helpers, contextual errors
- **Run**: `otter run examples/basic/error_handling_advanced.ot`

**`examples/basic/error_handling_resource.ot`**
- **Purpose**: Resource management with fallible paths
- **Demonstrates**: Cleanup patterns, RAII-style scopes, deterministic teardown
- **Run**: `otter run examples/basic/error_handling_resource.ot`

**`examples/basic/error_handling_validation.ot`**
- **Purpose**: Input validation pipelines
- **Demonstrates**: Defensive programming, user-friendly diagnostics
- **Run**: `otter run examples/basic/error_handling_validation.ot`

### Data Exchange and Concurrency

**`examples/basic/yaml_demo.ot`**
- **Purpose**: Working with YAML data
- **Demonstrates**: Parsing documents using the stdlib `yaml` module
- **Run**: `otter run examples/basic/yaml_demo.ot`

**`examples/basic/task_benchmark.ot`**
- **Purpose**: Concurrent task performance benchmarking
- **Demonstrates**: Task spawning, scheduling primitives, throughput measurement
- **Run**: `otter run examples/basic/task_benchmark.ot`

## Foreign Function Interface (FFI)

**`examples/ffi/test_ffi_struct.ot`**
- **Purpose**: Showcases bridging structured data over FFI
- **Demonstrates**: Declaring FFI bindings, mapping structs and methods, interoperability with Rust types
- **Run**: `otter run examples/ffi/test_ffi_struct.ot`

## Running Examples

All examples can be run using the Otter CLI:

```bash
# Run directly
otter run examples/basic/hello.ot

# Build to executable first
otter build examples/basic/fibonacci.ot -o fibonacci
./fibonacci
```

## Learning Path

For new users, we recommend exploring samples in this order:

1. **Start here**: `hello.ot` → `enum_demo.ot` → `struct_demo.ot`
2. **Control flow**: `fibonacci.ot` → `advanced_pipeline.ot`
3. **Error handling**: `error_handling_basics.ot` → `error_handling_advanced.ot`
4. **Interop**: `http_request.ot` → `test_ffi_struct.ot`

Each example includes comments explaining the concepts being demonstrated. For more comprehensive tutorials, see [TUTORIALS.md](TUTORIALS.md).

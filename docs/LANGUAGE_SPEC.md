# OtterLang Language Specification

This document describes the syntax and semantics implemented by the OtterLang compiler, runtime, LSP server, and standard library that live in this repository. All examples and descriptions below have been checked against the parser, type checker, and runtime in `main` so they match the behavior you get when building this tree.

## Table of Contents

1. [Lexical Structure](#lexical-structure)
2. [Type System](#type-system)
3. [Expressions](#expressions)
4. [Statements](#statements)
5. [Functions and Methods](#functions-and-methods)
6. [Structs](#structs)
7. [Enums](#enums)
8. [Pattern Matching](#pattern-matching)
9. [Modules and Visibility](#modules-and-visibility)
10. [Concurrency Primitives](#concurrency-primitives)
11. [Error Handling](#error-handling)
12. [Standard Library Overview](#standard-library-overview)
13. [Grammar Summary](#grammar-summary)
14. [Semantics and Implementation Notes](#semantics-and-implementation-notes)

## Lexical Structure

### Comments

OtterLang uses `#` for single-line comments. Multi-line comments are created by using multiple `#` lines.

```otter
# This is a comment
# Multi-line comments use multiple hash lines
```

### Whitespace and Indentation

OtterLang is indentation-sensitive. Statements are grouped using indentation levels:

- A colon (`:`) introduces a new indentation block
- Block headers include: `fn`, `if`, `elif`, `else`, `for`, `while`, `match`, `struct`, `enum`
- All indentation must use spaces (tabs are not allowed)
- The standard indentation is 4 spaces

### Identifiers

Identifiers start with a letter or underscore and may contain ASCII letters, digits, or underscores. Unicode identifiers are also accepted. The standalone underscore (`_`) is treated as the wildcard identifier in patterns.

### Keywords

The following words are reserved keywords and cannot be used as identifiers:

**Control flow:** `if`, `elif`, `else`, `for`, `while`, `break`, `continue`, `pass`, `return`, `match`, `case`

**Declarations:** `fn`, `let`, `struct`, `enum`, `pub`, `use`, `as`

**Concurrency:** `await`, `spawn`

**Operators:** `and`, `or`, `not`, `in`, `is`

**Literals/Builtins:** `true`, `false`, `None`, `print`

**Contextual keywords:**
- `type` — recognized only at the start of type alias declarations; elsewhere it is treated as an identifier

### Literals

- **Numbers** support underscores for readability and may be written as integers (`42`, `1_000`) or floating-point values (`3.14`, `2.0e-3`).
- **Strings** use single or double quotes. Prefix a string with `f` to enable interpolation with `{expr}` placeholders.
- **Booleans** are `true` and `false`.
- **None/Unit** literals are written as `None`/`none` or as the empty tuple `()`.

## Type System

OtterLang uses a static type system with inference. Type annotations are optional but recommended for public APIs.

### Built-in Types

| Type | Description |
|------|-------------|
| `int` / `i32` | 32-bit signed integer |
| `i64` | 64-bit signed integer |
| `float` / `f64` / `number` | 64-bit floating point |
| `bool` | Boolean value |
| `str` / `string` | UTF-8 string |
| `unit` / `None` / `()` | Unit type (absence of value) |
| `list<T>` | Dynamic array of type T |
| `dict<K, V>` | Dictionary mapping keys of type K to values of type V |

Any other identifier is treated as a custom type or a type alias (e.g., `User`, `Channel<string>`). Type annotations currently consist of a simple identifier with optional generic arguments—there is no separate syntax for tuple or function types yet.

### Type Annotations

Annotate bindings, parameters, and return types with a colon:

```otter
let name: string = "Otter"
let values: list<int> = [1, 2, 3]
fn len_text(text: string) -> int:
    return len(text)
```

### Generics

Structs, enums, and `type` aliases support generic parameters:

```otter
struct Pair<T, U>:
    first: T
    second: U

pub type Response<T> = Result<T, Error>

pub enum Option<T>:
    Some: (T)
    None
```

Functions do not currently accept `<T>` parameter lists; shared generic behavior is modeled through parameterized data structures and type aliases instead.

### Type Aliases

Define aliases with the contextual `type` keyword:

```otter
pub type UserId = int
pub type Response<T> = Result<T, Error>
```

## Expressions

### Arithmetic and Comparison

OtterLang supports `+`, `-`, `*`, `/`, and `%`. The `+` operator also performs string concatenation, automatically converting integers, floats, and booleans to strings. Comparison operators include `==`, `!=`, `<`, `>`, `<=`, `>=`, `is`, and `is not`.

```otter
let normalized = (value - min) / (max - min)
if count is not None and count > 0:
    print("ready")
```

### Logical Operators

Use `and`, `or`, and `not` for boolean logic.

```otter
if is_ready and not has_failed:
    proceed()
```

### Function and Method Calls

Call syntax uses parentheses. Methods are regular functions stored inside structs, so you call them with the dot operator: `point.distance()`.

### Member Access and Namespaces

Use `object.field` or `Module.symbol`. Enum variants use the same syntax: `Option.Some(value)`.

### Struct Instantiation

Structs use keyword-style arguments:

```otter
let origin = Point(x=0.0, y=0.0)
```

### Collection Literals

```otter
let numbers = [1, 2, 3]
let mapping = {"a": 1, "b": 2}
```

### Comprehensions

Lists and dictionaries support comprehension syntax with an optional `if` filter:

```otter
let squares = [x * x for x in 0..10]
let indexed = {x: idx for idx in 0..len(items) if items[idx] != None}
```

Both forms currently expect the iterable expression to evaluate to a list. Use the `range(start, end)` helper (or a pre-built list) when feeding numeric ranges into comprehensions.

### Range Expressions

`start..end` is shorthand syntax for building a range. The current compiler only lowers this form when it appears in a `for` loop header; other contexts should call `range(start, end)` from `stdlib/otter/builtins.ot`. Ranges materialize eagerly and are exclusive of `end`.

```otter
for i in 0..count:
    println(str(i))
```

### Await and Spawn

`spawn` must be followed by a call expression (`spawn fetch_data(url)`). It schedules that call on the runtime task scheduler and returns an opaque handle. Captured variables are copied into the spawned context, so use channels or shared structures if you need to send a result back.

`await handle` blocks until the task represented by `handle` finishes and yields the value that task returned. Handles are typed as `Task<T>` (or `Future<T>`); awaiting them produces `T`. Tasks that do not explicitly return anything still evaluate to `unit`.

```otter
let worker = spawn fetch_data(url)
// ...
await worker
```

### F-Strings and Interpolation

Prefix strings with `f` to embed arbitrary expressions:

```otter
let summary = f"Processed {len(items)} items in {duration_ms}ms"
```

## Statements

### Variable Declarations and Assignment

Use `let` to introduce bindings. `pub let` exports a binding from the current module.

```otter
let total = 0.0
pub let version: string = runtime.version()
```

Simple reassignments omit `let`:

```otter
total = total + chunk
items += [extra]
```

### Expression Statements

Any expression can appear as a statement. This is how function calls and comprehensions that produce side effects are executed.

### Control Flow

#### `if` / `elif` / `else`

```otter
if size == 0:
    return
elif size < 10:
    print("small batch")
else:
    print("large batch")
```

#### `while`

```otter
while remaining > 0:
    remaining -= 1
```

#### `for`

`for` iterates over lists and strings. A `start..end` expression in the loop header is treated specially by the compiler and expanded into a temporary list. Map iteration and custom iterator protocols are not wired up yet.

```otter
for user in users:
    println(user.name)
```

#### `match`

`match` dispatches on patterns. Guards (`case ... if ...`) are not supported in the current grammar.

```otter
let description = match result:
    case Result.Ok(value):
        f"ok: {value}"
    case Result.Err(error):
        f"error: {error}"
```

#### Error Handling with `Result<T, E>`

OtterLang uses `Result<T, E>` enum for error handling instead of exceptions. Functions return `Result.Ok(value)` for success or `Result.Err(error)` for errors.

```otter
fn divide(x: float, y: float) -> Result<float, string>:
    if y == 0.0:
        return Result.Err("Division by zero")
    return Result.Ok(x / y)

match divide(10.0, 2.0):
    case Result.Ok(value):
        println(f"Result: {value}")
    case Result.Err(error):
        println(f"Error: {error}")
```

For unrecoverable errors, use `panic(message)` from the standard library.

### Loop Control

Use `break`, `continue`, and `pass` inside loops or placeholders. `return` exits the current function.

## Functions and Methods

Functions use the following syntax:

```otter
pub fn greet(name: string, greeting: string = "Hello") -> string:
    return f"{greeting}, {name}!"

fn main():
    println("Hello, World!")
```

- Functions are declared with `fn` followed by the function name, parameters in parentheses, optional return type, and a colon
- Parameters can have default values. Once a parameter declares a default, all subsequent parameters must also declare defaults
- Functions currently cannot declare `<T>` parameter lists.
- Function declarations are only permitted at module scope; define helpers as separate top-level functions.
- Method definitions live inside `struct` blocks. The parser automatically inserts `self: StructName` as the first parameter if you omit it.

`print`, `println`, and `eprintln` automatically stringify whatever value you pass—integers, floats, bools, structs with `str()` implementations, etc.—so you can log `println(foo.bar)` without wrapping it in `str(...)` or formatting the value manually.

Top-level code may contain `fn` definitions, `let` bindings, `struct`/`enum`/`type` declarations, `use`/`pub use` statements, and expression statements. Control-flow constructs such as `if`/`for` must appear inside one of those blocks.

## Structs

Structs group named fields and optional methods.

```otter
pub struct Point:
    x: float
    y: float

    fn distance(self) -> float:
        return math.sqrt(self.x * self.x + self.y * self.y)
```

Instantiate structs with keyword arguments: `Point(x=3.0, y=4.0)`.

Struct definitions can declare generics: `struct Box<T>:`.

## Enums

Enums define tagged unions. Variants either carry payloads or act as unit variants.

```otter
pub enum Result<T, E>:
    Ok: (T)
    Err: (E)
```

Construct variants via `Result.Ok(value)`/`Result.Err(error)` and pattern match on them in `match` expressions.

## Pattern Matching

Patterns allow destructuring and conditional matching in `match` expressions and `let` bindings:

| Pattern | Example | Description |
|---------|---------|-------------|
| Wildcard | `_` | Matches any value, ignores it |
| Variable | `name` | Binds the matched value to a variable |
| Literal | `42`, `"hello"`, `true` | Matches exact values |
| Enum | `Result.Ok(value)` | Matches enum variants with payloads |
| Struct | `Point{x, y}` | Destructures struct fields |
| List | `[head, tail]..rest` | Matches fixed leading elements with an optional trailing capture |

Patterns are used in:
- `match` expression case clauses

Destructuring `let` bindings and pattern parameters are not implemented yet.

## Modules and Visibility

Each `.ot` file defines a module. Items are private by default. Mark functions, structs, enums, `let` bindings, and type aliases with `pub` to export them. A `use` statement may import one or more module paths separated by commas, and each path may provide an alias:

```otter
use std/io as io
use math, std/time as time
```

`pub use` re-exports either an entire module (`pub use math`) or a specific symbol (`pub use math.sqrt as square_root`). Unlike `use`, the `pub use` syntax accepts only a single path; you can re-export multiple items by writing multiple statements.

Only the built-in primitives (enums, `Option`/`Result`, `panic`, `print`, `len`, and the core string/list/map helpers plus arithmetic) live in the implicit prelude. Every other stdlib module—`http`, `json`, `io`, `sys`, `net`, `runtime`, `task`, etc.—must be imported before its dotted members (`module.fn`) become visible.

Module paths consist of segments separated by `/` or `:` (`use std/io`). Paths may begin with `.` or `..` for relative imports, and transparent Rust FFI uses the same mechanism (`use rust:serde/json`).

## Concurrency Primitives

OtterLang currently ships two layers of concurrency support:

1. **Language-level operators**: `spawn fn_call(...)` schedules a function call on the task runtime and returns a handle. `await handle` blocks until the task finishes and evaluates to the task's return value, enabling typed pipelines of `Task<T>` handles.
2. **Standard library**: `stdlib/otter/task.ot` exposes helpers for spawning tasks, joining or detaching handles, sleeping, working with typed channels, and building `select` statements. `stdlib/otter/sync` adds mutexes, wait groups, atomics, and `Once` primitives for coordinating work across threads.

Example:

```otter
use task

fn process_batch(batch: list<int>, sink: Channel<string>):
    # write the summary where the caller can read it later
    task.send_string(sink, f"processed {len(batch)} items")

fn main():
    let batch = [1, 2, 3]
    let results = task.channel_string()
    let worker = spawn process_batch(batch, results)
    await worker
    let summary = task.recv_string(results)
    println(summary)
```

## Error Handling

OtterLang uses `Result<T, E>` enum for error handling. Functions return `Result.Ok(value)` for success or `Result.Err(error)` for errors. Pattern matching with `match` is used to handle results.

- `Result<T, E>` and `Option<T>` live in `stdlib/otter/core.ot` and provide algebraic error handling.
- `panic(message)` is a built-in for unrecoverable failures.
- Use `match` expressions to handle `Result` and `Option` values.
- The `exceptions` runtime module surfaces lower-level exception state for FFI integrations, but the language itself does not raise/catch exceptions.

## Standard Library Overview

The `stdlib/otter` directory contains the modules shipped with the compiler. Import them with `use` statements.

- **builtins** – fundamental helpers such as `len`, `cap`, list/map mutation, `panic`, `recover`, `type_of`, `append`, `range`, and structured error utilities (`try_func`, `select`, `defer`).
- **core** – definitions of `Option<T>` and `Result<T, E>`.
- **fmt** – lightweight wrappers around standard output (`print`, `println`, `eprintln`).
- **fs** – filesystem helpers: `exists`, `mkdir`, `remove`, `list_dir`, file IO shortcuts, etc.
- **http** – convenience wrappers for HTTP verbs built on the runtime networking stack.
- **io** – file IO plus buffered IO helpers.
- **json** – encoding/decoding JSON strings, pretty printing, and validation.
- **math** – numeric algorithms (`sqrt`, `pow`, `exp`, `clamp`, `randf`, etc.).
- **net** – TCP-style networking primitives plus HTTP response helpers.
- **rand** – RNG seeding plus integer/float random generators.
- **runtime** – introspection and GC helpers (`gos`, `cpu_count`, `memory`, `stats`, `collect_garbage`).
- **sys** – host information (`cores`, memory totals), environment variables, and process termination helpers.
- **sync** – mutexes, wait groups, once cells, and atomics for cross-thread coordination.
- **task** – task spawning, sleeping, typed channels, select helpers, and task metrics.
- **time** – timestamps, sleeping, timers, formatting, and parsing.
- **yaml** – parsing and emitting YAML strings.
- **exceptions** – access to the runtime exception buffer for FFI integrations.
- **test** – helpers for building simple assertions and test harnesses.

Many of these modules are implemented in OtterLang (`stdlib/otter/*.ot`), while others (e.g., `http`, `exceptions`, `sync`) are written in Rust and exposed through the symbol registry. You always import them with the same `use module_name` syntax.

## Grammar

The summary below mirrors the parser implementation in [`crates/otterc_parser/src/grammar.rs`](../crates/otterc_parser/src/grammar.rs). Whitespace and indentation are omitted for brevity. Optional elements appear in `[square brackets]`.

### Program Structure

```
program         := (use_stmt | pub_use_stmt | type_alias | struct_def | enum_def | function | statement)*
statement       := let_stmt | assignment_stmt | augmented_assignment | return_stmt
                   | break_stmt | continue_stmt | pass_stmt | if_stmt | while_stmt
                   | for_stmt | match_stmt | expr_stmt
```

### Modules and Imports

```
use_stmt        := "use" use_import ("," use_import)*
use_import      := module_path ["as" identifier]
pub_use_stmt    := "pub" "use" module_path ["." identifier] ["as" identifier]
module_path     := path_segment (("/" | ":") path_segment)*
path_segment    := identifier | "." | ".."
```

### Types and Type Aliases

```
type            := identifier ["<" type ("," type)* ">"]
type_alias      := ["pub"] "type" identifier ["<" type_params ">"] "=" type
type_params     := identifier ("," identifier)*
```

### Functions

```
function        := ["pub"] "fn" identifier "(" [params] ")" ["->" type] ":" block
params          := param ("," param)*
param           := identifier [":" type] ["=" expr]
block           := NEWLINE INDENT statement+ DEDENT
```

### Structs and Enums

```
struct_def      := ["pub"] "struct" identifier ["<" type_params ">"] ":" NEWLINE
                   INDENT struct_item* DEDENT
struct_item     := struct_field NEWLINE | method_def
struct_field    := identifier ":" type
method_def      := "fn" identifier "(" [params] ")" ["->" type] ":" block

enum_def        := ["pub"] "enum" identifier ["<" type_params ">"] ":" NEWLINE
                   INDENT enum_variant+ DEDENT
enum_variant    := identifier [":" "(" type ("," type)* ")"]
```

### Expressions

```
expr            := logical_or_expr
logical_or_expr := logical_and_expr ("or" logical_and_expr)*
logical_and_expr:= comparison_expr ("and" comparison_expr)*
comparison_expr := range_expr ((comparison_op | is_op) range_expr)*
comparison_op   := "==" | "!=" | "<" | "<=" | ">" | ">="
is_op           := "is" ["not"]
range_expr      := additive_expr [".." additive_expr]
additive_expr   := multiplicative_expr (("+" | "-") multiplicative_expr)*
multiplicative_expr := unary_expr (("*" | "/" | "%") unary_expr)*
unary_expr      := ("not" | "!" | "-" | "+") unary_expr
                 | await_expr
                 | spawn_expr
                 | call_expr
await_expr      := "await" call_expr
spawn_expr      := "spawn" call_expr
call_expr       := member_expr ("(" [expr ("," expr)*] ")")*
member_expr     := primary_expr ("." identifier)*
primary_expr    := literal
                 | identifier
                 | "(" expr ")"
                 | struct_init
                 | list_literal
                 | dict_literal
                 | list_comprehension
                 | dict_comprehension
literal         := INTEGER | FLOAT | STRING | FSTRING | "true" | "false" | "None" | "()"
struct_init     := identifier "(" field_init ("," field_init)* ")"
field_init      := identifier "=" expr
list_literal    := "[" [expr ("," expr)*] "]"
dict_literal    := "{" [dict_entry ("," dict_entry)*] "}"
dict_entry      := expr ":" expr
list_comprehension := "[" expr "for" identifier "in" expr ["if" expr] "]"
dict_comprehension := "{" expr ":" expr "for" identifier "in" expr ["if" expr] "}"
```

### Statements

```
let_stmt        := ["pub"] "let" identifier [":" type] "=" expr
assignment_stmt := identifier "=" expr
augmented_assignment := identifier ("+=" | "-=" | "*=" | "/=") expr

return_stmt     := "return" [expr]
break_stmt      := "break"
continue_stmt   := "continue"
pass_stmt       := "pass"

if_stmt         := "if" expr ":" block ("elif" expr ":" block)* ["else" ":" block]
while_stmt      := "while" expr ":" block
for_stmt        := "for" identifier "in" expr ":" block

match_stmt      := "match" expr ":" NEWLINE INDENT match_case+ DEDENT
match_case      := "case" pattern ":" block
```

### Patterns

```
pattern         := wildcard_pattern | literal_pattern | identifier_pattern
                   | enum_pattern | struct_pattern | list_pattern

wildcard_pattern    := "_"
literal_pattern     := literal
identifier_pattern  := identifier
enum_pattern        := identifier "." identifier ["(" pattern ("," pattern)* ")"]
struct_pattern      := identifier "{" [field_pattern ("," field_pattern)*] "}"
field_pattern       := identifier [":" pattern]
list_pattern        := "[" [pattern ("," pattern)*] "]" [".." identifier]
```

### Operators and Precedence

Operators are listed from highest to lowest precedence:

```
Primary:     () [] . call
Unary:       await spawn not ! + -
Multiplicative: * / %
Additive:    + -
Range:       ..
Comparison:  == != < <= > >= is is not
Logical AND: and
Logical OR:  or
```

### Lexical Structure

```
identifier      := [a-zA-Z_][a-zA-Z0-9_]* (Unicode identifiers are also accepted)
keyword         := fn | let | return | if | elif | else | for | while
                   | match | case | struct | enum | pub | await | spawn
                   | true | false | None | and | or | not | in | is | as | use
                   | break | continue | pass | print
comment         := "#" [^\n]*
whitespace      := [ \t\n\r]+
```

## Semantics and Implementation Notes

- **Type Checking** – Static type checking with inference is performed before code generation. Generic parameters default to unconstrained type variables.
- **Evaluation Order** – Expressions evaluate left-to-right. Function arguments are evaluated before the call.
- **Memory Management** – The runtime ships multiple GC strategies (reference counting for short-lived objects, mark-and-sweep, and arena allocators) that can be selected via `runtime/memory` configuration. Modules such as `gc` and `runtime` expose helpers for GC control from Otter code.
- **Code Generation** – The `otter` binary currently targets LLVM for JIT/native code generation.
- **Task Runtime** – `spawn`, `await`, `task.*` helpers, and `sync` primitives are thin wrappers around the scheduler implemented in `src/runtime/task`, so task handles, typed channels, and wait groups interoperate consistently.
- **Tooling** – The repository ships a formatter, language server, and VS Code extension that all understand the syntax described in this document.

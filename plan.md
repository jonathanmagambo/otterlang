# OtterLang Development Plan

## Exception Handling Feature Implementation

This document tracks the implementation of Python-style exception handling (`try/except/else/finally`) in OtterLang.

### Overview

The exception handling feature will provide Python-compatible syntax with zero-cost exception handling on the success path, following these design principles:

- **Python Compatibility**: `try:`, `except Type as name:`, `else:`, `finally:` syntax
- **Zero Cost Success Path**: No allocations or runtime overhead when no exceptions occur
- **Type Safety**: Exception types are validated at compile time
- **Efficient Implementation**: Thread-local error stack with LLVM desugaring

### Implementation Phases

#### Phase 1: Lexer/AST/Formatter
- [ ] Add `Try`, `Except`, `Finally`, `Raise` tokens to lexer
- [ ] Create `Statement::Try` with except handlers, optional else, optional finally
- [ ] Add `ExceptHandler` struct: `{ exception: Option<Type>, alias: Option<String>, body: Block }`
- [ ] Update formatter to pretty-print exception constructs

#### Phase 2: Parser
- [ ] Recognize `try:` blocks followed by indented suites
- [ ] Support multiple `except` clauses in Python order (except, else, finally)
- [ ] Parse `except Type as name:` and bare `except:` syntax
- [ ] Handle `raise` statement with zero or one expression

#### Phase 3: Type Checker
- [ ] Validate handler exception types (must be compatible exceptions)
- [ ] Add `Error` type alias (TypeInfo::Struct or new variant)
- [ ] Ensure handler alias variables are scoped within handler blocks
- [ ] Handle `raise` forms: with expression (must be Error-compatible) or without (inside handler)
- [ ] Add diagnostics for unreachable/missing handlers and finally behavior

#### Phase 4: Runtime
- [ ] Create `OtError` struct and thread-local error stack
- [ ] Implement C-facing functions: push_context, pop, raise, clear, get_message, rethrow
- [ ] Ensure zero cost on success path (no allocations unless error occurs)

#### Phase 5: LLVM Lowering
- [ ] Desugar `try` into basic blocks with fast success path
- [ ] Generate exception handler branching logic (first handler tests type and consumes error)
- [ ] Implement `else` execution (only when no exception occurred)
- [ ] Ensure `finally` always runs via cleanup paths
- [ ] Map `raise` to runtime raise helper with error block exit

#### Phase 6: Tests/Examples/Docs
- [ ] Add comprehensive unit/integration tests
- [ ] Test successful try/else/finally execution
- [ ] Test matching and non-matching exception handling
- [ ] Test nested try/finally behavior
- [ ] Test re-raise functionality
- [ ] Document syntax in README and stdlib docs
- [ ] Create example programs in `examples/` directory

### Success Criteria

- [ ] All syntax elements parse correctly
- [ ] Type checking validates exception compatibility
- [ ] Runtime handles exceptions with zero success path cost
- [ ] LLVM generation produces correct control flow
- [ ] All test cases pass
- [ ] Documentation is complete and accurate
- [ ] Examples demonstrate real-world usage patterns

### Implementation Notes

- Follow Python exception semantics exactly
- Maintain backward compatibility with existing code
- Ensure thread safety in runtime error handling
- Optimize for the common case (no exceptions)
- Provide clear error messages for exception-related issues

# Blood Error Handling Best Practices

**Version**: 0.1.0
**Status**: Tutorial
**Last Updated**: 2026-01-14

This guide covers error handling strategies in Blood, including when to use effects vs `Result`, error type design, recovery patterns, and testing error paths.

---

## Table of Contents

1. [Error Handling Philosophy](#1-error-handling-philosophy)
2. [Choosing an Error Strategy](#2-choosing-an-error-strategy)
3. [Using Result<T, E>](#3-using-resultt-e)
4. [Effect-Based Error Handling](#4-effect-based-error-handling)
5. [Error Type Design](#5-error-type-design)
6. [Propagation Patterns](#6-propagation-patterns)
7. [Recovery Strategies](#7-recovery-strategies)
8. [Logging and Telemetry](#8-logging-and-telemetry)
9. [Testing Error Paths](#9-testing-error-paths)
10. [Anti-Patterns to Avoid](#10-anti-patterns-to-avoid)

---

## 1. Error Handling Philosophy

### 1.1 Blood's Approach

Blood provides two complementary error handling mechanisms:

| Mechanism | Use Case | Propagation | Recovery |
|-----------|----------|-------------|----------|
| `Result<T, E>` | Expected failures | Explicit (`?`) | At call site |
| Effect `Error<E>` | Recoverable errors | Implicit (effect row) | Via handler |

**Key Insight**: Effects give you "exceptions done right"—structured, typed, and composable—while `Result` provides explicit control flow.

### 1.2 Guiding Principles

1. **Be explicit about failure modes**: Functions should declare their error possibilities
2. **Fail early, fail loud**: Don't hide errors; surface them immediately
3. **Provide actionable information**: Error messages should help users recover
4. **Separate concerns**: Error detection, propagation, and recovery are different concerns
5. **Test error paths**: Error handling code needs testing too

### 1.3 Error Categories

| Category | Example | Typical Handling |
|----------|---------|------------------|
| **Recoverable** | File not found | Retry, fallback, user prompt |
| **Propagatable** | Parse error | Bubble up to caller |
| **Fatal** | Out of memory | Terminate gracefully |
| **Programmer error** | Index out of bounds | Fix the code |

---

## 2. Choosing an Error Strategy

### 2.1 Decision Tree

```
Does the caller ALWAYS need to handle this error?
│
├─ YES → Use Result<T, E>
│        (Forces explicit handling)
│
└─ NO
   │
   ├─ Can error handling be DEFERRED to an outer scope?
   │  │
   │  ├─ YES → Use Effect Error<E>
   │  │        (Handler decides recovery strategy)
   │  │
   │  └─ NO
   │     │
   │     └─ Is this an unrecoverable programmer error?
   │        │
   │        ├─ YES → Use panic/unreachable
   │        │        (Bug in program logic)
   │        │
   │        └─ NO → Use Result<T, E>
   │                (Default to explicit)
```

### 2.2 When to Use Result

Use `Result<T, E>` when:

- The caller **must** handle the error immediately
- Error handling varies significantly by call site
- You want explicit control flow
- Performance is critical (no handler lookup)
- You're writing library code

```blood
// GOOD: Result for parsing (caller decides how to handle parse failure)
fn parse_config(text: &str) -> Result<Config, ParseError> / pure {
    // Parse logic...
}

// Usage: caller MUST handle
let config = match parse_config(text) {
    Ok(c) => c,
    Err(e) => return Err(e.into()),  // Propagate
};
```

### 2.3 When to Use Effect Error

Use effect `Error<E>` when:

- Error handling should be **centralized**
- Multiple operations can fail with similar errors
- You want **flexible recovery strategies**
- The error handling policy should be **configurable**

```blood
// GOOD: Effect for database errors (handler decides retry policy)
effect DbError {
    op db_error(err: SqlError) -> !;  // Never returns (handler takes over)
}

fn query_user(id: UserId) -> User / {DbError} {
    match execute_query(id) {
        Ok(user) => user,
        Err(e) => perform DbError::db_error(e),  // Handler decides
    }
}

// Usage: handler centralizes policy
with RetryHandler { retries: 3 } handle {
    let user = query_user(id);  // Retries automatically
    process(user);
}
```

### 2.4 Comparison Matrix

| Aspect | `Result<T, E>` | Effect `Error<E>` |
|--------|----------------|-------------------|
| Propagation | Explicit (`?`) | Implicit (effect row) |
| Handling | At each call site | Centralized in handler |
| Performance | Zero overhead | Handler lookup (~1 cycle) |
| Flexibility | Fixed at call site | Configurable via handler |
| Composability | Manual chaining | Automatic with effect rows |
| Type safety | Full | Full |

---

## 3. Using Result<T, E>

### 3.1 Basic Patterns

**Creating Results:**
```blood
fn divide(a: i32, b: i32) -> Result<i32, DivideError> / pure {
    if b == 0 {
        Err(DivideError::DivisionByZero)
    } else {
        Ok(a / b)
    }
}
```

**Propagating with `?`:**
```blood
fn compute(x: i32, y: i32) -> Result<i32, MathError> / pure {
    let a = divide(x, y)?;      // Propagates Err if division fails
    let b = square_root(a)?;    // Propagates Err if sqrt fails
    Ok(b + 1)
}
```

**Handling Results:**
```blood
fn process(input: String) -> Output {
    match parse(input) {
        Ok(data) => transform(data),
        Err(ParseError::InvalidFormat(s)) => {
            log_warning(format!("Invalid format: {}", s));
            Output::default()
        }
        Err(ParseError::TooLarge) => {
            log_error("Input too large");
            Output::error()
        }
    }
}
```

### 3.2 Combinators

**map - Transform success value:**
```blood
let doubled: Result<i32, E> = result.map(|x| x * 2);
```

**map_err - Transform error value:**
```blood
let converted: Result<T, NewError> = result.map_err(|e| NewError::from(e));
```

**and_then - Chain operations:**
```blood
let final_result = parse(input)
    .and_then(|data| validate(data))
    .and_then(|valid| process(valid));
```

**or_else - Recover from error:**
```blood
let result = primary_source()
    .or_else(|_| fallback_source())
    .or_else(|_| default_value());
```

**unwrap_or - Provide default:**
```blood
let value = result.unwrap_or(default);
```

**unwrap_or_else - Compute default lazily:**
```blood
let value = result.unwrap_or_else(|e| {
    log_error(e);
    compute_default()
});
```

### 3.3 Type Inference with Results

```blood
// Specify error type when ambiguous
let x: Result<_, ParseError> = "123".parse();

// Use turbofish for methods
let parsed = input.parse::<i32>();

// Type inference from context
fn process() -> Result<i32, MyError> {
    let x = "42".parse()?;  // Infers i32 from return type
    Ok(x)
}
```

---

## 4. Effect-Based Error Handling

### 4.1 Defining Error Effects

```blood
// Simple error effect
effect Fail<E> {
    op fail(error: E) -> !;  // Never returns normally
}

// Error effect with recovery option
effect Error<E> {
    op raise(error: E) -> !;
    op try_recover(error: E) -> bool;  // Handler can attempt recovery
}

// Typed error effect for specific domain
effect ParseError {
    op invalid_token(pos: usize, found: char) -> !;
    op unexpected_eof(expected: String) -> !;
    op syntax_error(msg: String) -> !;
}
```

### 4.2 Raising Errors

```blood
fn parse_number(s: &str) -> i32 / {ParseError} {
    if s.is_empty() {
        perform ParseError::unexpected_eof("digit".to_string());
    }

    let mut result = 0;
    for (i, c) in s.chars().enumerate() {
        if !c.is_digit(10) {
            perform ParseError::invalid_token(i, c);
        }
        result = result * 10 + (c as i32 - '0' as i32);
    }
    result
}
```

### 4.3 Handler Patterns

**Fail-Fast Handler:**
```blood
deep handler FailFast<E: Debug> for Fail<E> {
    return(x) { Ok(x) }

    op fail(error) {
        Err(error)
    }
}

// Usage
let result: Result<i32, MyError> = with FailFast handle {
    risky_operation()
};
```

**Default Value Handler:**
```blood
deep handler DefaultOnError<T, E> for Fail<E> {
    let default_value: T

    return(x) { x }

    op fail(_error) {
        resume(default_value)  // Continue with default
    }
}

// Usage
let value: i32 = with DefaultOnError { default_value: 0 } handle {
    risky_operation()
};
```

**Retry Handler:**
```blood
deep handler RetryHandler<E> for Fail<E> {
    let max_retries: i32
    let mut attempts: i32 = 0

    return(x) { Ok(x) }

    op fail(error) {
        attempts += 1;
        if attempts < max_retries {
            // Resume the operation (retry)
            resume(())
        } else {
            Err(error)
        }
    }
}
```

**Logging Handler:**
```blood
deep handler LoggingFailHandler<E: Display> for Fail<E> {
    return(x) { Ok(x) }

    op fail(error) {
        log_error(format!("Operation failed: {}", error));
        Err(error)
    }
}
```

### 4.4 Composing Error Effects

```blood
// Multiple error effects can compose
fn complex_operation() / {ParseError, IoError, DbError} {
    let config = parse_config()?;    // May raise ParseError
    let data = read_file(path)?;     // May raise IoError
    save_to_db(data)?;               // May raise DbError
}

// Handle each differently
with ParseErrorHandler handle {
    with IoErrorHandler handle {
        with DbErrorHandler handle {
            complex_operation()
        }
    }
}
```

---

## 5. Error Type Design

### 5.1 Error Type Hierarchy

```blood
// Root error type with variants for each category
enum AppError {
    // IO errors
    Io(IoError),

    // Parsing errors
    Parse(ParseError),

    // Business logic errors
    Validation(ValidationError),

    // External service errors
    External(ExternalError),
}

// Specific error types
enum IoError {
    NotFound { path: String },
    PermissionDenied { path: String },
    Timeout { duration: Duration },
}

enum ParseError {
    InvalidSyntax { line: u32, col: u32, msg: String },
    UnexpectedToken { expected: String, found: String },
    UnexpectedEof,
}

enum ValidationError {
    Required { field: String },
    InvalidFormat { field: String, expected: String },
    OutOfRange { field: String, min: i64, max: i64, got: i64 },
}
```

### 5.2 Error Trait Implementation

```blood
trait Error: Display + Debug {
    fn source(&self) -> Option<&dyn Error> { None }
    fn code(&self) -> &'static str;
}

impl Error for IoError {
    fn code(&self) -> &'static str {
        match self {
            IoError::NotFound { .. } => "IO_NOT_FOUND",
            IoError::PermissionDenied { .. } => "IO_PERMISSION_DENIED",
            IoError::Timeout { .. } => "IO_TIMEOUT",
        }
    }
}

impl Display for IoError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            IoError::NotFound { path } =>
                write!(f, "File not found: {}", path),
            IoError::PermissionDenied { path } =>
                write!(f, "Permission denied: {}", path),
            IoError::Timeout { duration } =>
                write!(f, "Operation timed out after {:?}", duration),
        }
    }
}
```

### 5.3 Conversion Traits

```blood
// Automatic conversion with From trait
impl From<IoError> for AppError {
    fn from(e: IoError) -> AppError {
        AppError::Io(e)
    }
}

impl From<ParseError> for AppError {
    fn from(e: ParseError) -> AppError {
        AppError::Parse(e)
    }
}

// Enables automatic conversion with ?
fn process_file(path: &str) -> Result<Data, AppError> {
    let content = read_file(path)?;  // IoError -> AppError
    let parsed = parse(content)?;    // ParseError -> AppError
    Ok(parsed)
}
```

### 5.4 Context and Wrapping

```blood
// Add context to errors
struct ErrorWithContext<E> {
    inner: E,
    context: String,
    location: &'static str,
}

// Extension trait for adding context
trait ResultExt<T, E> {
    fn context(self, ctx: &str) -> Result<T, ErrorWithContext<E>>;
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T, ErrorWithContext<E>>;
}

// Usage
fn load_config() -> Result<Config, AppError> {
    let path = get_config_path();
    read_file(&path)
        .context("Failed to load configuration")?
        .parse()
        .with_context(|| format!("Failed to parse config at {}", path))
}
```

### 5.5 Error Codes for APIs

```blood
// Structured error for API responses
struct ApiError {
    code: String,           // Machine-readable: "USER_NOT_FOUND"
    message: String,        // Human-readable: "User with ID 123 not found"
    details: Option<Value>, // Additional structured data
    trace_id: String,       // For debugging
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> ApiError {
        ApiError {
            code: e.code().to_string(),
            message: e.to_string(),
            details: e.details(),
            trace_id: generate_trace_id(),
        }
    }
}
```

---

## 6. Propagation Patterns

### 6.1 The ? Operator

```blood
fn process() -> Result<Output, Error> {
    let a = step_one()?;   // Early return on error
    let b = step_two(a)?;  // Chain operations
    let c = step_three(b)?;
    Ok(c)
}
```

### 6.2 Effect Row Propagation

```blood
// Effects propagate automatically through function signatures
fn inner() / {Fail<E>} {
    perform Fail::fail(error);
}

fn outer() / {Fail<E>} {
    inner();  // Effect propagates to outer
}

// Only need handler at the boundary
fn boundary() -> Result<T, E> {
    with FailFast handle {
        outer()  // All nested Fail effects handled here
    }
}
```

### 6.3 Cross-Boundary Propagation

```blood
// Converting between Result and Effect

// Effect to Result
fn effectful() / {Fail<E>} -> T { ... }

fn as_result() -> Result<T, E> {
    with FailFast handle {
        effectful()
    }
}

// Result to Effect
fn result_based() -> Result<T, E> { ... }

fn as_effect() / {Fail<E>} -> T {
    match result_based() {
        Ok(v) => v,
        Err(e) => perform Fail::fail(e),
    }
}
```

### 6.4 Aggregating Errors

```blood
// Collect all errors, don't stop at first
fn validate_all(items: &[Item]) -> Result<(), Vec<ValidationError>> {
    let errors: Vec<ValidationError> = items
        .iter()
        .filter_map(|item| validate(item).err())
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// Using effects for error accumulation
effect Accumulate<E> {
    op add_error(e: E) -> ();
}

deep handler AccumulateHandler<E> for Accumulate<E> {
    let mut errors: Vec<E> = Vec::new()

    return(x) {
        if errors.is_empty() {
            Ok(x)
        } else {
            Err(errors)
        }
    }

    op add_error(e) {
        errors.push(e);
        resume(())  // Continue, don't stop
    }
}
```

---

## 7. Recovery Strategies

### 7.1 Fallback Values

```blood
// Static default
let config = load_config().unwrap_or(Config::default());

// Computed default
let config = load_config().unwrap_or_else(|e| {
    log_warning(format!("Using default config: {}", e));
    Config::default()
});
```

### 7.2 Fallback Sources

```blood
fn get_data() -> Result<Data, Error> {
    // Try primary source
    primary_source()
        // Fall back to secondary
        .or_else(|_| secondary_source())
        // Fall back to cache
        .or_else(|_| cached_source())
        // Last resort: empty data
        .or_else(|_| Ok(Data::empty()))
}
```

### 7.3 Retry with Backoff

```blood
fn retry_with_backoff<T, E, F>(
    mut operation: F,
    max_attempts: u32,
    initial_delay: Duration,
) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>
{
    let mut delay = initial_delay;

    for attempt in 0..max_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(e) if attempt < max_attempts - 1 => {
                log_warning(format!("Attempt {} failed, retrying in {:?}", attempt + 1, delay));
                sleep(delay);
                delay = delay * 2;  // Exponential backoff
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}
```

### 7.4 Circuit Breaker

```blood
struct CircuitBreaker<E> {
    failure_count: u32,
    failure_threshold: u32,
    last_failure: Option<Instant>,
    reset_timeout: Duration,
    state: CircuitState,
}

enum CircuitState {
    Closed,      // Normal operation
    Open,        // Failing fast
    HalfOpen,    // Testing recovery
}

impl<E> CircuitBreaker<E> {
    fn call<T, F: FnOnce() -> Result<T, E>>(&mut self, f: F) -> Result<T, CircuitError<E>> {
        match self.state {
            CircuitState::Open => {
                if self.should_attempt_reset() {
                    self.state = CircuitState::HalfOpen;
                } else {
                    return Err(CircuitError::Open);
                }
            }
            _ => {}
        }

        match f() {
            Ok(value) => {
                self.on_success();
                Ok(value)
            }
            Err(e) => {
                self.on_failure();
                Err(CircuitError::Inner(e))
            }
        }
    }
}
```

### 7.5 Graceful Degradation

```blood
// Full-featured function
fn get_user_profile(id: UserId) -> UserProfile / {Network, Cache} {
    // Try to get fresh data
    match try_fetch_user(id) {
        Ok(user) => {
            // Update cache
            perform Cache::set(id, user.clone());
            user
        }
        Err(NetworkError::Timeout) => {
            // Degrade to cached data
            match perform Cache::get(id) {
                Some(cached) => {
                    log_warning("Using cached profile due to network timeout");
                    cached
                }
                None => {
                    // Degrade to minimal profile
                    log_warning("No cached profile, using placeholder");
                    UserProfile::placeholder(id)
                }
            }
        }
        Err(e) => {
            perform Network::error(e)
        }
    }
}
```

---

## 8. Logging and Telemetry

### 8.1 Structured Error Logging

```blood
effect Log {
    op log(level: Level, msg: String, context: Map<String, Value>) -> ();
}

fn log_error(error: &dyn Error, context: &RequestContext) / {Log} {
    let mut fields = Map::new();
    fields.insert("error_code", error.code().into());
    fields.insert("error_message", error.to_string().into());
    fields.insert("request_id", context.request_id.into());
    fields.insert("user_id", context.user_id.into());

    // Include error chain
    let mut chain = Vec::new();
    let mut current: Option<&dyn Error> = Some(error);
    while let Some(e) = current {
        chain.push(e.to_string());
        current = e.source();
    }
    fields.insert("error_chain", chain.into());

    perform Log::log(Level::Error, "Operation failed", fields);
}
```

### 8.2 Error Metrics

```blood
effect Metrics {
    op increment(name: String, tags: Map<String, String>) -> ();
    op timing(name: String, duration: Duration, tags: Map<String, String>) -> ();
}

fn track_error<E: Error>(error: &E) / {Metrics} {
    let mut tags = Map::new();
    tags.insert("error_code", error.code().to_string());
    tags.insert("error_type", type_name::<E>().to_string());

    perform Metrics::increment("errors.total", tags);
}
```

### 8.3 Error Reporting Handler

```blood
// Handler that logs all errors before propagating
deep handler ErrorReporter<E: Error> for Fail<E> {
    return(x) { Ok(x) }

    op fail(error) {
        // Log the error
        log_error(&error, &get_context());

        // Track metrics
        track_error(&error);

        // Still fail (propagate the error)
        Err(error)
    }
}
```

---

## 9. Testing Error Paths

### 9.1 Testing Result-Based Code

```blood
#[test]
fn test_division_by_zero() {
    let result = divide(10, 0);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), DivideError::DivisionByZero);
}

#[test]
fn test_parse_error_message() {
    let result = parse("invalid");
    match result {
        Err(ParseError::InvalidSyntax { line, col, msg }) => {
            assert_eq!(line, 1);
            assert!(msg.contains("expected digit"));
        }
        _ => panic!("Expected InvalidSyntax error"),
    }
}
```

### 9.2 Testing Effect-Based Code

```blood
// Handler that captures errors for testing
deep handler TestErrorCapture<E> for Fail<E> {
    let mut captured: Option<E> = None

    return(x) { (x, captured) }

    op fail(error) {
        captured = Some(error);
        Err(error)
    }
}

#[test]
fn test_effect_error() {
    let (result, captured) = with TestErrorCapture handle {
        operation_that_may_fail()
    };

    assert!(result.is_err());
    assert!(captured.is_some());
    assert_eq!(captured.unwrap().code(), "EXPECTED_ERROR");
}
```

### 9.3 Injecting Errors for Testing

```blood
// Handler that injects errors
deep handler ErrorInjector<E> for Fail<E> {
    let should_fail: bool
    let injected_error: E

    return(x) {
        if should_fail {
            Err(injected_error)
        } else {
            Ok(x)
        }
    }

    op fail(error) {
        Err(error)  // Real errors still propagate
    }
}

#[test]
fn test_error_recovery() {
    let result = with ErrorInjector {
        should_fail: true,
        injected_error: TestError::Simulated
    } handle {
        with RecoveryHandler handle {
            risky_operation()
        }
    };

    // Verify recovery worked
    assert!(result.is_ok());
}
```

### 9.4 Property-Based Testing

```blood
#[test]
fn prop_parse_never_panics() {
    // Generate random strings
    for input in arbitrary_strings(1000) {
        // Should return Result, never panic
        let result = parse(&input);
        assert!(result.is_ok() || result.is_err());
    }
}

#[test]
fn prop_errors_have_context() {
    for input in invalid_inputs(100) {
        let result = process(input);
        if let Err(e) = result {
            // Every error should have useful context
            assert!(!e.to_string().is_empty());
            assert!(!e.code().is_empty());
        }
    }
}
```

---

## 10. Anti-Patterns to Avoid

### 10.1 Swallowing Errors

```blood
// BAD: Error silently ignored
fn bad_process(input: String) -> Output {
    let data = parse(input).unwrap_or_default();  // Error lost!
    transform(data)
}

// GOOD: Error is logged or propagated
fn good_process(input: String) -> Result<Output, ProcessError> {
    let data = parse(input).map_err(|e| {
        log_warning(format!("Parse failed: {}", e));
        ProcessError::Parse(e)
    })?;
    Ok(transform(data))
}
```

### 10.2 Excessive Unwrapping

```blood
// BAD: Panic on any error
fn bad_chain(input: String) -> Output {
    let a = step_one(input).unwrap();
    let b = step_two(a).unwrap();
    step_three(b).unwrap()
}

// GOOD: Proper error handling
fn good_chain(input: String) -> Result<Output, Error> {
    let a = step_one(input)?;
    let b = step_two(a)?;
    step_three(b)
}
```

### 10.3 Stringly-Typed Errors

```blood
// BAD: Error type is just String
fn bad_validate(data: Data) -> Result<(), String> {
    if data.value < 0 {
        Err("Value must be positive".to_string())
    } else {
        Ok(())
    }
}

// GOOD: Structured error type
fn good_validate(data: Data) -> Result<(), ValidationError> {
    if data.value < 0 {
        Err(ValidationError::OutOfRange {
            field: "value".to_string(),
            min: 0,
            max: i64::MAX,
            got: data.value,
        })
    } else {
        Ok(())
    }
}
```

### 10.4 Catching Too Broadly

```blood
// BAD: Catches all errors the same way
fn bad_handler() {
    with CatchAll handle {  // Loses error specificity
        complex_operation()
    }
}

// GOOD: Handle different errors appropriately
fn good_handler() -> Result<Output, Error> {
    with NetworkErrorHandler handle {
        with ValidationErrorHandler handle {
            with DbErrorHandler handle {
                complex_operation()
            }
        }
    }
}
```

### 10.5 Mixing Error Strategies

```blood
// BAD: Inconsistent error handling
fn bad_mixed(input: String) -> Option<Output> / {Fail<Error>} {
    let a = step_one(input)?;  // Uses ?
    let b = match step_two(a) {  // Uses match
        Ok(v) => v,
        Err(e) => perform Fail::fail(e),  // Uses effect
    };
    Some(step_three(b).ok()?)  // Uses Option
}

// GOOD: Consistent approach
fn good_consistent(input: String) -> Result<Output, Error> {
    let a = step_one(input)?;
    let b = step_two(a)?;
    step_three(b)
}
```

---

## Quick Reference

### Choosing Error Strategy

| Scenario | Use |
|----------|-----|
| Caller must handle immediately | `Result<T, E>` |
| Centralized error handling | Effect `Fail<E>` |
| Unrecoverable bug | `panic!` |
| Optional value (not error) | `Option<T>` |

### Common Patterns

```blood
// Propagate error
let x = risky_operation()?;

// Provide default
let x = risky_operation().unwrap_or(default);

// Convert error type
let x = risky_operation().map_err(AppError::from)?;

// Add context
let x = risky_operation().context("during initialization")?;

// Effect-based recovery
with RetryHandler { max: 3 } handle {
    risky_operation()
}
```

---

## Related Documentation

- [STDLIB.md](../spec/STDLIB.md) - Result and Option types
- [EFFECTS_COOKBOOK.md](./EFFECTS_COOKBOOK.md) - Effect patterns
- [DEBUGGING_GUIDE.md](./DEBUGGING_GUIDE.md) - Error debugging
- [DIAGNOSTICS.md](../spec/DIAGNOSTICS.md) - Compiler error codes

---

*Last updated: 2026-01-14*

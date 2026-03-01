# Blood Effects Cookbook

**Version**: 0.1.0
**Last Updated**: 2026-01-14

This cookbook provides practical patterns and recipes for using Blood's algebraic effect system effectively.

---

## Table of Contents

1. [Basic Patterns](#1-basic-patterns)
2. [State Management](#2-state-management)
3. [Error Handling](#3-error-handling)
4. [Logging and Tracing](#4-logging-and-tracing)
5. [Fiber/Concurrent Patterns](#5-fiberconcurrent-patterns)
6. [Effect Composition](#6-effect-composition)
7. [Handler Optimization](#7-handler-optimization)
8. [Testing with Effects](#8-testing-with-effects)
9. [Anti-Patterns](#9-anti-patterns)

---

## 1. Basic Patterns

### 1.1 Defining a Simple Effect

```blood
// Effect declaration: what operations are available
effect Logger {
    op log(level: LogLevel, message: String);
}

enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

// Function using the effect
fn do_work() / {Logger} {
    perform Logger.log(LogLevel::Info, "Starting work...");
    // ... work ...
    perform Logger.log(LogLevel::Info, "Work complete");
}
```

### 1.2 Simple Handler

```blood
// Handler: how to interpret the effect
handler ConsoleLogger for Logger {
    return(x) { x }

    log(level, message) {
        let prefix = match level {
            LogLevel::Debug => "[DEBUG]",
            LogLevel::Info  => "[INFO]",
            LogLevel::Warn  => "[WARN]",
            LogLevel::Error => "[ERROR]",
        };
        println("{} {}", prefix, message);
        resume(())
    }
}

fn main() {
    with ConsoleLogger handle {
        do_work()
    }
}
```

### 1.3 Effect with Return Value

```blood
effect Reader<T> {
    op ask() -> T;
}

fn get_config() / {Reader<Config>} -> String {
    let config = perform Reader.ask();
    config.database_url
}

handler ConfigReader(config: Config) for Reader<Config> {
    return(x) { x }

    ask() {
        resume(config)  // Return the config value
    }
}
```

---

## 2. State Management

### 2.1 Mutable State Effect

```blood
effect State<T> {
    op get() -> T;
    op put(value: T);
    op modify(f: fn(T) -> T);
}

// Counter using state
fn count_up(n: i32) / {State<i32>} -> i32 {
    for _ in 0..n {
        perform State.modify(|x| x + 1);
    }
    perform State.get()
}
```

### 2.2 Stateful Handler

```blood
handler StateHandler<T>(mut state: T) for State<T> {
    return(x) { (x, state) }  // Return result and final state

    get() {
        resume(state)
    }

    put(value) {
        state = value;
        resume(())
    }

    modify(f) {
        state = f(state);
        resume(())
    }
}

fn main() {
    let (result, final_state) = with StateHandler(0) handle {
        count_up(5)
    };
    println("Result: {}, Final state: {}", result, final_state);
    // Output: Result: 5, Final state: 5
}
```

### 2.3 Multiple State Handlers

```blood
// Using two different state effects
fn transfer() / {State<Account>, State<TransactionLog>} {
    let account = perform State<Account>.get();

    // Update account
    perform State<Account>.put(Account {
        balance: account.balance - 100
    });

    // Log transaction
    let log = perform State<TransactionLog>.get();
    perform State<TransactionLog>.put(log.append("Withdrew 100"));
}
```

---

## 3. Error Handling

### 3.1 Error Effect

```blood
effect Error<E> {
    op throw(error: E) -> !;  // Never returns normally
}

fn divide(a: i32, b: i32) / {Error<String>} -> i32 {
    if b == 0 {
        perform Error.throw("Division by zero");
    }
    a / b
}
```

### 3.2 Try-Catch Handler

```blood
handler TryCatch<E, T> for Error<E> {
    return(x) { Ok(x) }

    throw(error) {
        // Don't resume - return error instead
        Err(error)
    }
}

fn main() {
    let result = with TryCatch handle {
        let x = divide(10, 2);  // Ok: 5
        let y = divide(x, 0);   // Error: "Division by zero"
        y
    };

    match result {
        Ok(value) => println("Result: {}", value),
        Err(msg)  => println("Error: {}", msg),
    }
}
```

### 3.3 Default Value on Error

```blood
handler DefaultOnError<E, T>(default: T) for Error<E> {
    return(x) { x }

    throw(_error) {
        default  // Return default, don't resume
    }
}

fn safe_divide(a: i32, b: i32) -> i32 {
    with DefaultOnError(0) handle {
        divide(a, b)
    }
}
```

### 3.4 Retry on Error

```blood
handler RetryOnError<E>(max_retries: i32) for Error<E> {
    state: retries: i32 = 0

    return(x) { x }

    throw(error) {
        if retries < max_retries {
            retries = retries + 1;
            resume(())  // Retry the operation
        } else {
            panic!("Max retries exceeded: {}", error)
        }
    }
}
```

---

## 4. Logging and Tracing

### 4.1 Structured Logging

```blood
effect Trace {
    op span_enter(name: String);
    op span_exit(name: String);
    op event(level: LogLevel, message: String, fields: Map<String, String>);
}

fn traced_function() / {Trace} {
    perform Trace.span_enter("traced_function");

    perform Trace.event(LogLevel::Debug, "Starting", {});

    // ... do work ...

    perform Trace.event(LogLevel::Info, "Completed", {
        "items_processed" => "42"
    });

    perform Trace.span_exit("traced_function");
}
```

### 4.2 Timing Handler

```blood
handler TimingTracer for Trace {
    state: start_times: Vec<(String, Instant)> = vec![]

    return(x) { x }

    span_enter(name) {
        start_times.push((name, Instant::now()));
        resume(())
    }

    span_exit(name) {
        if let Some((_, start)) = start_times.pop() {
            let elapsed = start.elapsed();
            println("[TIMING] {} took {:?}", name, elapsed);
        }
        resume(())
    }

    event(level, message, fields) {
        let fields_str = fields.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(" ");
        println("[{}] {} {}", level, message, fields_str);
        resume(())
    }
}
```

### 4.3 Silent Logger (for Testing)

```blood
handler SilentLogger for Logger {
    return(x) { x }

    log(_level, _message) {
        resume(())  // Do nothing
    }
}
```

---

## 5. Fiber/Concurrent Patterns

### 5.1 Fiber Effect

```blood
effect Fiber {
    op suspend<T>(future: Future<T>) -> T;
    op spawn<T>(f: fn() -> T / {Fiber}) -> FiberHandle<T>;
    op yield_();
}

fn fetch_data() / {Fiber, IO} -> Data {
    let response = perform Fiber.suspend(http_get("https://api.example.com/data"));
    response.json()
}
```

### 5.2 Parallel Execution

```blood
fn fetch_all(urls: Vec<String>) / {Fiber, IO} -> Vec<Response> {
    let handles: Vec<FiberHandle<Response>> = urls.iter()
        .map(|url| perform Fiber.spawn(|| http_get(url)))
        .collect();

    handles.iter()
        .map(|h| perform Fiber.suspend(h))
        .collect()
}
```

### 5.3 Rate Limiting Handler

```blood
handler RateLimiter(rate: i32, per: Duration) for Fiber {
    state: tokens: i32 = rate
    state: last_refill: Instant = Instant::now()

    return(x) { x }

    suspend(future) {
        // Refill tokens
        let now = Instant::now();
        if now - last_refill >= per {
            tokens = rate;
            last_refill = now;
        }

        // Wait for token
        while tokens == 0 {
            perform Fiber.yield_();
            // Check again after yield
        }

        tokens = tokens - 1;
        resume(future.suspend)
    }

    spawn(f) {
        resume(spawn_fiber(f))
    }

    yield_() {
        resume(())
    }
}
```

---

## 6. Effect Composition

### 6.1 Combining Multiple Effects

```blood
// Application with multiple effects
fn process_order(order_id: i32) / {
    State<OrderState>,
    Error<OrderError>,
    Logger,
    Fiber,
    IO
} -> Receipt {
    perform Logger.log(LogLevel::Info, "Processing order {}".format(order_id));

    let order = perform Fiber.suspend(fetch_order(order_id));

    if !order.is_valid() {
        perform Error.throw(OrderError::InvalidOrder);
    }

    perform State.put(OrderState::Processing);

    let receipt = perform Fiber.suspend(charge_card(order));

    perform State.put(OrderState::Complete);
    perform Logger.log(LogLevel::Info, "Order {} complete".format(order_id));

    receipt
}
```

### 6.2 Nested Handlers

```blood
fn run_order_processing() {
    with FiberRuntime handle {
        with TryCatch<OrderError> handle {
            with StateHandler(OrderState::Pending) handle {
                with ConsoleLogger handle {
                    process_order(12345)
                }
            }
        }
    }
}
```

### 6.3 Handler Composition Helper

```blood
// Compose handlers into a single runner
fn run_with_standard_handlers<T, E>(
    f: fn() -> T / {State<E>, Error<String>, Logger}
) -> Result<(T, E), String> {
    with TryCatch handle {
        with StateHandler(E::default()) handle {
            with ConsoleLogger handle {
                f()
            }
        }
    }
}
```

---

## 7. Handler Optimization

### 7.1 Tail-Resumptive Handlers

Tail-resumptive handlers are optimized to avoid continuation capture:

```blood
// GOOD: Tail-resumptive (resume is the last expression)
handler OptimizedState<T>(mut state: T) for State<T> {
    return(x) { x }

    get() {
        resume(state)  // Tail position - optimized
    }

    put(value) {
        state = value;
        resume(())     // Tail position - optimized
    }
}

// BAD: Non-tail-resumptive (work after resume)
handler UnoptimizedState<T>(mut state: T) for State<T> {
    return(x) { x }

    get() {
        let result = resume(state);  // Not in tail position
        println("Got state");        // This prevents optimization
        result
    }
}
```

### 7.2 Stateless Handlers

Stateless handlers use static allocation:

```blood
// GOOD: No state needed
handler PureLogger for Logger {
    return(x) { x }

    log(level, message) {
        println("[{}] {}", level, message);
        resume(())
    }
}

// LESS OPTIMAL: Unnecessary state
handler LoggerWithCounter for Logger {
    state: count: i32 = 0  // Prevents static optimization

    return(x) { x }

    log(level, message) {
        count = count + 1;
        println("[{}] {} (message #{})", level, message, count);
        resume(())
    }
}
```

### 7.3 Effect Polymorphism for Reusable Code

```blood
// Generic function that works with any effect set
fn map_with_effects<T, U, E>(
    items: Vec<T>,
    f: fn(T) -> U / E
) -> Vec<U> / E {
    items.into_iter().map(f).collect()
}

// Works with any effect combination
fn example() / {Logger, Error<String>} {
    let numbers = vec![1, 2, 3];
    let doubled = map_with_effects(numbers, |x| {
        perform Logger.log(LogLevel::Debug, "Processing {}".format(x));
        x * 2
    });
}
```

---

## 8. Testing with Effects

### 8.1 Mock Handlers

```blood
// Production handler
handler RealDatabase for Database {
    return(x) { x }

    query(sql) {
        let result = postgres_query(sql);
        resume(result)
    }
}

// Test handler with mock data
handler MockDatabase(responses: Map<String, QueryResult>) for Database {
    return(x) { x }

    query(sql) {
        let result = responses.get(sql).unwrap_or(QueryResult::empty());
        resume(result)
    }
}

#[test]
fn test_user_lookup() {
    let mock_data = hashmap! {
        "SELECT * FROM users WHERE id = 1" => QueryResult::row(User { id: 1, name: "Alice" })
    };

    let user = with MockDatabase(mock_data) handle {
        find_user(1)
    };

    assert_eq!(user.name, "Alice");
}
```

### 8.2 Recording Handler

```blood
// Handler that records all operations
handler RecordingLogger for Logger {
    state: logs: Vec<(LogLevel, String)> = vec![]

    return(x) { (x, logs) }

    log(level, message) {
        logs.push((level, message.clone()));
        resume(())
    }
}

#[test]
fn test_logging() {
    let (_, logs) = with RecordingLogger handle {
        do_work()
    };

    assert!(logs.contains((LogLevel::Info, "Starting work...")));
    assert!(logs.contains((LogLevel::Info, "Work complete")));
}
```

### 8.3 Assertion Handler

```blood
handler AssertingState<T: Eq>(expected_sequence: Vec<T>) for State<T> {
    state: index: usize = 0
    state: state: T = expected_sequence[0]

    return(x) {
        assert_eq!(index, expected_sequence.len() - 1, "Not all states visited");
        x
    }

    get() {
        resume(state.clone())
    }

    put(value) {
        index = index + 1;
        assert!(index < expected_sequence.len(), "Too many state changes");
        assert_eq!(value, expected_sequence[index], "Unexpected state");
        state = value;
        resume(())
    }
}
```

---

## 9. Anti-Patterns

### 9.1 Avoid: Deep Handler Nesting

```blood
// BAD: Deep nesting is hard to read
fn deeply_nested() {
    with Handler1 handle {
        with Handler2 handle {
            with Handler3 handle {
                with Handler4 handle {
                    with Handler5 handle {
                        do_work()
                    }
                }
            }
        }
    }
}

// BETTER: Factor out handler composition
fn run_with_handlers<T>(f: fn() -> T / E) -> T {
    with Handler1 handle {
        with Handler2 handle {
            with Handler3 handle {
                f()
            }
        }
    }
}

fn cleaner() {
    run_with_handlers(|| {
        with Handler4 handle {
            with Handler5 handle {
                do_work()
            }
        }
    })
}
```

### 9.2 Avoid: Overusing Effects

```blood
// BAD: Using effects for simple operations
effect Add {
    op add(a: i32, b: i32) -> i32;
}

fn over_effected() / {Add} -> i32 {
    perform Add.add(1, 2)  // Just use: 1 + 2
}

// GOOD: Use effects for actual side effects
fn appropriate_effects() / {Logger, IO} -> i32 {
    perform Logger.log(LogLevel::Debug, "Computing...");
    let result = 1 + 2;  // Pure computation
    perform Logger.log(LogLevel::Debug, "Result: {}".format(result));
    result
}
```

### 9.3 Avoid: Non-Resuming When Resume Expected

```blood
// BAD: Forgetting to resume
handler BrokenState<T>(mut state: T) for State<T> {
    return(x) { x }

    get() {
        state  // Missing resume! This returns from the handler, not to caller
    }

    put(value) {
        state = value;
        // Missing resume! Computation stops here
    }
}

// GOOD: Always resume (unless intentionally stopping)
handler CorrectState<T>(mut state: T) for State<T> {
    return(x) { x }

    get() {
        resume(state)
    }

    put(value) {
        state = value;
        resume(())
    }
}
```

### 9.4 Avoid: Capturing Linear Values in Multi-Shot Handlers

```blood
// BAD: Linear value captured in potentially multi-shot handler
fn dangerous() / {Choice} {
    let file = open_file("data.txt");  // Linear resource

    // If Choice handler resumes multiple times,
    // file will be used-after-close
    let choice = perform Choice.choose(vec![1, 2, 3]);

    file.write(choice);
    file.close();
}

// GOOD: Create resource inside handler scope
fn safe() / {Choice} {
    let choice = perform Choice.choose(vec![1, 2, 3]);

    let file = open_file("data.txt");
    file.write(choice);
    file.close();
}
```

---

## Summary

Key takeaways for effective effect usage:

1. **Design effects around capabilities**, not implementations
2. **Use tail-resumptive patterns** when possible for optimization
3. **Compose handlers** from the outside, not inside
4. **Test with mock handlers** for isolated unit tests
5. **Avoid deep nesting** by factoring out handler composition
6. **Be careful with linear types** in multi-shot handlers

For more details, see:
- [EFFECTS_TUTORIAL.md](./EFFECTS_TUTORIAL.md) - Effect basics
- [SPECIFICATION.md](../spec/SPECIFICATION.md) - Effect system specification
- [EFFECTS_CODEGEN.md](../spec/EFFECTS_CODEGEN.md) - Implementation details

---

*Last updated: 2026-01-14*

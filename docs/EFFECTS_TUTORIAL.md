# Algebraic Effects in Blood: A Tutorial

Algebraic effects are Blood's approach to managing side effects in a composable, type-safe way. This tutorial explains effects from the ground up.

## Why Effects?

Traditional approaches to side effects have trade-offs:

| Approach | Pros | Cons |
|----------|------|------|
| **Implicit side effects** | Easy to write | Hard to reason about, test |
| **Monads** | Composable | Complex types, monad transformers |
| **Dependency injection** | Testable | Boilerplate, runtime overhead |

**Algebraic effects** give you:
- Explicit effect annotations (know what a function does)
- Easy composition (no monad transformers)
- Testability (swap handlers)
- Performance (zero-cost when optimized)

## Effect Basics

### Defining an Effect

An effect declares operations that can be performed:

```blood
effect Logger {
    op log(level: LogLevel, message: String);
}

enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}
```

### Performing Effects

Use `perform` to invoke an effect operation:

```blood
fn process_data(data: [i32]) -> i32 / {Logger} {
    perform Logger::log(LogLevel::Info, "Processing " + data.len().to_string() + " items");

    let mut sum = 0;
    for item in data {
        sum = sum + item;
    }

    perform Logger::log(LogLevel::Debug, "Sum computed: " + sum.to_string());
    sum
}
```

The `/ {Logger}` annotation declares this function performs the `Logger` effect.

### Effect Propagation

Effects propagate through call chains:

```blood
fn helper() / {Logger} {
    perform Logger::log(LogLevel::Debug, "In helper");
}

fn caller() / {Logger} {
    helper();  // Logger effect propagates
    perform Logger::log(LogLevel::Info, "Done");
}
```

## Creating Handlers

Handlers provide implementations for effect operations.

### Basic Handler

```blood
handler ConsoleLogger for Logger {
    op log(level: LogLevel, message: String) {
        let prefix = match level {
            LogLevel::Debug => "[DEBUG] ",
            LogLevel::Info => "[INFO] ",
            LogLevel::Warn => "[WARN] ",
            LogLevel::Error => "[ERROR] ",
        };
        println(prefix + message);
        resume(())  // Continue execution
    }
}
```

### Using Handlers

Wrap effectful code with `with ... handle`:

```blood
fn main() {
    with ConsoleLogger handle {
        let result = process_data([1, 2, 3, 4, 5]);
        println("Result: " + result.to_string());
    }
}
```

## The `resume` Keyword

`resume` is crucial: it continues the computation from where the effect was performed.

### Without resume (terminating)

```blood
handler FailFastLogger for Logger {
    op log(level: LogLevel, message: String) {
        if level == LogLevel::Error {
            panic("Fatal error: " + message);
            // No resume - computation stops
        }
        println(message);
        resume(())
    }
}
```

### Returning values with resume

```blood
effect Ask<T> {
    op ask(prompt: String) -> T;
}

handler MockInput for Ask<String> {
    op ask(prompt: String) -> String {
        // Return a mock value to the caller
        resume("mock_value")
    }
}
```

## Stateful Handlers

Handlers can maintain state:

```blood
effect Counter {
    op increment();
    op get() -> i32;
}

handler LocalCounter for Counter {
    state: i32 = 0,

    op increment() {
        self.state = self.state + 1;
        resume(())
    }

    op get() -> i32 {
        resume(self.state)
    }
}
```

Initialize state when installing the handler:

```blood
fn main() {
    let final_count = with LocalCounter { state: 10 } handle {
        perform Counter::increment();
        perform Counter::increment();
        perform Counter::increment();
        perform Counter::get()
    };
    // final_count is 13
}
```

## Deep vs Shallow Handlers

Blood supports two handler styles:

### Deep Handlers (default)

Deep handlers remain installed for all nested effect operations:

```blood
deep handler DeepCounter for Counter {
    state: i32 = 0,

    op increment() {
        self.state = self.state + 1;
        resume(())  // Handler stays active
    }
}

fn count_things() / {Counter} {
    perform Counter::increment();  // Handled
    nested_count();
}

fn nested_count() / {Counter} {
    perform Counter::increment();  // Also handled by same handler
}
```

### Shallow Handlers

Shallow handlers handle only one operation, then must be reinstalled:

```blood
shallow handler ShallowCounter for Counter {
    state: i32 = 0,

    op increment() {
        self.state = self.state + 1;
        // Must re-wrap for further operations
        with ShallowCounter { state: self.state } handle {
            resume(())
        }
    }
}
```

Use shallow handlers for:
- Coroutines and generators
- One-shot continuations
- Fine-grained control flow

## Effect Composition

Multiple effects compose naturally:

```blood
effect Logger {
    op log(msg: String);
}

effect Database {
    op query(sql: String) -> [Row];
    op execute(sql: String) -> i32;
}

fn get_users() -> [User] / {Logger, Database} {
    perform Logger::log("Fetching users...");
    let rows = perform Database::query("SELECT * FROM users");
    perform Logger::log("Found " + rows.len().to_string() + " users");
    rows.map(|r| User::from_row(r))
}
```

Handle multiple effects:

```blood
fn main() {
    with ConsoleLogger handle {
        with PostgresDB { connection: conn } handle {
            let users = get_users();
            // ...
        }
    }
}
```

## Effect Polymorphism

Functions can be polymorphic over effects:

```blood
fn map_with_effects<T, U, E>(
    items: [T],
    f: fn(T) -> U / E
) -> [U] / E {
    let mut result = [];
    for item in items {
        result.push(f(item));
    }
    result
}
```

The `E` is an effect variable that can represent any effect row.

## Pure Functions

Declare a function performs no effects with `/ pure`:

```blood
fn add(a: i32, b: i32) -> i32 / pure {
    a + b
}

fn fibonacci(n: i32) -> i32 / pure {
    if n <= 1 { n }
    else { fibonacci(n - 1) + fibonacci(n - 2) }
}
```

Pure functions can be called from any context.

## Effect Inference

Blood can often infer effect signatures:

```blood
// Effect signature inferred as / {Logger}
fn greet(name: String) {
    perform Logger::log("Hello, " + name);
}
```

When the compiler infers effects, it reports them:

```
note: inferred effect signature: fn greet(name: String) / {Logger}
```

## Real-World Patterns

### Testing with Mock Handlers

```blood
// Production handler
handler ProductionHTTP for HTTP {
    op get(url: String) -> Response {
        resume(actual_http_get(url))
    }
}

// Test handler
handler MockHTTP for HTTP {
    responses: HashMap<String, Response>,

    op get(url: String) -> Response {
        let response = self.responses.get(url)
            .unwrap_or(Response::not_found());
        resume(response)
    }
}

#[test]
fn test_api_client() {
    let mocks = MockHTTP {
        responses: HashMap::from([
            ("/users", Response::ok("[{\"name\": \"Alice\"}]")),
        ])
    };

    with mocks handle {
        let users = fetch_users();
        assert_eq!(users.len(), 1);
    }
}
```

### Async Effects

```blood
effect Async {
    op spawn<T>(task: fn() -> T / Async) -> Handle<T>;
    op await<T>(handle: Handle<T>) -> T;
    op yield();
}

fn fetch_all(urls: [String]) -> [Response] / {Async, HTTP} {
    let handles = urls.map(|url| {
        perform Async::spawn(|| {
            perform HTTP::get(url)
        })
    });

    handles.map(|h| perform Async::await(h))
}
```

### Transaction Effects

```blood
effect Transaction {
    op begin();
    op commit();
    op rollback();
}

fn transfer_funds(from: Account, to: Account, amount: i64) / {Transaction, Database} {
    perform Transaction::begin();

    let from_balance = perform Database::query_one(
        "SELECT balance FROM accounts WHERE id = ?",
        [from.id]
    );

    if from_balance < amount {
        perform Transaction::rollback();
        return Err(InsufficientFunds);
    }

    perform Database::execute(
        "UPDATE accounts SET balance = balance - ? WHERE id = ?",
        [amount, from.id]
    );

    perform Database::execute(
        "UPDATE accounts SET balance = balance + ? WHERE id = ?",
        [amount, to.id]
    );

    perform Transaction::commit();
    Ok(())
}
```

## Error Handling with Effects

```blood
effect Fail<E> {
    op fail(error: E) -> !;  // Never returns
}

fn parse_config(text: String) -> Config / {Fail<ParseError>} {
    if text.is_empty() {
        perform Fail::fail(ParseError::EmptyInput);
    }
    // Parse logic...
}

// Handle by converting to Result
handler ToResult<E> for Fail<E> {
    op fail(error: E) -> ! {
        // Don't resume - return early with error
        return Err(error);
    }
}

fn main() {
    let result: Result<Config, ParseError> = with ToResult handle {
        Ok(parse_config(input))
    };
}
```

## Performance Considerations

1. **Handler inlining**: When the handler is known at compile time, effect operations can be inlined.

2. **Effect elimination**: Pure handlers (that just resume) can be eliminated entirely.

3. **Stack allocation**: Handler state that doesn't escape can be stack-allocated.

4. **Tail resumption**: When `resume` is in tail position, no stack frame is needed.

```blood
// This handler can be fully optimized away
handler IdentityLogger for Logger {
    op log(level: LogLevel, message: String) {
        resume(())  // No-op, optimized away
    }
}
```

## Best Practices

1. **Keep effects small and focused** - One effect per concern
2. **Use effect inference during development** - Add explicit signatures for public APIs
3. **Test with mock handlers** - Makes testing pure
4. **Prefer deep handlers** - Unless you need fine-grained control
5. **Document effect semantics** - What does `resume` mean for your operations?

## Next Steps

- See [EFFECTS.md](../EFFECTS.md) for the complete effect system specification
- Explore [examples/algebraic_effects.blood](../examples/algebraic_effects.blood) for more examples
- Read about [handler semantics](../EFFECTS.md#handler-semantics) for advanced patterns

# Blood vs Unison: Content-Addressed Languages Compared

**Purpose**: Compare Blood and Unison, two languages featuring content-addressed code.

Both Blood and Unison use content addressing (hashing code by content) as a foundational feature. This comparison examines how each uses this concept and their different design goals.

---

## Executive Summary

| Aspect | Blood | Unison |
|--------|-------|--------|
| **Primary Focus** | Systems programming | Distributed computing |
| **Content Addressing** | Module-level hashing | Definition-level hashing |
| **Memory Model** | Generational references | Garbage collection |
| **Effect System** | Algebraic effects | Abilities (similar) |
| **Typing** | Static, strict | Static, structural |
| **Distributed Support** | Via effects | Built-in |
| **Ecosystem** | Early | Growing |

---

## 1. Content-Addressed Code

Both languages hash code by content, but with different granularity and purpose.

### Unison: Definition-Level Hashing

Unison hashes every definition individually:

```unison
-- Each definition has a unique hash
square : Nat -> Nat
square x = x * x
-- Hash: #abc123...

-- References use hashes, not names
double : Nat -> Nat
double x = x + x
-- Hash: #def456...

-- Renaming doesn't change the hash
multiply : Nat -> Nat -> Nat
multiply x y = x * y
-- Hash remains #ghi789... regardless of name
```

**Unison Content Addressing:**
- Every function/type has a unique hash
- Hashes based on syntax tree structure
- Names are metadata, not identity
- Renaming never breaks code
- No build step - code is always "compiled"

### Blood: Module-Level Hashing

Blood hashes at module granularity:

```blood
// Module hash: includes all public definitions
// module_hash: #abc123...

pub struct Point {
    x: f64,
    y: f64,
}

pub fn distance(a: Point, b: Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

// Dependencies tracked by module hash
use geometry@#abc123::Point;
```

**Blood Content Addressing:**
- Modules have content hashes
- Dependencies pin to specific versions via hash
- Enables deterministic builds
- Supports verified supply chain
- Incremental compilation by hash comparison

### Content Addressing Comparison

| Aspect | Unison | Blood |
|--------|--------|-------|
| Granularity | Definition | Module |
| Rename safety | Perfect | Partial (within module) |
| Hash storage | Codebase database | File system / registry |
| Version pinning | Implicit (by hash) | Explicit (in manifest) |
| Build system | None needed | Incremental |
| Distribution | Unison Share | Package registry |

---

## 2. Effect/Ability Systems

Both languages have effect-like systems, but Unison calls them "abilities."

### Unison: Abilities

```unison
-- Ability declaration
ability Store v where
  get : v
  put : v -> ()

-- Using abilities
increment : '{Store Nat} ()
increment = '(let x = get; put (x + 1))

-- Handler
runStore : v -> '{Store v} a -> a
runStore initial computation =
  handle !computation with cases
    { pure a } -> a
    { get -> resume } -> runStore initial (resume initial)
    { put v -> resume } -> runStore v (resume ())

-- Usage
> runStore 0 increment
1
```

**Unison Ability Features:**
- `ability` keyword for declaration
- Delayed computation with `'`
- `handle...with` for handlers
- Structural typing for abilities
- First-class continuations

### Blood: Effects

```blood
// Effect declaration
effect Store<V> {
    fn get() -> V;
    fn put(value: V) -> ();
}

// Using effects
fn increment() with Store<u64> {
    let x = do Store.get();
    do Store.put(x + 1);
}

// Handler
handler StateStore<V>: Store<V> {
    state: V,

    fn get() -> V {
        resume(self.state.clone())
    }

    fn put(value: V) -> () {
        self.state = value;
        resume(())
    }
}

// Usage
fn main() {
    let mut store = StateStore { state: 0u64 };
    try {
        increment()
    } with store;
    println!("{}", store.state);  // 1
}
```

**Blood Effect Features:**
- `effect` keyword for declaration
- `do Effect.operation()` for invocation
- `try`/`with` for handlers
- Nominal typing for effects
- Stateful handlers

### Effect System Comparison

| Aspect | Unison | Blood |
|--------|--------|-------|
| Declaration | `ability Name where` | `effect Name { }` |
| Invocation | Direct call | `do Effect.op()` |
| Handler | `handle...with cases` | `try { } with handler` |
| Typing | Structural | Nominal |
| Handler state | Pattern matching | Struct fields |
| Delayed computation | `'expr` | Closures |

---

## 3. Type Systems

### Unison: Structural Types

Unison uses structural typing throughout:

```unison
-- Type aliases are structural
type Person = { name: Text, age: Nat }

-- Same structure = same type
type Employee = { name: Text, age: Nat }
-- Person and Employee are the SAME type

-- Algebraic data types
type Optional a = None | Some a

-- Pattern matching
getName : Optional Person -> Text
getName = cases
  None -> "Unknown"
  Some p -> p.name
```

### Blood: Nominal Types

Blood uses nominal typing:

```blood
// Structs are nominally typed
struct Person {
    name: String,
    age: u64,
}

// Different type, even with same structure
struct Employee {
    name: String,
    age: u64,
}
// Person != Employee (different names)

// Algebraic data types
enum Option<T> {
    None,
    Some(T),
}

// Pattern matching
fn get_name(opt: Option<Person>) -> String {
    match opt {
        Option::None => "Unknown".to_string(),
        Option::Some(p) => p.name,
    }
}
```

### Type System Comparison

| Feature | Unison | Blood |
|---------|--------|-------|
| Type equivalence | Structural | Nominal |
| Generics | Yes | Yes |
| Type inference | Extensive | Limited |
| Type classes | Yes (abilities) | Traits |
| Higher-kinded types | Yes | Planned |
| Row types | No | No |

---

## 4. Memory Management

### Unison: Garbage Collection

Unison uses traditional GC:

```unison
-- Allocate freely
createList : Nat -> [Nat]
createList n =
  List.range 0 n

-- GC handles cleanup
process : () -> ()
process _ =
  let data = createList 1000000
  -- Process data...
  -- GC collects when unreachable
```

**Unison Memory:**
- Managed runtime
- GC optimized for functional patterns
- No manual memory management
- Suitable for distributed/cloud workloads

### Blood: Generational References

Blood uses generation-based safety:

```blood
// Deterministic allocation
fn create_list(n: u64) -> Vec<u64> {
    (0..n).collect()
}

// Deterministic cleanup
fn process() {
    let data = create_list(1_000_000);
    // Process data...
}  // data freed here, deterministically
```

**Blood Memory:**
- No GC pauses
- Deterministic deallocation
- 128-bit pointers with generation
- Suitable for real-time/embedded

### Memory Comparison

| Aspect | Unison | Blood |
|--------|--------|-------|
| Model | Garbage collection | Generational references |
| Determinism | No | Yes |
| Pauses | Yes (small) | No |
| Overhead | GC metadata | 128-bit pointers |
| Use case | Distributed | Systems/real-time |

---

## 5. Distributed Computing

### Unison: Built-In Distribution

Unison was designed for distributed computing:

```unison
-- Remote execution is first-class
ability Remote where
  at : Location -> '{Remote} a -> a

-- Transfer computation to another node
distributedSearch : Location -> Text -> '{Remote, IO} [Result]
distributedSearch server query =
  '(Remote.at server '(searchLocal query))

-- Serialization is automatic (code is hashed)
-- No need for protobuf/JSON - code IS the schema
```

**Unison Distribution:**
- Code moves with data (same hash = same code)
- No serialization schemas needed
- Built-in remote execution primitives
- Unison Share for code distribution

### Blood: Distribution via Effects

Blood handles distribution through effects:

```blood
effect Remote {
    fn at<T: Serialize>(location: Location, task: fn() -> T) -> T;
}

fn distributed_search(server: Location, query: String) -> Vec<Result> with Remote {
    do Remote.at(server, || search_local(query))
}

// Handler implements network protocol
handler HttpRemote: Remote {
    fn at<T: Serialize>(location: Location, task: fn() -> T) -> T {
        // Serialize task, send to location, await result
        let result = http_call(&location, serialize_task(&task));
        resume(deserialize(&result))
    }
}
```

**Blood Distribution:**
- Distribution is not built-in
- Implemented via effects + handlers
- Requires explicit serialization
- More flexible but more work

### Distribution Comparison

| Aspect | Unison | Blood |
|--------|--------|-------|
| Built-in | Yes | No |
| Code mobility | Automatic (hashes) | Manual (serialization) |
| Serialization | Implicit | Explicit |
| Remote execution | `Remote.at` | Via effect handlers |
| Use case | Cloud/distributed | Local/systems |

---

## 6. Development Experience

### Unison: Codebase Manager (UCM)

Unison has a unique development model:

```
.> add square           -- Add definition to codebase
.> find square          -- Search by name
.> view #abc123         -- View definition by hash
.> mv square square'    -- Rename (doesn't break anything)
.> branch feature       -- Create branch
.> merge feature main   -- Merge (structural)
```

**Unison UCM:**
- REPL-based development
- No files (code stored in database)
- Rename-safe refactoring
- Structural merge (no text conflicts)
- Scratch files for development

### Blood: Traditional Files + Tools

Blood uses traditional file-based development:

```bash
$ bloodc build          # Compile
$ bloodc test           # Run tests
$ bloodc fmt            # Format code
$ blood-lsp             # Language server

# Standard file-based workflow
$ git add src/
$ git commit -m "feat: add feature"
```

**Blood Development:**
- File-based source code
- Standard VCS integration
- LSP for IDE support
- Familiar workflow

### Development Comparison

| Aspect | Unison | Blood |
|--------|--------|-------|
| Code storage | Database | Files |
| VCS | UCM + namespace branches | Git |
| IDE | UCM + basic | LSP + extensions |
| Refactoring | Rename-safe | Standard |
| Learning curve | New paradigm | Familiar |

---

## 7. Use Case Comparison

### Unison Excels At:

1. **Distributed Systems**
   - Microservices that share code
   - Cloud functions
   - Distributed data processing

2. **Functional Programming**
   - Pure functional style
   - Immutable data
   - Pattern matching

3. **Refactoring Safety**
   - Large codebases with frequent renames
   - Teams with different naming conventions
   - Code archaeology

4. **Code Sharing**
   - Unison Share ecosystem
   - Easy code discovery
   - Automatic compatibility

### Blood Excels At:

1. **Systems Programming**
   - Low-level code
   - Embedded systems
   - Real-time applications

2. **Deterministic Behavior**
   - Predictable memory
   - No GC pauses
   - Timing-sensitive code

3. **Effect-Heavy Applications**
   - Compilers and interpreters
   - Complex control flow
   - Testable business logic

4. **Traditional Development**
   - File-based workflow
   - Git integration
   - IDE support

---

## 8. Code Examples

### HTTP Request Handling

**Unison:**
```unison
ability Http where
  get : Url -> Response

fetchUsers : '{Http, Throw HttpError} [User]
fetchUsers = '(
  let resp = Http.get (Url.parse "https://api.example.com/users")
  case Json.decode resp.body of
    Left err -> Throw.throw (ParseError err)
    Right users -> users
)

-- Handler using IO
runHttp : '{Http, Throw e} a ->{IO, Throw e} a
runHttp computation = handle !computation with cases
  { Http.get url -> resume } ->
    let resp = IO.fetch url
    resume resp
```

**Blood:**
```blood
effect Http {
    fn get(url: String) -> Response;
}

fn fetch_users() -> Vec<User> with Http, Fail {
    let resp = do Http.get("https://api.example.com/users".to_string());

    match json::decode::<Vec<User>>(&resp.body) {
        Ok(users) => users,
        Err(e) => do Fail.fail(format!("Parse error: {}", e)),
    }
}

handler RealHttp: Http {
    fn get(url: String) -> Response {
        let resp = reqwest::blocking::get(&url).unwrap();
        resume(Response::from(resp))
    }
}
```

### State Management

**Unison:**
```unison
ability State s where
  get : s
  put : s -> ()
  modify : (s -> s) -> ()

counter : '{State Nat} Nat
counter = '(
  modify (x -> x + 1)
  modify (x -> x + 1)
  get
)

runState : s -> '{State s} a -> (s, a)
runState initial computation = handle !computation with cases
  { pure a } -> (initial, a)
  { State.get -> resume } -> runState initial (resume initial)
  { State.put s -> resume } -> runState s (resume ())
  { State.modify f -> resume } -> runState (f initial) (resume ())
```

**Blood:**
```blood
effect State<S> {
    fn get() -> S;
    fn put(s: S) -> ();
    fn modify(f: fn(S) -> S) -> ();
}

fn counter() -> u64 with State<u64> {
    do State.modify(|x| x + 1);
    do State.modify(|x| x + 1);
    do State.get()
}

handler StateHandler<S: Clone>: State<S> {
    value: S,

    fn get() -> S {
        resume(self.value.clone())
    }

    fn put(s: S) -> () {
        self.value = s;
        resume(())
    }

    fn modify(f: fn(S) -> S) -> () {
        self.value = f(self.value.clone());
        resume(())
    }
}
```

---

## 9. Ecosystem Status

### Unison

- **Status**: Active development by Unison Computing
- **Community**: Growing (Discord, forums)
- **Packages**: Unison Share (growing library)
- **Documentation**: Good (tutorials, videos)
- **Companies**: Unison Computing (commercial)

### Blood

- **Status**: Early development
- **Community**: Small (GitHub)
- **Packages**: Standard library only
- **Documentation**: Specification complete
- **Companies**: None yet

---

## 10. Honest Assessment

### Unison's Advantages Over Blood

1. **True Content Addressing**: Definition-level hashing is revolutionary
2. **Distribution**: Built-in distributed computing
3. **Refactoring**: Rename-safe by design
4. **Ecosystem**: More packages, better tooling
5. **Innovation**: Novel approach to programming

### Blood's Advantages Over Unison

1. **Systems Programming**: Better for low-level code
2. **Determinism**: No GC, predictable behavior
3. **Familiar Workflow**: File-based, Git-compatible
4. **Performance**: Systems-level performance
5. **Effect Handlers**: Stateful, flexible handlers

### Philosophical Difference

- **Unison**: Reimagine programming from first principles
- **Blood**: Evolve existing paradigms with new safety features

---

## 11. Choosing Between Them

### Choose Unison When:

- Building distributed systems
- Functional programming paradigm fits
- Code sharing/reuse is critical
- Refactoring safety matters most
- Willing to learn new workflow

### Choose Blood When:

- Building systems software
- Deterministic behavior required
- Team prefers familiar tooling
- Performance/real-time constraints
- Effect handling is central

### Neither When:

- Need mature ecosystem today
- Team needs extensive training
- Standard industry tooling required

---

## Conclusion

Unison and Blood share the vision that code should be identified by content, not by name/location. They apply this vision to different domains:

- **Unison**: Revolutionary approach to distributed functional programming
- **Blood**: Evolutionary approach to systems programming with effects

Both represent important advances in programming language design. Unison pushes the boundaries of what's possible; Blood makes safety practical for systems programming.

For developers interested in content addressing:
- Unison offers the purest vision
- Blood offers the most practical systems programming focus

Both are worth watching as they mature.

---

*This comparison reflects the current state of both languages. Both are actively evolving.*

#!/usr/bin/env python3
"""Blood codegen stress fuzzer.

Generates programs that PASS type-checking, then compiles and runs them
to find codegen crashes, LLVM IR errors, segfaults, and wrong output.

Targets the five highest-risk codegen paths:
  1. Effect handlers with snapshots
  2. Nested closures
  3. Generic monomorphization in handlers
  4. Regions with effect suspension
  5. Pattern matching on generic enums

Usage:
    python3 tools/codegen_fuzz.py [count] [--compiler PATH]
"""

import random
import subprocess
import sys
import os
import argparse
import tempfile
import shutil
from typing import List, Tuple, Optional

COMPILER = "src/selfhost/build/first_gen"

# ─── Template library ─────────────────────────────────────────────────────
# Each template is a (program_text, expected_exit_code) tuple.
# Programs are designed to type-check AND compile.

def t_basic_effect(depth=1) -> Tuple[str, int]:
    """Effect handler with varying nesting depth."""
    effects = []
    handlers = []
    performs = []
    handler_opens = []
    handler_closes = []

    for i in range(depth):
        ename = f"Eff{i}"
        hname = f"H{i}"
        opname = f"op{i}"
        effects.append(f"""effect {ename} {{
    op {opname}(x: i32) -> i32;
}}""")
        handlers.append(f"""deep handler {hname} for {ename} {{
    return(x) {{ x }}
    op {opname}(x) {{ resume(x + {i + 1}) }}
}}""")
        handler_opens.append(f"with {hname} {{}} handle {{")
        handler_closes.append("}")
        performs.append(f"let v{i}: i32 = perform {ename}.{opname}(v{i-1 if i > 0 else 'start'});")

    body = "\n        ".join(performs)
    opens = "\n        ".join(handler_opens)
    closes = " ".join(handler_closes)

    expected = 10
    for i in range(depth):
        expected += (i + 1)

    prog = "\n\n".join(effects) + "\n\n" + "\n\n".join(handlers) + f"""

fn main() -> i32 {{
    let result: i32 = {opens}
        let vstart: i32 = 10;
        {body}
        v{depth - 1}
    {closes};
    if result == {expected} {{ 0 }} else {{ 1 }}
}}
"""
    return (prog, 0)


def t_nested_closures(depth=2) -> Tuple[str, int]:
    """Closures nested N levels deep."""
    inner = "base"
    for i in range(depth):
        inner = f"|x{i}: i32| -> i32 {{ ({inner}) + x{i} }}"

    calls = ""
    val = 1
    total = 0
    for i in range(depth):
        calls += f"    let f{i} = {inner if i == 0 else f'f{i-1}'};\n"
        if i == 0:
            calls = f"    let base: i32 = 10;\n    let f0 = |x0: i32| -> i32 {{ base + x0 }};\n"
            total = 10 + 1
        else:
            pass

    # Simpler approach: just nest directly
    prog = f"""fn main() -> i32 {{
    let base: i32 = 10;
    let f1 = |a: i32| -> i32 {{ base + a }};
    let f2 = |b: i32| -> i32 {{ f1(b) + b }};
"""
    if depth >= 3:
        prog += "    let f3 = |c: i32| -> i32 { f2(c) + c };\n"
    if depth >= 4:
        prog += "    let f4 = |d: i32| -> i32 { f3(d) + d };\n"

    last = f"f{min(depth, 4)}"
    prog += f"""    let result: i32 = {last}(5);
    if result > 0 {{ 0 }} else {{ 1 }}
}}
"""
    return (prog, 0)


def t_generic_struct_methods() -> Tuple[str, int]:
    """Generic struct with impl methods exercising monomorphization."""
    prog = """struct Pair<A, B> {
    first: A,
    second: B,
}

impl<A, B> Pair<A, B> {
    fn new(a: A, b: B) -> Pair<A, B> {
        Pair { first: a, second: b }
    }
}

fn get_first<A, B>(p: &Pair<A, B>) -> A {
    p.first
}

fn get_second<A, B>(p: &Pair<A, B>) -> B {
    p.second
}

fn main() -> i32 {
    let p1: Pair<i32, i32> = Pair.new(42, 99);
    let p2: Pair<i32, bool> = Pair.new(10, true);
    let p3: Pair<bool, i32> = Pair.new(false, 77);
    let a: i32 = get_first(&p1);
    let b: i32 = get_second(&p1);
    let c: i32 = get_first(&p2);
    let d: i32 = get_second(&p3);
    if a == 42 && b == 99 && c == 10 && d == 77 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_enum_pattern_matching() -> Tuple[str, int]:
    """Complex pattern matching on enums with payloads."""
    prog = """enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Triangle(f64, f64, f64),
    Point,
}

fn classify(s: &Shape) -> i32 {
    match s {
        &Shape.Circle(r) => {
            if r > 0.0 { 1 } else { 0 }
        }
        &Shape.Rect(w, h) => {
            if w == h { 2 } else { 3 }
        }
        &Shape.Triangle(a, b, c) => {
            if a == b && b == c { 4 } else { 5 }
        }
        &Shape.Point => 6,
    }
}

fn main() -> i32 {
    let c: Shape = Shape.Circle(3.14);
    let r: Shape = Shape.Rect(5.0, 5.0);
    let t: Shape = Shape.Triangle(3.0, 4.0, 5.0);
    let p: Shape = Shape.Point;
    let r1: i32 = classify(&c);
    let r2: i32 = classify(&r);
    let r3: i32 = classify(&t);
    let r4: i32 = classify(&p);
    if r1 == 1 && r2 == 2 && r3 == 5 && r4 == 6 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_region_with_refs() -> Tuple[str, int]:
    """Region allocation with references."""
    prog = """struct Point {
    x: i32,
    y: i32,
}

fn distance_sq(p: &Point) -> i32 {
    p.x * p.x + p.y * p.y
}

fn main() -> i32 {
    let mut total: i32 = 0;
    region {
        let p1: Point = Point { x: 3, y: 4 };
        let p2: Point = Point { x: 5, y: 12 };
        total = distance_sq(&p1) + distance_sq(&p2);
    }
    if total == 25 + 169 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_effect_with_handler_state() -> Tuple[str, int]:
    """Effect handler with mutable state."""
    prog = """effect Counter {
    op increment() -> ();
    op get_count() -> i32;
}

deep handler CounterImpl for Counter {
    let mut count: i32 = 0
    return(x) { x }
    op increment() {
        count = count + 1;
        resume(())
    }
    op get_count() {
        resume(count)
    }
}

fn count_to_five() -> i32 / {Counter} {
    perform Counter.increment();
    perform Counter.increment();
    perform Counter.increment();
    perform Counter.increment();
    perform Counter.increment();
    perform Counter.get_count()
}

fn main() -> i32 {
    let result: i32 = with CounterImpl { count: 0 } handle {
        count_to_five()
    };
    if result == 5 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_trait_dispatch() -> Tuple[str, int]:
    """Trait with multiple impls exercising dispatch."""
    prog = """trait Describable {
    fn describe(&self) -> i32;
}

struct Cat {
    lives: i32,
}

struct Dog {
    tricks: i32,
}

impl Describable for Cat {
    fn describe(&self) -> i32 {
        self.lives
    }
}

impl Describable for Dog {
    fn describe(&self) -> i32 {
        self.tricks
    }
}

fn report_cat(c: &Cat) -> i32 {
    c.describe()
}

fn report_dog(d: &Dog) -> i32 {
    d.describe()
}

fn main() -> i32 {
    let c: Cat = Cat { lives: 9 };
    let d: Dog = Dog { tricks: 7 };
    let r1: i32 = report_cat(&c);
    let r2: i32 = report_dog(&d);
    if r1 == 9 && r2 == 7 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_vec_operations() -> Tuple[str, int]:
    """Vec with push, index, iteration."""
    prog = """fn main() -> i32 {
    let mut v: Vec<i32> = Vec.new();
    v.push(10);
    v.push(20);
    v.push(30);
    v.push(40);
    v.push(50);
    let mut sum: i32 = 0;
    for x in &v {
        sum = sum + *x;
    }
    if sum == 150 && v.len() == 5 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_string_operations() -> Tuple[str, int]:
    """String building and slicing."""
    prog = """fn main() -> i32 {
    let mut s: String = String.new();
    s.push_str("hello");
    s.push_str(" ");
    s.push_str("world");
    let slice: &str = s.as_str();
    let len: i64 = s.len();
    if len == 11 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_hashmap_operations() -> Tuple[str, int]:
    """HashMap insert, get, contains."""
    prog = """fn main() -> i32 {
    let mut m: HashMap<i32, i32> = HashMap.new();
    m.insert(1, 100);
    m.insert(2, 200);
    m.insert(3, 300);
    let a: i32 = *m.get(&1);
    let b: i32 = *m.get(&2);
    if a == 100 && b == 200 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_for_loop_patterns() -> Tuple[str, int]:
    """Various for loop patterns."""
    prog = """fn main() -> i32 {
    let mut sum: i32 = 0;
    for i in 0..10 {
        sum = sum + i;
    }
    if sum == 45 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_recursive_enum() -> Tuple[str, int]:
    """Recursive enum (linked list)."""
    prog = """enum List {
    Cons(i32, Box<List>),
    Nil,
}

fn sum_list(l: &List) -> i32 {
    match l {
        &List.Cons(val, ref rest) => val + sum_list(rest),
        &List.Nil => 0,
    }
}

fn main() -> i32 {
    let l: List = List.Cons(1, Box.new(List.Cons(2, Box.new(List.Cons(3, Box.new(List.Nil))))));
    let s: i32 = sum_list(&l);
    if s == 6 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_where_clauses() -> Tuple[str, int]:
    """Generic function with where clauses."""
    prog = """trait Addable {
    fn add_val(&self, other: &Self) -> Self;
}

impl Addable for i32 {
    fn add_val(&self, other: &i32) -> i32 {
        *self + *other
    }
}

fn sum_pair<T>(a: &T, b: &T) -> T where T: Addable {
    a.add_val(b)
}

fn main() -> i32 {
    let a: i32 = 40;
    let b: i32 = 2;
    let result: i32 = sum_pair(&a, &b);
    if result == 42 { 0 } else { 1 }
}
"""
    return (prog, 0)


def t_multi_effect_composition() -> Tuple[str, int]:
    """Multiple effects composed in one function."""
    prog = """effect Logger {
    op log(msg: i32) -> ();
}

effect State {
    op get() -> i32;
    op set(val: i32) -> ();
}

deep handler LogHandler for Logger {
    return(x) { x }
    op log(msg) {
        println_int(msg);
        resume(())
    }
}

deep handler StateHandler for State {
    let mut val: i32 = 0
    return(x) { x }
    op get() { resume(val) }
    op set(new_val) { val = new_val; resume(()) }
}

fn stateful_logging() -> i32 / {Logger, State} {
    perform State.set(10);
    let v: i32 = perform State.get();
    perform Logger.log(v);
    perform State.set(v + 5);
    perform State.get()
}

fn main() -> i32 {
    let result: i32 = with LogHandler {} handle {
        with StateHandler { val: 0 } handle {
            stateful_logging()
        }
    };
    if result == 15 { 0 } else { 1 }
}
"""
    return (prog, 0)


# ─── Mutators ──────────────────────────────────────────────────────────────

def mutate_program(prog: str) -> str:
    """Apply a random mutation to a well-typed program."""
    lines = prog.split('\n')
    strategy = random.randint(0, 5)

    if strategy == 0 and len(lines) > 5:
        # Duplicate a function body line
        body_lines = [i for i, l in enumerate(lines) if l.strip().startswith('let ')]
        if body_lines:
            idx = random.choice(body_lines)
            lines.insert(idx + 1, lines[idx])

    elif strategy == 1:
        # Change an integer literal
        for i, line in enumerate(lines):
            if any(c.isdigit() for c in line) and 'fn ' not in line:
                new_line = ''
                j = 0
                while j < len(line):
                    if line[j].isdigit():
                        start = j
                        while j < len(line) and line[j].isdigit():
                            j += 1
                        new_line += str(random.randint(0, 1000))
                    else:
                        new_line += line[j]
                        j += 1
                lines[i] = new_line
                break

    elif strategy == 2:
        # Add an extra perform call in effect context
        for i, line in enumerate(lines):
            if 'perform ' in line:
                lines.insert(i, line)
                break

    elif strategy == 3:
        # Swap two non-empty body lines
        body_lines = [i for i, l in enumerate(lines)
                      if l.strip() and not l.strip().startswith(('fn ', 'struct ', 'enum ',
                          'effect ', 'handler ', 'trait ', 'impl ', '}', '{', '//'))]
        if len(body_lines) >= 2:
            a, b = random.sample(body_lines, 2)
            lines[a], lines[b] = lines[b], lines[a]

    elif strategy == 4:
        # Add a region block around a let binding
        for i, line in enumerate(lines):
            if line.strip().startswith('let ') and 'fn ' not in lines[max(0,i-1)]:
                indent = len(line) - len(line.lstrip())
                lines[i] = ' ' * indent + 'region {\n' + line + '\n' + ' ' * indent + '}'
                break

    elif strategy == 5:
        # Wrap an expression in a closure call
        for i, line in enumerate(lines):
            if '= ' in line and line.strip().startswith('let ') and ';' in line:
                parts = line.split('= ', 1)
                if len(parts) == 2:
                    val = parts[1].rstrip(';').strip()
                    if val and not val.startswith('|') and not val.startswith('perform'):
                        lines[i] = parts[0] + '= (|| -> i32 { ' + val + ' })();'
                        break

    return '\n'.join(lines)


# ─── Fuzzer engine ─────────────────────────────────────────────────────────

TEMPLATES = [
    t_basic_effect,
    lambda: t_basic_effect(2),
    lambda: t_basic_effect(3),
    t_nested_closures,
    lambda: t_nested_closures(3),
    t_generic_struct_methods,
    t_enum_pattern_matching,
    t_region_with_refs,
    t_effect_with_handler_state,
    t_trait_dispatch,
    t_vec_operations,
    t_string_operations,
    t_hashmap_operations,
    t_for_loop_patterns,
    t_recursive_enum,
    t_where_clauses,
    t_multi_effect_composition,
]


def run_compiler(compiler: str, args: List[str], input_file: str, timeout_s: int = 30) -> Tuple[int, str, str]:
    """Run compiler and return (exit_code, stdout, stderr)."""
    try:
        result = subprocess.run(
            [compiler] + args + [input_file],
            capture_output=True, text=True, timeout=timeout_s
        )
        return (result.returncode, result.stdout, result.stderr)
    except subprocess.TimeoutExpired:
        return (124, "", "TIMEOUT")
    except Exception as e:
        return (999, "", str(e))


def classify_result(code: int, stderr: str) -> str:
    """Classify a compiler/runtime result."""
    if code == 0:
        return "pass"
    if code == 124:
        return "hang"
    if code >= 128:
        sig = code - 128
        if sig == 11:
            return "segfault"
        elif sig == 6:
            return "abort"
        return f"signal-{sig}"
    if "llc" in stderr.lower() or "undefined" in stderr.lower():
        return "llc-error"
    if "panic" in stderr.lower():
        return "panic"
    if "error" in stderr.lower():
        return "compile-error"
    return "error"


def fuzz(count: int, compiler: str, findings_dir: str):
    """Main fuzzing loop."""
    os.makedirs(findings_dir, exist_ok=True)
    tmpdir = tempfile.mkdtemp(prefix="blood_fuzz_")

    stats = {"check_pass": 0, "check_fail": 0, "build_pass": 0,
             "build_crash": 0, "run_pass": 0, "run_crash": 0,
             "run_wrong": 0, "llc_error": 0, "hang": 0}
    findings = []

    try:
        for i in range(count):
            # Generate a program from a template
            template = random.choice(TEMPLATES)
            prog, expected_exit = template()

            # Optionally mutate it
            if random.random() < 0.4:
                prog = mutate_program(prog)

            # Write to temp file
            src_path = os.path.join(tmpdir, f"fuzz_{i}.blood")
            with open(src_path, "w") as f:
                f.write(prog)

            # Phase 1: Type-check
            code, _, stderr = run_compiler(compiler, ["check"], src_path, timeout_s=10)
            if code != 0:
                stats["check_fail"] += 1
                continue
            stats["check_pass"] += 1

            # Phase 2: Build
            code, _, stderr = run_compiler(compiler, ["build"], src_path, timeout_s=30)
            result = classify_result(code, stderr)

            if result in ("segfault", "abort", "panic", "hang") or result.startswith("signal"):
                stats["build_crash"] += 1
                finding_path = os.path.join(findings_dir, f"build_{result}_{i}.blood")
                shutil.copy2(src_path, finding_path)
                findings.append(("BUILD", result, finding_path))
                print(f"  FINDING: BUILD {result}: {finding_path}", file=sys.stderr)
                continue
            elif result == "llc-error":
                stats["llc_error"] += 1
                finding_path = os.path.join(findings_dir, f"llc_error_{i}.blood")
                shutil.copy2(src_path, finding_path)
                with open(finding_path + ".stderr", "w") as f:
                    f.write(stderr)
                findings.append(("LLC", result, finding_path))
                print(f"  FINDING: LLC error: {finding_path}", file=sys.stderr)
                continue
            elif code != 0:
                stats["check_fail"] += 1  # build failed for other reasons
                continue

            stats["build_pass"] += 1

            # Phase 3: Run the compiled binary
            bin_path = src_path.replace(".blood", "")
            if not os.path.exists(bin_path):
                # Try build/ directory
                base = os.path.basename(src_path).replace(".blood", "")
                bin_path = os.path.join(os.path.dirname(src_path), "build", "debug", base)

            if os.path.exists(bin_path):
                code, stdout, stderr = run_compiler(bin_path, [], "", timeout_s=10)
                result = classify_result(code, stderr)

                if result in ("segfault", "abort", "panic", "hang") or result.startswith("signal"):
                    stats["run_crash"] += 1
                    finding_path = os.path.join(findings_dir, f"run_{result}_{i}.blood")
                    shutil.copy2(src_path, finding_path)
                    findings.append(("RUN", result, finding_path))
                    print(f"  FINDING: RUN {result}: {finding_path}", file=sys.stderr)
                elif code != expected_exit:
                    stats["run_wrong"] += 1
                    finding_path = os.path.join(findings_dir, f"wrong_output_{i}.blood")
                    shutil.copy2(src_path, finding_path)
                    findings.append(("WRONG", f"exit={code}, expected={expected_exit}", finding_path))
                    print(f"  FINDING: WRONG OUTPUT (exit={code}): {finding_path}", file=sys.stderr)
                else:
                    stats["run_pass"] += 1

            # Progress
            if (i + 1) % 25 == 0:
                total_findings = len(findings)
                print(f"  [{i+1}/{count}] check={stats['check_pass']} build={stats['build_pass']} "
                      f"run={stats['run_pass']} findings={total_findings}", file=sys.stderr)

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)

    # Report
    print(f"\n{'='*60}")
    print(f"Codegen Fuzzing Results ({count} programs generated)")
    print(f"{'='*60}")
    print(f"  Type-check pass:  {stats['check_pass']}")
    print(f"  Type-check fail:  {stats['check_fail']}")
    print(f"  Build pass:       {stats['build_pass']}")
    print(f"  Build crash:      {stats['build_crash']}")
    print(f"  LLC error:        {stats['llc_error']}")
    print(f"  Run pass:         {stats['run_pass']}")
    print(f"  Run crash:        {stats['run_crash']}")
    print(f"  Wrong output:     {stats['run_wrong']}")
    print(f"  Hang:             {stats['hang']}")
    print(f"  Total findings:   {len(findings)}")
    print()

    if findings:
        print("Findings:")
        for phase, result, path in findings:
            print(f"  [{phase}] {result}: {path}")
    else:
        print("No crashes or wrong output found.")

    return len(findings)


def main():
    parser = argparse.ArgumentParser(description="Blood codegen stress fuzzer")
    parser.add_argument("count", type=int, nargs="?", default=200,
                        help="Number of programs to generate (default: 200)")
    parser.add_argument("--compiler", "-c", default=COMPILER,
                        help=f"Compiler path (default: {COMPILER})")
    parser.add_argument("--findings", "-f", default=".fuzz/codegen_findings",
                        help="Findings output directory")
    args = parser.parse_args()

    if not os.path.exists(args.compiler):
        print(f"Compiler not found: {args.compiler}", file=sys.stderr)
        sys.exit(1)

    findings = fuzz(args.count, args.compiler, args.findings)
    sys.exit(1 if findings > 0 else 0)


if __name__ == "__main__":
    main()

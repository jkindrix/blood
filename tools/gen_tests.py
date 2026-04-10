#!/usr/bin/env python3
"""Blood test program generator.

Generates syntactically valid Blood programs that exercise complex compiler
paths: effects, closures, generics, patterns, regions, and dispatch.

Usage:
    python3 tools/gen_tests.py [count] [--output DIR] [--check COMPILER]

Examples:
    python3 tools/gen_tests.py 100                      # Generate 100 programs to stdout
    python3 tools/gen_tests.py 50 --output .fuzz/gen     # Write to directory
    python3 tools/gen_tests.py 200 --check build/first_gen  # Generate + check each
"""

import random
import sys
import os
import subprocess
import argparse
from typing import List

# ─── Building blocks ──────────────────────────────────────────────────────

PRIMITIVES = ["i32", "i64", "u32", "u64", "bool", "f64"]
NAMES = list("abcdefghijklmnopqrstuvwxyz")

def rand_name():
    return random.choice(NAMES) + str(random.randint(0, 99))

def rand_type():
    return random.choice(PRIMITIVES)

def rand_int_literal():
    return str(random.randint(-100, 100))

def rand_bool_literal():
    return random.choice(["true", "false"])

def rand_literal(ty="i32"):
    if ty == "bool":
        return rand_bool_literal()
    if ty in ("f64",):
        return f"{random.uniform(-100, 100):.2f}"
    return rand_int_literal()

# ─── Generators ───────────────────────────────────────────────────────────

def gen_simple_fn() -> str:
    """Simple function with arithmetic."""
    name = rand_name()
    ty = random.choice(["i32", "i64"])
    params = ", ".join(f"{rand_name()}: {ty}" for _ in range(random.randint(0, 3)))
    body_lines = []
    vars_in_scope = []

    for _ in range(random.randint(1, 5)):
        v = rand_name()
        body_lines.append(f"    let {v}: {ty} = {rand_literal(ty)};")
        vars_in_scope.append(v)

    if vars_in_scope:
        ret = " + ".join(random.sample(vars_in_scope, min(len(vars_in_scope), 3)))
    else:
        ret = rand_literal(ty)

    return f"fn {name}({params}) -> {ty} {{\n" + "\n".join(body_lines) + f"\n    {ret}\n}}"


def gen_struct() -> str:
    """Struct with fields."""
    name = "S" + rand_name()
    fields = []
    for _ in range(random.randint(1, 5)):
        fields.append(f"    {rand_name()}: {rand_type()},")
    return f"struct {name} {{\n" + "\n".join(fields) + "\n}"


def gen_enum() -> str:
    """Enum with variants."""
    name = "E" + rand_name()
    variants = []
    for i in range(random.randint(2, 5)):
        variant_name = f"V{i}"
        if random.random() < 0.5:
            variants.append(f"    {variant_name}({rand_type()}),")
        else:
            variants.append(f"    {variant_name},")
    return f"enum {name} {{\n" + "\n".join(variants) + "\n}"


def gen_match_expr(enum_name: str, variants: List[str]) -> str:
    """Match expression over an enum."""
    arms = []
    for v in variants:
        if random.random() < 0.5:
            arms.append(f"        {enum_name}.{v}(x) => x as i32,")
        else:
            arms.append(f"        {enum_name}.{v} => {rand_int_literal()},")
    return "match val {\n" + "\n".join(arms) + "\n    }"


def gen_closure() -> str:
    """Closure expression."""
    params = ", ".join(f"{rand_name()}: i32" for _ in range(random.randint(1, 3)))
    return f"|{params}| -> i32 {{ {rand_int_literal()} }}"


def gen_effect_program() -> str:
    """Program with effect declaration, handler, and perform."""
    effect_name = "Eff" + rand_name()
    op_name = "op" + rand_name()
    handler_name = "H" + rand_name()

    return f"""effect {effect_name} {{
    op {op_name}(x: i32) -> i32;
}}

deep handler {handler_name} for {effect_name} {{
    return(x) {{ x }}
    op {op_name}(x) {{
        resume(x + 1)
    }}
}}

fn use_{effect_name}() -> i32 / {{{effect_name}}} {{
    let a: i32 = perform {effect_name}.{op_name}(10);
    let b: i32 = perform {effect_name}.{op_name}(a);
    b
}}

fn main() -> i32 {{
    let result: i32 = with {handler_name} {{}} handle {{
        use_{effect_name}()
    }};
    if result == 12 {{ 0 }} else {{ 1 }}
}}
"""


def gen_generic_program() -> str:
    """Program with generic function and struct."""
    return f"""struct Pair<A, B> {{
    first: A,
    second: B,
}}

fn make_pair<A, B>(a: A, b: B) -> Pair<A, B> {{
    Pair {{ first: a, second: b }}
}}

fn first<A, B>(p: &Pair<A, B>) -> A {{
    p.first
}}

fn main() -> i32 {{
    let p: Pair<i32, i32> = make_pair(42, 99);
    let a: i32 = first(&p);
    if a == 42 {{ 0 }} else {{ 1 }}
}}
"""


def gen_trait_program() -> str:
    """Program with trait, impl, and dispatch."""
    trait_name = "T" + rand_name()
    method_name = "m" + rand_name()

    return f"""trait {trait_name} {{
    fn {method_name}(&self) -> i32;
}}

struct Foo {{
    val: i32,
}}

impl {trait_name} for Foo {{
    fn {method_name}(&self) -> i32 {{
        self.val
    }}
}}

fn call_trait(x: &Foo) -> i32 {{
    x.{method_name}()
}}

fn main() -> i32 {{
    let f: Foo = Foo {{ val: 42 }};
    let r: i32 = call_trait(&f);
    if r == 42 {{ 0 }} else {{ 1 }}
}}
"""


def gen_pattern_program() -> str:
    """Program exercising pattern matching."""
    return f"""enum Shape {{
    Circle(f64),
    Rect(f64, f64),
    Point,
}}

fn area(s: &Shape) -> f64 {{
    match s {{
        &Shape.Circle(r) => r * r * 3.14159,
        &Shape.Rect(w, h) => w * h,
        &Shape.Point => 0.0,
    }}
}}

fn main() -> i32 {{
    let c: Shape = Shape.Circle(2.0);
    let a: f64 = area(&c);
    if a > 12.0 {{ 0 }} else {{ 1 }}
}}
"""


def gen_closure_program() -> str:
    """Program with closures and higher-order functions."""
    return f"""fn apply(f: fn(i32) -> i32, x: i32) -> i32 {{
    f(x)
}}

fn main() -> i32 {{
    let add_one: fn(i32) -> i32 = |x: i32| -> i32 {{ x + 1 }};
    let double: fn(i32) -> i32 = |x: i32| -> i32 {{ x * 2 }};
    let a: i32 = apply(add_one, 10);
    let b: i32 = apply(double, a);
    if b == 22 {{ 0 }} else {{ 1 }}
}}
"""


def gen_region_program() -> str:
    """Program with region allocation."""
    return f"""fn main() -> i32 {{
    let mut total: i32 = 0;
    region {{
        let x: i32 = 42;
        let y: i32 = 58;
        total = x + y;
    }}
    if total == 100 {{ 0 }} else {{ 1 }}
}}
"""


def gen_for_loop_program() -> str:
    """Program with for-in loops."""
    return f"""fn main() -> i32 {{
    let v: Vec<i32> = vec![1, 2, 3, 4, 5];
    let mut sum: i32 = 0;
    for x in &v {{
        sum = sum + *x;
    }}
    if sum == 15 {{ 0 }} else {{ 1 }}
}}
"""


def gen_combined_program() -> str:
    """Combine multiple features in one program."""
    generators = [
        gen_effect_program,
        gen_generic_program,
        gen_trait_program,
        gen_pattern_program,
        gen_closure_program,
        gen_region_program,
        gen_for_loop_program,
    ]
    return random.choice(generators)()


# ─── Main ─────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Generate Blood test programs")
    parser.add_argument("count", type=int, nargs="?", default=10)
    parser.add_argument("--output", "-o", help="Output directory")
    parser.add_argument("--check", "-c", help="Compiler path to check each program")
    parser.add_argument("--seed", "-s", type=int, help="Random seed")
    args = parser.parse_args()

    if args.seed is not None:
        random.seed(args.seed)

    if args.output:
        os.makedirs(args.output, exist_ok=True)

    passed = 0
    failed = 0
    errors = 0

    for i in range(args.count):
        program = gen_combined_program()

        if args.output:
            path = os.path.join(args.output, f"gen_{i:04d}.blood")
            with open(path, "w") as f:
                f.write(program)

            if args.check:
                result = subprocess.run(
                    [args.check, "check", path],
                    capture_output=True, text=True, timeout=10
                )
                if result.returncode == 0:
                    passed += 1
                else:
                    failed += 1
                    if "panic" in result.stderr.lower() or "signal" in result.stderr.lower():
                        errors += 1
                        print(f"  ERROR: {path}", file=sys.stderr)

                if (i + 1) % 50 == 0:
                    print(f"  [{i+1}/{args.count}] pass={passed} fail={failed} error={errors}",
                          file=sys.stderr)
        else:
            print(f"// === Generated test {i} ===")
            print(program)
            print()

    if args.check:
        print(f"\nResults: {args.count} generated, {passed} passed check, "
              f"{failed} failed check, {errors} errors/crashes")


if __name__ == "__main__":
    main()

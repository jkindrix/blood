#!/usr/bin/env python3
"""
fpdiff - Floating-point diff utility for benchmark output validation.

Compares two files line-by-line, treating numbers as floating-point with tolerance.

Usage: fpdiff.py [-abserr TOL] [-relerr TOL] expected.txt actual.txt

Options:
    -abserr TOL   Absolute error tolerance (default: 1e-8)
    -relerr TOL   Relative error tolerance (default: 1e-8)

Exit codes:
    0 = files match within tolerance
    1 = files differ
    2 = error (missing file, etc.)
"""

import sys
import re
import argparse

def parse_args():
    parser = argparse.ArgumentParser(description='Floating-point diff utility')
    parser.add_argument('-abserr', type=float, default=1e-8, help='Absolute error tolerance')
    parser.add_argument('-relerr', type=float, default=1e-8, help='Relative error tolerance')
    parser.add_argument('expected', help='Expected output file')
    parser.add_argument('actual', help='Actual output file')
    return parser.parse_args()

def extract_numbers(line):
    """Extract all numbers from a line."""
    pattern = r'[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?'
    return re.findall(pattern, line)

def numbers_match(expected, actual, abserr, relerr):
    """Check if two number strings match within tolerance."""
    try:
        e = float(expected)
        a = float(actual)
    except ValueError:
        return expected == actual

    if abs(e - a) <= abserr:
        return True

    if e != 0 and abs((e - a) / e) <= relerr:
        return True

    return False

def compare_lines(expected_line, actual_line, abserr, relerr):
    """Compare two lines, treating numbers with tolerance."""
    exp_nums = extract_numbers(expected_line)
    act_nums = extract_numbers(actual_line)

    if len(exp_nums) != len(act_nums):
        return False, f"Different number count: {len(exp_nums)} vs {len(act_nums)}"

    for i, (e, a) in enumerate(zip(exp_nums, act_nums)):
        if not numbers_match(e, a, abserr, relerr):
            return False, f"Number mismatch at position {i}: {e} vs {a}"

    exp_text = re.sub(r'[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?', 'NUM', expected_line)
    act_text = re.sub(r'[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?', 'NUM', actual_line)

    if exp_text != act_text:
        return False, f"Non-numeric content differs"

    return True, None

def main():
    args = parse_args()

    try:
        with open(args.expected, 'r') as f:
            expected_lines = f.readlines()
        with open(args.actual, 'r') as f:
            actual_lines = f.readlines()
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 2

    if len(expected_lines) != len(actual_lines):
        print(f"Line count differs: {len(expected_lines)} vs {len(actual_lines)}")
        return 1

    errors = []
    for i, (exp, act) in enumerate(zip(expected_lines, actual_lines), 1):
        match, reason = compare_lines(exp.strip(), act.strip(), args.abserr, args.relerr)
        if not match:
            errors.append(f"Line {i}: {reason}")
            errors.append(f"  Expected: {exp.strip()}")
            errors.append(f"  Actual:   {act.strip()}")

    if errors:
        print("Files differ:")
        for e in errors[:20]:
            print(e)
        if len(errors) > 20:
            print(f"... and {len(errors) - 20} more differences")
        return 1

    print("Files match within tolerance")
    return 0

if __name__ == '__main__':
    sys.exit(main())

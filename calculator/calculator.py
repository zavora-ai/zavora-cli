"""Simple calculator library.

Provides basic arithmetic operations and a CLI entry point.
"""
from __future__ import annotations

from typing import Union

Number = Union[int, float]


def add(a: Number, b: Number) -> Number:
    return a + b


def subtract(a: Number, b: Number) -> Number:
    return a - b


def multiply(a: Number, b: Number) -> Number:
    return a * b


def divide(a: Number, b: Number) -> Number:
    if b == 0:
        raise ZeroDivisionError("division by zero")
    return a / b


def power(a: Number, b: Number) -> Number:
    return a ** b


def mod(a: Number, b: Number) -> Number:
    return a % b


def mean(numbers: list[Number]) -> float:
    if not numbers:
        raise ValueError("mean requires at least one number")
    return sum(numbers) / len(numbers)


def parse_number(value: str) -> Number:
    try:
        if "." in value:
            return float(value)
        return int(value)
    except ValueError:
        raise ValueError(f"invalid number: {value}")


def cli(argv: list[str]) -> int:
    """Minimal CLI: calculator add 1 2

    Returns exit code.
    """
    import argparse

    parser = argparse.ArgumentParser(prog="calculator")
    sub = parser.add_subparsers(dest="cmd", required=True)

    def two_args(name, help):
        p = sub.add_parser(name, help=help)
        p.add_argument("a")
        p.add_argument("b")
        return p

    two_args("add", "Add two numbers")
    two_args("sub", "Subtract two numbers")
    two_args("mul", "Multiply two numbers")
    two_args("div", "Divide two numbers")
    two_args("pow", "Power a^b")
    two_args("mod", "Modulo a % b")

    mean_p = sub.add_parser("mean", help="Mean of numbers")
    mean_p.add_argument("numbers", nargs="+")

    args = parser.parse_args(argv)

    if args.cmd == "add":
        print(add(parse_number(args.a), parse_number(args.b)))
    elif args.cmd == "sub":
        print(subtract(parse_number(args.a), parse_number(args.b)))
    elif args.cmd == "mul":
        print(multiply(parse_number(args.a), parse_number(args.b)))
    elif args.cmd == "div":
        try:
            print(divide(parse_number(args.a), parse_number(args.b)))
        except ZeroDivisionError as e:
            print(e)
            return 1
    elif args.cmd == "pow":
        print(power(parse_number(args.a), parse_number(args.b)))
    elif args.cmd == "mod":
        print(mod(parse_number(args.a), parse_number(args.b)))
    elif args.cmd == "mean":
        nums = [parse_number(n) for n in args.numbers]
        print(mean(nums))

    return 0

from __future__ import annotations

from .calculator import cli


def main() -> int:
    import sys
    return cli(sys.argv[1:])

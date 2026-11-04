import argparse
import sys

from harness_ai import __version__


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="harness_ai", description="AI Harness intelligence layer")
    parser.add_argument("--version", action="version", version=f"harness_ai {__version__}")
    parser.parse_args(argv)
    # stdout is reserved for protocol JSONL in later tickets; diagnostics go to stderr.
    print("harness_ai: no commands implemented yet", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

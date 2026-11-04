import argparse
import sys

from harness_ai import __version__


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="harness_ai", description="AI Harness intelligence layer")
    parser.add_argument("--version", action="version", version=f"harness_ai {__version__}")
    sub = parser.add_subparsers(dest="command")
    sub.add_parser("worker", help="run the JSONL worker on stdin/stdout")
    args = parser.parse_args(argv)
    if args.command == "worker":
        from harness_ai.worker import run_stdio

        return run_stdio()
    # stdout is reserved for protocol JSONL; diagnostics go to stderr.
    print("harness_ai: no command given (try `worker`)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

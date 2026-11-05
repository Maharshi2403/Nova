#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
uv run --locked --project intelligence python -m unittest discover -s tests -p test_documentation_verifier.py
cd intelligence
uv run --locked ruff check .
uv run --locked ruff format --check .
uv run --locked mypy

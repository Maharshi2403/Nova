# AI Harness (Nova)

Local-first coding-agent runtime. Rust owns persisted state and scheduling; a Python
package (added in later tickets) proposes actions and runs constrained LLM loops.

Status: ticket H-001 — Cargo workspace with empty CLI and daemon entrypoints.

## Repository structure

```text
Cargo.toml            workspace
rust-toolchain.toml   pinned toolchain
apps/cli/             bin `harness` (package harness-cli)
apps/daemon/          bin `harnessd` (package harness-daemon)
crates/harness-core/  domain types and versioned contracts (protocol envelope)
intelligence/         Python package `harness_ai` (uv)
schemas/              contract-first JSON Schemas (source of truth for IPC/plan shapes)
tests/fixtures/       cross-language fixtures (valid/ and invalid/) used by Rust and Python tests
scripts/              check.ps1 / check.sh quality gates
.github/workflows/    CI (offline, mock provider)
```

config/               example configuration (safe defaults: provider=mock, concurrency=2)
docs/adr/             architecture decision records (ADR-001..008, stubs)

Configuration precedence: CLI flags > `HARNESS_*` env vars > config file > built-in defaults.

Planned (later tickets): `crates/harness-{store,runtime,git,api}`.

## Develop

```powershell
cargo test --workspace
cargo run -p harness-cli -- --help
cargo run -p harness-daemon -- --help
```

## Quality checks

One command per platform runs `cargo fmt --check`, `cargo clippy -D warnings`,
`ruff check`, `ruff format --check` and strict `mypy`:

```powershell
./scripts/check.ps1     # Windows
./scripts/check.sh      # macOS / Linux
```

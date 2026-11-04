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
```

Planned (later tickets): `crates/harness-{core,store,runtime,git,api}`, `intelligence/`
(Python, uv), `schemas/`, `config/`, `tests/`, `docs/adr/`, `.github/workflows/`.

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

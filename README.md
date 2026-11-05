# Nova — local coding-agent harness

Rust owns task state, scheduling, budgets, permissions, verification and Git
integration. Python proposes plans and coding actions through versioned JSONL.
The CLI talks to an authenticated loopback daemon. Tauri UI is deferred.

The offline runtime and opt-in live coder are implemented. Local macOS tests pass;
Windows and hosted release gates require CI evidence before release. See
[PROGRESS.md](PROGRESS.md) for actual checks and limitations.

## Build and demonstrate

Requires Git, uv, Python >=3.12 and the pinned Rust toolchain. Initial dependency
installation uses package registries; tests and demos make no live provider calls.

```sh
uv sync --locked --project intelligence
cargo build --locked --workspace
uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 3
```

The demo creates a disposable repo, starts the daemon, registers a trusted project,
submits/approves a plan, executes real workers and brokered writes, verifies exact
output, and checks task/integration SHAs. The mock coder writes synthetic fixture
files, not general code. Your original checkout stays unchanged.

```sh
uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 100
uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 1 --fault
uv run --locked --offline --project intelligence python scripts/offline_demo.py --tasks 1 --cancel
```

## Operate

Start `target/debug/harnessd --data-dir /absolute/private/harness-data` in a terminal.
On Windows append `.exe`. Pass the same directory to `harness --daemon-dir`.
Default state is `AIHarness` beneath `LOCALAPPDATA`, falling back to `HOME`.
One daemon owns a state directory.

Register with `harness init /absolute/repo --name Demo --trust-local` only when you
authorize unsandboxed execution of that repository's code. Omitting the flag creates
an untrusted project that cannot execute. Worktrees are not OS sandboxes.

See [operator workflows](docs/OPERATIONS.md), [configuration](docs/CONFIGURATION.md),
[trust boundary](docs/TRUST_BOUNDARY.md), and [control API](docs/CONTROL_API.md).
Live coding requires `--live --model-config`, task/run/daily budgets, and a
daemon-scoped `OPENAI_API_KEY`. Never put credentials in plans or config files.
Paid-provider smoke testing has not been performed as part of this build.

## Develop

```sh
cargo test --locked --offline --workspace
cargo fmt --all --check
cargo clippy --locked --offline --workspace --all-targets -- -D warnings
uv run --locked --offline --project intelligence pytest -q
```

`scripts/check.sh` and `scripts/check.ps1` run lint, formatting and strict typing.
CI also runs full tests and load/crash/cancel demos on Windows/macOS/Linux.
`apps/{cli,daemon}` are control surfaces; `crates/harness-core` holds contracts,
`harness-store` owns SQLite, and `harness-runtime` owns execution/brokers/Git.
`intelligence` contains the Python planner, coder and provider adapters. Shared
schemas and cross-language fixtures live in `schemas` and `tests/fixtures`.

No push, deployment, release publication or tag is automatic.

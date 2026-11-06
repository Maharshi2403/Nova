# Nova — local coding-agent harness

Rust owns task state, scheduling, budgets, permissions, verification and Git
integration. Python proposes plans and coding actions through versioned JSONL.
The CLI and native macOS dashboard talk to an authenticated loopback daemon.

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

Install the single Nova executable, then start it:

```sh
cargo install --path apps/cli --bin nova --locked
nova
```

`nova` starts the service in the background or connects if it is already running.
`nova start` does the same thing. `nova stop` safely stops an idle service; active
runs must finish or be paused/cancelled first. `nova --help` lists the commands.
Use `nova ui` to open the small floating island panel after installing the desktop
bundle; see [desktop setup](docs/DESKTOP.md). It opens as a compact Nova logo/status
pill near the top-center island, with expandable controls and guided project/task-plan
forms, provider setup, model routes, progress, approvals, costs and evidence.
No agents or paid provider calls start automatically. No separate daemon binary
is needed to use the installed Nova command.

Default state on macOS is `~/Library/Application Support/Nova`; elsewhere it is
`Nova` beneath LOCALAPPDATA or HOME. Use `nova --daemon-dir PATH` for another
directory, including an existing legacy AIHarness directory. Nothing is migrated
or deleted automatically. The `harness` binary remains a compatibility alias.

Register with `nova init /absolute/repo --name Demo --trust-local` only when you
authorize unsandboxed execution of that repository's code. Omitting the flag creates
an untrusted project that cannot execute. Projects can be added with uncommitted
changes; desktop execution uses a committed snapshot in separate worktrees and
leaves your unfinished files untouched. CLI users can select `--committed-snapshot`;
normal execution still requires a clean checkout. Worktrees are not OS sandboxes.

See [operator workflows](docs/OPERATIONS.md), [configuration](docs/CONFIGURATION.md),
[trust boundary](docs/TRUST_BOUNDARY.md), and [control API](docs/CONTROL_API.md).
Connect providers once, then choose models for each use case:

```sh
nova providers add openai
nova providers add anthropic
nova models list
nova routes set coding openai/gpt-6.1-sol
nova routes set review anthropic/claude-sonnet-5-5
nova routes list
```

Setup prompts for each API key privately, stores it in your OS credential vault,
and discovers models available to your account. Use IDs printed by `models list`.
OpenAI-compatible services (including OpenRouter and local servers) use
`nova providers add NAME --kind compatible --base-url URL`; `openrouter` is also a
preset. See [provider setup](docs/PROVIDERS.md) for metadata and routing details.
Live coding requires `--live` and task/run/daily budgets. The daemon uses saved
provider settings; `nova budget set --run USD --daily USD` sets spend ceilings.
Never put credentials in plans or config files.
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
For native candidate packaging and installation smoke tests, see
[release checklist](docs/RELEASE_CHECKLIST.md).

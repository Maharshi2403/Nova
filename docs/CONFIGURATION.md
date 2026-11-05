# Configuration reference

| Surface | Option | Effective behavior |
|---|---|---|
| advanced daemon | `--data-dir PATH` | Private token/discovery/SQLite/worktree directory |
| CLI | `--daemon-dir PATH` | Same directory for discovery/authentication |
| CLI | bare `nova` / `nova start` | Start or attach to background service |
| CLI | `nova stop` | Authenticated idle shutdown; refuses active runs |
| CLI | `--json` | JSON output; otherwise pretty JSON |
| init | `--trust-local` | Explicit unsandboxed trusted-local authorization |
| execute | `--python PATH` | Absolute installed intelligence venv interpreter |
| execute | `--test-commands FILE` | JSON array of exact accepted test argv arrays |
| execute | `--concurrency 1..2` | Default two; daemon-wide execution capacity two |
| execute | `--live [--model-config FILE]` | `--live` enables saved providers; model-config is optional |
| daemon env | `OPENAI_API_KEY` | Only for explicitly opted-in provider calls |
| tests | `HARNESS_PYTHON` | Optional test worker interpreter override |

`config/defaults.example.yaml` and `harness_core::config::Config::resolve` define the
foundational layered contract (CLI > HARNESS_* env > file > defaults). The shipped
daemon does not automatically load that YAML or apply every library setting. Use
the explicit options above. Changing the example cannot enable network, larger
concurrency or automatic push. Live mode is explicit; the pool caps capacity at two.

`config/live.example.json` has a deliberately unavailable placeholder. Registry
entries specify logical ID, actual provider model ID, provider, tool capabilities,
structured output, context/output limits, prices per million tokens, availability
and privacy. Routing low/medium/high fields reference logical IDs. High-risk tasks
use the high route. Missing/unavailable/incompatible routes fail closed.

Live run/daily budgets must be nonzero (integer microdollars). Enforcement uses
configured estimates: review prices/account availability before authorizing spend.
No production model ID or price is assumed.

## Bounds

Frames: 1 MiB; writes: 1 MiB; diagnostics: 64 KiB; default prompt cap: 64 KiB. Live
coder prompts use up to 32 KiB plus bounded schema/framing, fitting the configured
window. Acceptance is never silently truncated. Coder loops have at most 16 turns,
at most three transient retries per turn and an assignment deadline. Rust also
limits aggregate tool calls, output tokens and estimated cost. Provider responses:
2 MiB. Event polling: 1000 rows, 25 seconds, 64 concurrent waiters.

Saved provider profiles and use-case routes are documented in [PROVIDERS.md](PROVIDERS.md).

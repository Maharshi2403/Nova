# Provider setup and model routing

Nova supports OpenAI's Responses API, native Claude Messages, and services speaking
OpenAI-compatible Chat Completions with JSON Schema structured output. A different
protocol needs an adapter. A ChatGPT or Claude chat subscription is separate from
API credentials/billing. Models must support Nova's structured action schema.

```sh
nova providers add openai
nova providers add anthropic
nova providers add openrouter
nova models list
nova routes set coding openai/gpt-6.1-sol
nova routes set review anthropic/claude-sonnet-5-5
nova routes set architecture openai/gpt-6-astra
nova routes list
```

Use the exact discovered model IDs; the examples require account access to those
models. Adding a provider prompts for a hidden key, discovers account models using
GET /models (no generation), and saves its connection. On macOS keys live in
Keychain; Windows uses Credential Manager. The profile JSON contains opaque vault
references, never keys. Never paste keys into chat or CLI arguments. Native OS
vault access may display an operating-system unlock/access prompt.

Custom gateways and local servers:

```sh
nova providers add gateway --kind compatible --base-url https://gateway.example/v1
nova providers add local --kind compatible --base-url http://127.0.0.1:8000/v1
```

HTTPS is required except literal loopback/localhost HTTP. Redirects and environment
HTTP proxies are disabled. Only explicitly configured compatible connections can
use custom endpoints; native providers use official URLs. Local servers still need
a nonempty key (use their documented token). Gateways receive your task context.

`nova config set python PATH` saves and audits an installed Nova intelligence
interpreter. `nova providers add --python PATH` also saves and audits it. Current
source installations and extracted bundles with a sibling `runtime` venv are found
automatically. `--skip-discovery` saves a connection without a network request;
it does not claim any model is available. On Linux or headless setups use
`--key-env VARIABLE` referencing a secret supplied to the Nova service environment.
An already-running daemon cannot see variables added later to another shell; restart
Nova from the environment containing those references. Never save keys in shell
history. List/remove connections with `nova providers list` and
`nova providers remove NAME`. Removing a connection clears its models/routes/key.

`nova models sync PROVIDER` refreshes account discovery. Known exact native model
IDs get bundled official metadata dated 2026-11-05. Compatible rich catalogs such
as OpenRouter can supply context, output caps, structured-output support and
per-token prices. Unknown metadata remains unavailable, rather than free. For a
model whose provider documents its capabilities/prices, supply them explicitly:

```sh
nova models configure gateway MODEL_ID --context 128000 --max-output 8192 --input-price 2 --output-price 10 --structured-output --tier medium
```

Prices above are illustrative USD per million tokens. Confirm actual rates first.
Models with per-request fees need an accounting adapter and cannot be automatically
marked ready. Sync replaces manual metadata for that provider; reconfigure after
sync when necessary. Haiku's higher-context token rate is used conservatively.
These are estimates, not provider billing guarantees; no paid capability probes or
benchmarks run automatically.

Each task proposal can carry `use_case`: coding (default), planning, review,
debugging, architecture or fast. A saved route pins that use case to one model
across complexity/risk tiers. Other tasks use deterministic low/medium/high routes:
cheapest known model in each declared tier, falling back to the cheapest ready
model when a tier is absent. Selection does not infer model expertise or promise
quality; choose models based on your own use cases. `nova routes reset` clears
pins. Route changes apply to newly launched executions; in-flight runs retain
their validated configuration. A planning/review task follows the same constrained
worker and acceptance workflow; it does not automatically create extra paid
planning/review passes. Include report paths and executable acceptance as needed.

Execute an approved run using `nova execute RUN_ID --live --test-commands FILE`.
Saved settings default to $5 per run and $10 per UTC day; each approved task still
has its own cost cap. `providers.json` stores these integer microdollar caps and
public routing/model metadata. Set or change ceilings with
`nova budget set --run USD --daily USD`; one launch may request lower caps.
The daemon builds the live registry from saved settings and never accepts a
request-supplied model registry. Missing keys/capabilities reject execution.
Credentials are resolved by Rust and sent only to the chosen helper; coder and
verifier processes never receive them. Every paid attempt/retry remains subject
to grants, tool limits and persistent spend reservations. No automatic provider
fallback silently switches billing accounts.

API references: [OpenAI models](https://developers.openai.com/api/docs/models),
[Claude structured output](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
[Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing),
[OpenRouter catalog](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties).

Anthropic requests pin `anthropic-version` through the adapter's
`ANTHROPIC_API_VERSION` constant (`2026-10-21`, verified 2026-11-07 against
https://platform.claude.com/docs/en/api/versioning). Null or absent cache token
counts contribute zero; absent input/output token counts remain unknown.
Non-`end_turn` completions fail rather than being treated as complete actions.

Structured task prompts place tools and contract first, then required task data,
context snippets, and finally volatile history. Anthropic receives the stable
prefix as a text block with ephemeral cache control; snippets and history stay
in a separate uncached block. This marks a cache boundary, not evidence of a
cache hit. Actual cache token accounting comes from provider usage fields.

Anthropic explicit cache boundaries are enabled only when both cached-input
and cache-write prices are configured. Rust includes the highest input price
in its grant bound and records cache-aware costs independently of worker claims.

Model registries and saved model metadata accept optional
`cached_input_usd_per_million_tokens` and `cache_write_usd_per_million_tokens`.
Omitted values mean unknown, not zero. Supplied prices must be finite and
non-negative. The provider adapter preserves Anthropic cache-creation token
counts separately from total input and cached-read counts; older adapter
results may omit this optional count.

Configure per-million-token cache prices with `nova models configure` using
`--cached-input-price` and `--cache-write-price`, alongside the existing input
and output prices. For observed nonzero cache buckets, an absent required price
leaves the cost unknown. Total input includes normal, cached-read and
cache-creation tokens; the buckets are priced separately without double counting.

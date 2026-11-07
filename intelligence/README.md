# harness-ai

Python intelligence layer for AI Harness. It never writes SQLite or changes task state;
Rust owns persisted state and scheduling.

```powershell
uv sync --locked
uv run --locked pytest
uv run --locked python -m harness_ai --version
```

## Bounded prompts (H-045)

`harness_ai.context.build_prompt` creates a stable role prefix and SHA-256 prefix
fingerprint, followed by JSON task data. `task_handoff` retains the exact acceptance
commands/assertions, dependencies, scope and limits without replaying conversation
history. Supply only snippets authorized by Rust's repository broker.

The cap (default and maximum 64 KiB) covers serialized messages, including JSON
escaping. Optional snippets (at most eight, each at most 8 KiB) and previous summaries
(at most 2 KiB) are omitted or shortened to fit. Mandatory task data is never silently
truncated: `ContextLimitError` stops oversized requests before provider invocation.
Planning uses this builder; coder prompt construction is available for future live
worker loops. This is a byte bound, not a model-specific token measurement; schemas
and provider framing add request overhead. Existing Rust token/spend limits remain
authoritative. Prefix fingerprints indicate identity, not a cache hit. Only provider
usage supplies measured cached input tokens; absent usage stays unknown.

Live coder workers use a dedicated stdin reader and await broker responses by
request correlation ID. Model calls and repository tools leave the event loop
available for heartbeats and deadlines. A daemon cancel frame cancels the
assignment, emits `failed` with reason `cancelled`, and exits successfully.
Each live worker handles one assignment; the reader is stopped on completion.
No credentials enter the coder process; Rust still authorizes provider calls.

Coder working memory retains six bounded turn records without summarizing them
again. Older turns compact deterministically into successful read ranges,
changed files, command argv/exit codes and model hypotheses. The memory block
has a 24 KiB default ceiling; smaller context budgets reduce it further, allowing
for JSON escaping while preserving required task data. Excerpts are bounded
before insertion (results at most 8 KiB), and omitted summary facts are counted.
The optional model action `notes` field accepts at most 2 KiB of UTF-8 scratchpad
text. Memory and notes never grant permissions or substitute for Rust evidence.

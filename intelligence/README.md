# harness-ai

Python intelligence layer for AI Harness. It never writes SQLite or changes task state;
Rust owns persisted state and scheduling.

```powershell
uv sync --locked
uv run --locked pytest
uv run --locked python -m harness_ai --version
```

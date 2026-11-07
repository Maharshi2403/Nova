import asyncio
import json
from pathlib import Path
from typing import Any

from harness_ai.assignment import ModelRef, StartTask, ToolResponse
from harness_ai.coder import code_task
from harness_ai.providers.mock import MockModelProvider


def exercise(
    *,
    denied: bool = False,
    reads: int = 1,
    max_tool_calls: int = 10,
    max_model_turns: int | None = None,
) -> tuple[list[str], list[tuple[str, dict[str, Any]]], MockModelProvider]:
    task = json.loads(
        (Path(__file__).resolve().parents[2] / "tests/fixtures/plans/valid_plan.json").read_text()
    )["tasks"][0]
    task["runtime"] = {
        "max_output_tokens": 2000,
        "context_byte_cap": 16000,
        "input_usd_per_million_tokens": 1,
        "output_usd_per_million_tokens": 2,
    }
    calls: list[str] = []
    events: list[tuple[str, dict[str, Any]]] = []

    def tool(name: str, args: dict[str, Any]) -> ToolResponse:
        calls.append(name)
        if name == "task.describe":
            return ToolResponse(ok=True, result=task)
        return ToolResponse(ok=not denied, result={"content": "bounded repo text"})

    provider = MockModelProvider(
        queue=[
            *[
                {
                    "action": "tool",
                    "tool": "repo.read",
                    "arguments_json": '{"path":"README.md"}',
                    "summary": f"inspect {index}",
                }
                for index in range(reads)
            ],
            {"action": "completed", "tool": None, "arguments_json": None, "summary": "finished"},
        ]
    )
    assignment = StartTask(
        role="coder",
        worktree_id="wt",
        model=ModelRef(provider="openai", model_id="configured"),
        context_manifest_hash="h",
        allowed_tools=["task.describe", "model.authorize", "repo.read"],
        max_tool_calls=max_tool_calls,
        max_model_turns=max_model_turns,
        max_output_tokens=2000,
        timeout_seconds=10,
        cost_cap_usd=1,
    )
    asyncio.run(code_task(assignment, provider, tool, lambda k, p: events.append((k, p))))
    return calls, events, provider


def test_live_coder_uses_broker_and_preserves_usage_offline() -> None:
    calls, events, provider = exercise()
    assert calls == ["task.describe", "model.authorize", "repo.read", "model.authorize"]
    assert [e[0] for e in events] == ["usage", "usage", "completed"]
    assert events[0][1]["price_source"] == "registry_estimate"
    assert len(provider.calls) == 2
    assert provider.calls[0]["messages"][0] == provider.calls[1]["messages"][0]


def test_live_coder_spend_denied_before_any_provider_call() -> None:
    calls, events, provider = exercise(denied=True)
    assert calls == ["task.describe", "model.authorize"]
    assert not provider.calls
    assert events[-1][0] == "failed"


def test_live_coder_can_use_task_turn_budget_beyond_sixteen() -> None:
    calls, events, provider = exercise(reads=17, max_tool_calls=18)
    assert len(provider.calls) == 18
    assert calls.count("repo.read") == 17
    assert events[-1][0] == "completed"


def test_coder_ends_cleanly_on_turn_budget() -> None:
    calls, events, provider = exercise(reads=2, max_tool_calls=10, max_model_turns=1)
    assert len(provider.calls) == 1
    assert calls.count("repo.read") == 1
    assert events[-1] == ("failed", {"reason": "turn budget exhausted", "retryable": True})
    assert "final model turn" in provider.calls[0]["messages"][1]["content"].lower()


def test_coder_sends_bounded_transcript() -> None:
    _, events, provider = exercise(reads=8)
    prompt = json.loads(provider.calls[-1]["messages"][1]["content"])
    memory = json.loads(prompt["previous_summary"])
    assert len(memory["recent"]) == 6
    assert memory["summary"]["files_read"][0]["path"] == "README.md"
    assert events[-1][0] == "completed"


def test_action_notes_enforce_utf8_byte_cap() -> None:
    import pytest
    from pydantic import ValidationError

    from harness_ai.coder import Action

    data = {"action": "completed", "tool": None, "arguments_json": None, "summary": "done"}
    assert len(Action.model_validate({**data, "notes": "🔒" * 512}).notes.encode()) == 2048
    with pytest.raises(ValidationError):
        Action.model_validate({**data, "notes": "🔒" * 513})

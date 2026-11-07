import asyncio
import hashlib
import json
from pathlib import Path
from typing import Any

import pytest

from harness_ai.context import ContextLimitError, build_prompt, task_handoff
from harness_ai.plan import PlanProposal
from harness_ai.planner import propose_plan
from harness_ai.providers.base import Message, ModelResult, Usage


@pytest.mark.parametrize("content", ["x" * 50_000, '🔒\\\n"' * 10_000])
def test_context_within_cap(content: str) -> None:
    prompt = build_prompt(
        "coder",
        {"acceptance": ["must pass tests"]},
        snippets=[{"path": "src/main.py", "start_line": 1, "content": content}] * 100,
        previous_summary=content,
        byte_cap=2000,
    )
    assert prompt.byte_size <= 2000
    assert prompt.truncated
    assert json.loads(prompt.data)["required"]["acceptance"] == ["must pass tests"]
    assert len(json.loads(prompt.data)["snippets"]) <= 8


def test_stable_prefix_hash() -> None:
    a = build_prompt("coder", {"task": "first"})
    b = build_prompt("coder", {"task": "second"}, previous_summary="new facts")
    assert a.prefix_hash == b.prefix_hash == hashlib.sha256(a.system.encode()).hexdigest()
    assert a.messages[0] == b.messages[0]
    assert a.prefix_hash != build_prompt("planner", {}).prefix_hash
    assert a.messages[0]["role"] == "system"
    assert "untrusted data" in a.system


def test_summary_retains_acceptance() -> None:
    fixture = Path(__file__).resolve().parents[2] / "tests/fixtures/plans/valid_plan.json"
    task = PlanProposal.model_validate_json(fixture.read_text()).tasks[1]
    handoff = task_handoff(task)
    prompt = build_prompt("coder", handoff, previous_summary="old output " * 10_000)
    restored = json.loads(prompt.data)["required"]
    assert restored["acceptance"] == [a.model_dump(exclude_none=True) for a in task.acceptance]
    assert restored["dependencies"] == task.dependencies
    assert restored["candidate_paths"] == task.candidate_paths
    assert restored["limits"] == task.limits.model_dump()
    assert prompt.truncated


def test_no_truncation_when_context_fits() -> None:
    entry = {"path": "code.py", "start_line": 4, "content": "print('hello')"}
    a = build_prompt("coder", {"id": "a"}, snippets=[entry], previous_summary="done")
    b = build_prompt("coder", {"id": "a"}, snippets=[entry], previous_summary="done")
    assert a == b
    assert not a.truncated
    assert json.loads(a.data)["snippets"] == [entry]
    assert json.loads(a.data)["previous_summary"] == "done"


@pytest.mark.parametrize("cap", [0, -1, 65_537])
def test_invalid_cap_rejected(cap: int) -> None:
    with pytest.raises(ContextLimitError):
        build_prompt("coder", {}, byte_cap=cap)


def test_oversized_acceptance_rejected_intact() -> None:
    required = {"acceptance": ["test exact " * 1000]}
    with pytest.raises(ContextLimitError, match="acceptance was not truncated"):
        build_prompt("coder", required, byte_cap=1000)
    assert required["acceptance"] == ["test exact " * 1000]


class RecordingProvider:
    def __init__(self) -> None:
        self.calls: list[list[Message]] = []

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        self.calls.append(messages)
        fixture = Path(__file__).resolve().parents[2] / "tests/fixtures/plans/valid_plan.json"
        return ModelResult(
            structured_output=json.loads(fixture.read_text()),
            usage=Usage(input_tokens=100, output_tokens=20, cached_input_tokens=40, source="mock"),
            provider="mock",
            model_id=model_id,
        )


def test_planner_cap_before_provider_and_cache_usage_preserved() -> None:
    provider = RecordingProvider()
    with pytest.raises(ContextLimitError):
        asyncio.run(
            propose_plan(
                provider, project_id="p", idea="x" * 5000, model_id="mock", context_byte_cap=1000
            )
        )
    assert not provider.calls
    _, result = asyncio.run(
        propose_plan(provider, project_id="p", idea="A small API", model_id="mock")
    )
    assert len(provider.calls) == 1
    assert json.loads(provider.calls[0][1]["content"])["required"]["idea"] == "A small API"
    assert result.usage.cached_input_tokens == 40
    assert result.usage.input_tokens == 100


def test_use_case_survives_compact_handoff() -> None:
    fixture = Path(__file__).resolve().parents[2] / "tests/fixtures/plans/valid_plan.json"
    plan = json.loads(fixture.read_text())
    plan["tasks"][0]["use_case"] = "review"
    task = PlanProposal.model_validate(plan).tasks[0]
    assert task_handoff(task)["use_case"] == "review"
    plan["tasks"][0]["use_case"] = "unrecognized"
    with pytest.raises(ValueError):
        PlanProposal.model_validate(plan)


def test_stable_prefix_identical_across_turns() -> None:
    prompts = [
        build_prompt(
            "coder",
            {"acceptance": ["exact check"], "available_tools": ["repo.read"]},
            previous_summary=f"turn {turn}",
        )
        for turn in [1, 2, 3]
    ]
    prefixes = [p.data.split(',"snippets":', 1)[0].encode() for p in prompts]
    assert prefixes[0] == prefixes[2]
    assert b'"required"' in prefixes[0]
    assert b'"previous_summary"' not in prefixes[0]

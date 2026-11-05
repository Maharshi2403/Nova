"""Planner: ask a model provider for a plan proposal and validate it strictly.

The planner only proposes. Rust re-validates the DAG and decides what may run.
"""

from __future__ import annotations

from typing import Any

from pydantic import ValidationError

from harness_ai.context import MAX_CONTEXT_BYTES, build_prompt
from harness_ai.plan import PlanProposal
from harness_ai.providers.base import ModelProvider, ModelResult


class PlanningError(Exception):
    """The provider's output is not a valid plan."""


def plan_schema() -> dict[str, Any]:
    return PlanProposal.model_json_schema()


async def propose_plan(
    provider: ModelProvider,
    *,
    project_id: str,
    idea: str,
    model_id: str,
    max_output_tokens: int = 16_000,
    timeout_seconds: float = 120,
    context_byte_cap: int = MAX_CONTEXT_BYTES,
) -> tuple[PlanProposal, ModelResult]:
    prompt = build_prompt(
        "planner", {"project_id": project_id, "idea": idea}, byte_cap=context_byte_cap
    )
    result = await provider.complete(
        messages=prompt.messages,
        schema=plan_schema(),
        model_id=model_id,
        max_output_tokens=max_output_tokens,
        timeout_seconds=timeout_seconds,
    )
    try:
        return PlanProposal.model_validate(result.structured_output), result
    except ValidationError as exc:
        raise PlanningError(f"provider returned an invalid plan: {exc}") from exc

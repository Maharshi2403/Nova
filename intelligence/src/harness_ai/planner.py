"""Planner: ask a model provider for a plan proposal and validate it strictly.

The planner only proposes. Rust re-validates the DAG and decides what may run.
"""

from __future__ import annotations

import json
from typing import Any

from pydantic import ValidationError

from harness_ai.plan import PlanProposal
from harness_ai.providers.base import ModelProvider, ModelResult

SYSTEM_PROMPT = (
    "You are the planning supervisor of a coding harness. Return only JSON matching the plan "
    "schema: requirements, assumptions, a recommended architecture with alternatives and "
    "risks, and small tasks. Every task needs objective acceptance criteria. Never include "
    "secrets."
)


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
) -> tuple[PlanProposal, ModelResult]:
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": json.dumps({"project_id": project_id, "idea": idea})},
    ]
    result = await provider.complete(
        messages=messages,
        schema=plan_schema(),
        model_id=model_id,
        max_output_tokens=max_output_tokens,
        timeout_seconds=timeout_seconds,
    )
    try:
        return PlanProposal.model_validate(result.structured_output), result
    except ValidationError as exc:
        raise PlanningError(f"provider returned an invalid plan: {exc}") from exc

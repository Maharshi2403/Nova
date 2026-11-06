"""Main orchestration agent: turn one user goal into small, verifiable subagent tasks.

The model only proposes. This module converts its answer into a strict PlanProposal;
Rust re-validates the DAG, persists it for human approval, routes each task to a model
by complexity/risk, and enforces scope, budgets and verification.
"""

from __future__ import annotations

import re
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, ValidationError

from harness_ai.context import build_prompt
from harness_ai.plan import Acceptance, Architecture, PlanProposal, TaskLimits, TaskProposal
from harness_ai.providers.base import ModelProvider, ModelResult

Level = Literal["low", "medium", "high"]
UseCase = Literal["coding", "debugging", "architecture", "review", "fast"]


class PlanningError(Exception):
    """The model's decomposition is unusable. Messages never echo model output."""


class _Strict(BaseModel):
    # No defaults or length constraints: providers' strict JSON-schema modes reject them.
    model_config = ConfigDict(extra="forbid", frozen=True)


class GoalTask(_Strict):
    key: str
    title: str
    instructions: str
    use_case: UseCase
    complexity: Level
    risk: Level
    depends_on: list[str]
    files: list[str]
    test_command: list[str]


class GoalPlan(_Strict):
    summary: str
    assumptions: list[str]
    risks: list[str]
    tasks: list[GoalTask]


def goal_schema() -> dict[str, Any]:
    return GoalPlan.model_json_schema()


INSTRUCTIONS = {
    "decomposition": (
        "Split the goal into the smallest set of independent, precise subtasks a focused "
        "coding agent can finish alone. Each subtask owns a disjoint set of files where "
        "possible. Use depends_on only when a subtask needs another's output. Prefer one "
        "subtask for a small goal."
    ),
    "files": (
        "files are repository-relative paths (or directories ending in /) the subtask may "
        "create or edit. Nothing else can be changed."
    ),
    "tests": (
        "test_command is one argv array that exits 0 only when the subtask is correct. "
        "argv[0] must be exactly one of available_test_runners. No shell syntax: pipes, "
        "&&, redirects and globs are not interpreted. It runs from the repository root "
        "after the subtask's files are written, together with earlier subtasks' output."
    ),
    "rating": (
        "complexity low = mechanical edit; medium = ordinary feature/bug fix; high = "
        "cross-cutting design or subtle logic. risk high for security, data loss, or "
        "migrations. Cheaper models are used for low complexity, stronger ones for high."
    ),
    "instructions": (
        "instructions must be self-contained: the subagent sees only its own task, files, "
        "and test. State exact behaviour, names and edge cases."
    ),
}


def _slug(text: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:40]
    return slug or "task"


async def plan_goal(
    provider: ModelProvider,
    *,
    project_id: str,
    goal: str,
    model_id: str,
    repository: dict[str, Any],
    test_runners: list[str],
    max_tasks: int,
    task_cost_cap_usd: float,
    max_output_tokens: int,
    timeout_seconds: float,
    context_byte_cap: int = 48 * 1024,
) -> tuple[PlanProposal, ModelResult]:
    if not 1 <= max_tasks <= 50 or not test_runners or task_cost_cap_usd <= 0:
        raise PlanningError("invalid orchestration limits")
    required = {
        "goal": goal,
        "max_subtasks": max_tasks,
        "available_test_runners": test_runners,
        "rules": INSTRUCTIONS,
    }
    prompt = build_prompt(
        "orchestrator",
        required,
        snippets=[
            {"path": "<repository files>", "start_line": 1, "content": repository.get("files", "")},
            {"path": "README.md", "start_line": 1, "content": repository.get("readme", "")},
        ],
        byte_cap=context_byte_cap,
    )
    result = await provider.complete(
        messages=prompt.messages,
        schema=goal_schema(),
        model_id=model_id,
        max_output_tokens=max_output_tokens,
        timeout_seconds=timeout_seconds,
    )
    try:
        decomposition = GoalPlan.model_validate(result.structured_output)
    except ValidationError:
        raise PlanningError("planner output does not match the decomposition schema") from None
    return to_proposal(
        decomposition,
        project_id=project_id,
        goal=goal,
        test_runners=test_runners,
        max_tasks=max_tasks,
        task_cost_cap_usd=task_cost_cap_usd,
    ), result


def to_proposal(
    decomposition: GoalPlan,
    *,
    project_id: str,
    goal: str,
    test_runners: list[str],
    max_tasks: int,
    task_cost_cap_usd: float,
) -> PlanProposal:
    tasks = decomposition.tasks
    if not 1 <= len(tasks) <= max_tasks:
        raise PlanningError(f"planner proposed {len(tasks)} subtasks; allowed 1..{max_tasks}")
    ids: dict[str, str] = {}
    for index, task in enumerate(tasks, start=1):
        if task.key in ids:
            raise PlanningError(f"subtask {index} reuses another subtask key")
        ids[task.key] = f"task-{index}-{_slug(task.key or task.title)}"
    proposals: list[TaskProposal] = []
    for index, task in enumerate(tasks, start=1):
        if not task.files or any(not f.strip() for f in task.files):
            raise PlanningError(f"subtask {index} has no editable files")
        if not task.test_command or task.test_command[0] not in test_runners:
            raise PlanningError(f"subtask {index} test must start with an available test runner")
        missing = [d for d in task.depends_on if d not in ids]
        if missing or task.key in task.depends_on:
            raise PlanningError(f"subtask {index} has an unknown or self dependency")
        try:
            proposals.append(
                TaskProposal(
                    id=ids[task.key],
                    title=task.title.strip()[:200] or f"Subtask {index}",
                    description=task.instructions.strip()[:8000],
                    use_case=task.use_case,
                    complexity=task.complexity,
                    risk=task.risk,
                    # Earlier subtasks first when otherwise unordered.
                    agent_priority=max(10, 90 - 5 * (index - 1)),
                    manual_priority=None,
                    dependencies=sorted({ids[d] for d in task.depends_on}),
                    candidate_paths=[f.strip().removeprefix("./") for f in task.files],
                    acceptance=[Acceptance(kind="test", command=task.test_command)],
                    limits=TaskLimits(
                        max_tool_calls=100,
                        max_output_tokens=16_000,
                        timeout_seconds=900,
                        cost_cap_usd=task_cost_cap_usd,
                    ),
                )
            )
        except ValidationError:
            raise PlanningError(f"subtask {index} violates task limits") from None
    try:
        return PlanProposal(
            schema_version="1.0",
            project_id=project_id,
            idea=goal,
            assumptions=[a[:1000] for a in decomposition.assumptions[:20]],
            requirements=[goal],
            architecture=Architecture(
                recommended=(decomposition.summary.strip() or "Planned by Nova main agent.")
                + " (Proposed by the Nova main agent.)",
                alternatives=[],
                risks=[r[:1000] for r in decomposition.risks[:20]],
            ),
            tasks=proposals,
        )
    except ValidationError:
        raise PlanningError("planner output does not form a valid plan") from None

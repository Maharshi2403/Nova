import asyncio
import copy
from typing import Any

import pytest

from harness_ai.orchestrator import PlanningError, goal_schema, plan_goal
from harness_ai.providers.mock import MockModelProvider
from harness_ai.providers.openai import strict_schema

RUNNERS = ["/usr/bin/python3", "/usr/bin/git"]

DECOMPOSITION: dict[str, Any] = {
    "summary": "Add a greeting module, then a CLI that uses it.",
    "assumptions": ["Python 3 is available"],
    "risks": [],
    "tasks": [
        {
            "key": "greeting",
            "title": "Greeting function",
            "instructions": "Create greeting.py with greet(name) returning 'Hello, {name}!'.",
            "use_case": "coding",
            "complexity": "low",
            "risk": "low",
            "depends_on": [],
            "files": ["./greeting.py"],
            "test_command": ["/usr/bin/python3", "-c", "from greeting import greet"],
        },
        {
            "key": "cli",
            "title": "Command line entry point",
            "instructions": "Create cli.py printing greet(sys.argv[1]) using greeting.greet.",
            "use_case": "coding",
            "complexity": "medium",
            "risk": "low",
            "depends_on": ["greeting"],
            "files": ["cli.py", ".github/"],
            "test_command": ["/usr/bin/python3", "-c", "import cli"],
        },
    ],
}


def plan(decomposition: dict[str, Any], **overrides: Any) -> Any:
    arguments: dict[str, Any] = {
        "project_id": "proj-demo",
        "goal": "Build a greeting CLI",
        "model_id": "mock-1",
        "repository": {"files": "README.md\n", "readme": "demo"},
        "test_runners": RUNNERS,
        "max_tasks": 8,
        "task_cost_cap_usd": 0.25,
        "max_output_tokens": 4000,
        "timeout_seconds": 30,
    }
    arguments.update(overrides)
    return asyncio.run(plan_goal(MockModelProvider(default=decomposition), **arguments))


def test_goal_becomes_ordered_subtasks_with_routing_metadata() -> None:
    proposal, _ = plan(DECOMPOSITION)
    first, second = proposal.tasks
    assert first.id == "task-1-greeting" and second.id == "task-2-cli"
    assert second.dependencies == ["task-1-greeting"]
    assert (first.complexity, second.complexity) == ("low", "medium")
    assert first.candidate_paths == ["greeting.py"]
    assert second.candidate_paths == ["cli.py", ".github/"]
    assert first.acceptance[0].command == DECOMPOSITION["tasks"][0]["test_command"]
    assert all(t.limits.cost_cap_usd == 0.25 for t in proposal.tasks)
    assert first.agent_priority > second.agent_priority


def mutated(change: Any) -> dict[str, Any]:
    value = copy.deepcopy(DECOMPOSITION)
    change(value)
    return value


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (lambda d: d["tasks"][0].update(test_command=["sh", "-c", "true"]), "test runner"),
        (lambda d: d["tasks"][0].update(test_command=[]), "test runner"),
        (lambda d: d["tasks"][1].update(depends_on=["missing"]), "dependency"),
        (lambda d: d["tasks"][1].update(depends_on=["cli"]), "dependency"),
        (lambda d: d["tasks"][0].update(files=[]), "editable files"),
        (lambda d: d["tasks"][1].update(key="greeting", depends_on=[]), "reuses"),
        (lambda d: d.update(tasks=[]), "subtasks"),
    ],
)
def test_unsafe_or_incoherent_decompositions_are_rejected(change: Any, message: str) -> None:
    with pytest.raises(PlanningError, match=message):
        plan(mutated(change))


def test_task_count_is_capped() -> None:
    with pytest.raises(PlanningError, match="allowed 1..1"):
        plan(DECOMPOSITION, max_tasks=1)


def test_schema_mismatch_does_not_echo_model_output() -> None:
    with pytest.raises(PlanningError) as error:
        plan({"tasks": "SECRET-MODEL-TEXT"})
    assert "SECRET" not in str(error.value)


def test_schema_is_strict_mode_compatible() -> None:
    schema = strict_schema(goal_schema())
    text = str(schema)
    for keyword in ("maxLength", "minLength", "default", "maxItems"):
        assert keyword not in text

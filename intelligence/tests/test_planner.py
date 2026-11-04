import asyncio
import copy
import json
from pathlib import Path
from typing import Any

import jsonschema
import pytest
from pydantic import ValidationError

from harness_ai.plan import PlanProposal
from harness_ai.planner import PlanningError, propose_plan
from harness_ai.providers.mock import MockModelProvider

ROOT = Path(__file__).resolve().parents[2]


def load_plan() -> dict[str, Any]:
    data: dict[str, Any] = json.loads(
        (ROOT / "tests/fixtures/plans/valid_plan.json").read_text(encoding="utf-8")
    )
    return data


def plan_with(mutate: Any) -> dict[str, Any]:
    data = copy.deepcopy(load_plan())
    mutate(data)
    return data


def run(provider: MockModelProvider) -> Any:
    return asyncio.run(
        propose_plan(
            provider, project_id="proj-demo", idea="Build an orders API", model_id="mock-1"
        )
    )


def test_planner_fixture_valid() -> None:
    plan, result = run(MockModelProvider(default=load_plan()))
    assert [t.id for t in plan.tasks] == ["task-model", "task-endpoint"]
    assert plan.tasks[1].dependencies == ["task-model"]
    assert result.provider == "mock" and result.usage.source == "mock"


def test_fixture_matches_json_schema() -> None:
    schemas = ROOT / "schemas"
    store = {
        p.name: json.loads(p.read_text(encoding="utf-8")) for p in schemas.glob("*.schema.json")
    }
    from referencing import Registry, Resource

    registry: Registry[Any] = Registry().with_resources(
        (store[n]["$id"].rsplit("/", 1)[0] + "/" + n, Resource.from_contents(store[n]))
        for n in store
    )
    jsonschema.Draft202012Validator(store["plan.schema.json"], registry=registry).validate(
        load_plan()
    )


def test_planner_rejects_missing_acceptance() -> None:
    def drop(d: dict[str, Any]) -> None:
        d["tasks"][0]["acceptance"] = []

    with pytest.raises(PlanningError):
        run(MockModelProvider(default=plan_with(drop)))

    def drop_key(d: dict[str, Any]) -> None:
        del d["tasks"][1]["acceptance"]

    with pytest.raises(PlanningError):
        run(MockModelProvider(default=plan_with(drop_key)))


@pytest.mark.parametrize(
    "mutate",
    [
        lambda d: d["tasks"][1].update(dependencies=["ghost"]),
        lambda d: d["tasks"][1].update(id="task-model"),
        lambda d: d["tasks"][0].update(dependencies=["task-model"]),
        lambda d: d["tasks"][0].update(dependencies=["a", "a"]),
        lambda d: d["tasks"][0].update(complexity="extreme"),
        lambda d: d["tasks"][0].update(agent_priority=101),
        lambda d: d["tasks"][0].update(surprise=True),
        lambda d: d["tasks"][0]["acceptance"].append({"kind": "test"}),
        lambda d: d["tasks"][0]["acceptance"].append({"kind": "assertion"}),
        lambda d: d.update(requirements=[]),
        lambda d: d.update(schema_version="2.0"),
        lambda d: d.update(tasks=[]),
        lambda d: d["tasks"][0]["limits"].update(timeout_seconds=0),
    ],
)
def test_invalid_plans_rejected(mutate: Any) -> None:
    with pytest.raises(ValidationError):
        PlanProposal.model_validate(plan_with(mutate))

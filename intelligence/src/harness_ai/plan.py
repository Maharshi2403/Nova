"""Typed plan proposals. Mirrors schemas/plan.schema.json and schemas/task.schema.json."""

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, StringConstraints, model_validator

Level = Literal["low", "medium", "high"]
UseCase = Literal["coding", "planning", "review", "debugging", "architecture", "fast"]
NonEmpty = Annotated[str, StringConstraints(min_length=1)]


class Acceptance(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    kind: Literal["test", "assertion"]
    command: list[Annotated[str, StringConstraints(max_length=1024)]] | None = Field(
        default=None, min_length=1, max_length=50
    )
    description: str | None = Field(default=None, min_length=1, max_length=1000)

    @model_validator(mode="after")
    def _check_kind(self) -> Acceptance:
        if self.kind == "test" and self.command is None:
            raise ValueError("a test acceptance criterion requires a command")
        if self.kind == "assertion" and self.description is None:
            raise ValueError("an assertion acceptance criterion requires a description")
        return self


class TaskLimits(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    max_tool_calls: int = Field(ge=1, le=1000)
    max_output_tokens: int = Field(ge=1, le=1_000_000)
    timeout_seconds: int = Field(ge=1, le=86_400)
    cost_cap_usd: float = Field(ge=0)


class TaskProposal(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    id: str = Field(min_length=1, max_length=128)
    title: str = Field(min_length=3, max_length=200)
    description: str = Field(min_length=10, max_length=8000)
    use_case: UseCase = "coding"
    complexity: Level
    risk: Level
    agent_priority: int = Field(ge=0, le=100)
    manual_priority: int | None = Field(ge=0, le=100)
    dependencies: list[NonEmpty]
    candidate_paths: list[Annotated[str, StringConstraints(min_length=1, max_length=1024)]] = Field(
        max_length=100
    )
    acceptance: list[Acceptance] = Field(min_length=1)
    limits: TaskLimits

    @model_validator(mode="after")
    def _check_dependencies(self) -> TaskProposal:
        if len(set(self.dependencies)) != len(self.dependencies):
            raise ValueError("dependencies must be unique")
        if self.id in self.dependencies:
            raise ValueError("a task cannot depend on itself")
        return self


class Architecture(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    recommended: str = Field(min_length=10)
    alternatives: list[str]
    risks: list[str]


class PlanProposal(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    schema_version: Literal["1.0"]
    project_id: str = Field(min_length=1)
    idea: str = Field(min_length=5, max_length=16_000)
    assumptions: list[str] = Field(max_length=100)
    requirements: list[NonEmpty] = Field(min_length=1)
    architecture: Architecture
    tasks: list[TaskProposal] = Field(min_length=1, max_length=1000)

    @model_validator(mode="after")
    def _check_graph_references(self) -> PlanProposal:
        ids = [t.id for t in self.tasks]
        if len(set(ids)) != len(ids):
            raise ValueError("task ids must be unique")
        known = set(ids)
        for task in self.tasks:
            missing = [d for d in task.dependencies if d not in known]
            if missing:
                raise ValueError(f"task {task.id} depends on unknown task(s) {missing}")
        return self

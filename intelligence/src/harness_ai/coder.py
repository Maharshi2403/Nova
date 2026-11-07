"""Bounded structured coder. All repository tools and provider spend need Rust approval.

Uses the existing Responses adapter, never an SDK session or a database connection.
Contract: https://developers.openai.com/api/docs/guides/structured-outputs
"""

from __future__ import annotations

import asyncio
import json
from collections.abc import Callable
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from harness_ai.assignment import Completed, Failed, StartTask, Usage
from harness_ai.broker_stdio import Tool, call_tool
from harness_ai.context import build_prompt, task_handoff
from harness_ai.memory import WorkingMemory
from harness_ai.plan import TaskProposal
from harness_ai.providers.base import (
    ModelProvider,
    PermanentProviderError,
    ProviderError,
    estimate_cost,
)
from harness_ai.providers.retry import RetryingProvider


class Action(BaseModel):
    model_config = ConfigDict(extra="forbid")
    action: Literal["tool", "completed", "failed"]
    tool: str | None
    arguments_json: str | None
    summary: str = Field(max_length=2000)
    notes: str = Field(
        default="", max_length=2048, description="Optional scratchpad, at most 2048 UTF-8 bytes"
    )

    @field_validator("notes")
    @classmethod
    def notes_within_byte_cap(cls, value: str) -> str:
        if len(value.encode()) > 2048:
            raise ValueError("notes exceed 2 KiB")
        return value


Emit = Callable[[str, dict[str, Any]], None]


async def code_task(assignment: StartTask, provider: ModelProvider, tool: Tool, emit: Emit) -> None:
    """One bounded assignment, including retries. Unknown usage prevents further calls."""
    heartbeat: asyncio.Task[None] | None = None

    async def pulse() -> None:
        while True:
            await asyncio.sleep(2)
            emit("heartbeat", {})

    try:
        description = await call_tool(tool, "task.describe", {})
        if not description.ok or not isinstance(description.result, dict):
            raise PermanentProviderError("task context denied")
        data = dict(description.result)
        runtime = data.pop("runtime")
        task = TaskProposal.model_validate(data)
        required = task_handoff(task)
        required["available_tools"] = assignment.allowed_tools
        required["tool_arguments"] = {
            "repo.read": "path, optional start_line/max_lines",
            "repo.patch": "path and content, or path and old/new exact replacement",
            "repo.glob": "pattern",
            "repo.search": "query, optional path",
            "cmd.run": "argv array, optional cwd; human approval may be required",
        }
        required["action_contract"] = (
            "Choose a tool action with tool name and JSON-encoded arguments, or completed/failed. "
            "Inspect relevant files before editing. Preserve exact acceptance and assigned scope. "
            "Never request extra privilege. If approval is pending, return failed with its ID."
        )
        remaining = min(assignment.max_output_tokens, int(runtime["max_output_tokens"]))
        context_cap = min(32768, int(runtime["context_byte_cap"]))
        required_bytes = build_prompt("coder", required, byte_cap=context_cap).byte_size
        # History is serialized inside two JSON strings; reserve worst-case escaping.
        available = (context_cap - required_bytes - 1024) // 4
        if available < 1024:
            raise ValueError("context cannot fit bounded working memory")
        memory = WorkingMemory(byte_cap=min(24 * 1024, available))
        changed: list[str] = []
        schema = Action.model_json_schema()
        max_model_turns = assignment.max_model_turns or assignment.max_tool_calls
        heartbeat = asyncio.create_task(pulse())
        async with asyncio.timeout(assignment.timeout_seconds):
            for turn in range(max_model_turns):
                if turn == max_model_turns - 1:
                    required["turn_budget"] = (
                        "This is the final model turn. Return completed or failed; "
                        "do not request a tool."
                    )
                prompt = build_prompt(
                    "coder",
                    required,
                    previous_summary=memory.render(),
                    previous_summary_byte_cap=memory.byte_cap,
                    byte_cap=min(32768, int(runtime["context_byte_cap"])),
                )
                limit = min(remaining, int(runtime["max_output_tokens"]))
                if limit <= 0:
                    raise PermanentProviderError("output budget exhausted")

                bound = prompt.byte_size + len(json.dumps(schema).encode()) + 512

                async def authorize(
                    model: str, attempt: int, output_limit: int = limit, input_bound: int = bound
                ) -> bool:
                    response = await call_tool(
                        tool,
                        "model.authorize",
                        {
                            "model_id": model,
                            "max_output_tokens": output_limit,
                            "input_byte_bound": input_bound,
                            "retry_attempt": attempt,
                        },
                    )
                    return response.ok

                retried = RetryingProvider(provider, authorize=authorize)
                result = await retried.complete(
                    messages=prompt.messages,
                    schema=schema,
                    model_id=assignment.model.model_id,
                    max_output_tokens=limit,
                    timeout_seconds=min(120, assignment.timeout_seconds),
                )
                usage = result.usage
                cost = estimate_cost(usage, runtime)
                emit(
                    "usage",
                    Usage(
                        provider=result.provider,
                        model_id=result.model_id,
                        input_tokens=usage.input_tokens,
                        output_tokens=usage.output_tokens,
                        cached_input_tokens=usage.cached_input_tokens,
                        cost_usd=cost,
                        price_source="registry_estimate" if cost is not None else "unknown",
                    ).model_dump(),
                )
                action = Action.model_validate(result.structured_output)
                if action.action == "completed":
                    emit(
                        "completed",
                        Completed(
                            summary=action.summary, changed_paths=changed, claimed_tests=[]
                        ).model_dump(),
                    )
                    return
                if action.action == "failed":
                    emit("failed", Failed(reason=action.summary, retryable=False).model_dump())
                    return
                if action.tool not in assignment.allowed_tools or action.tool in (
                    "task.describe",
                    "model.authorize",
                    "model.complete",
                ):
                    raise PermanentProviderError("model requested unauthorized tool")
                arguments = json.loads(action.arguments_json or "null")
                if not isinstance(arguments, dict):
                    raise PermanentProviderError("tool arguments must be an object")
                response = await call_tool(tool, str(action.tool), arguments)
                if response.ok and action.tool == "repo.patch":
                    path = str(arguments.get("path", ""))
                    if path not in changed:
                        changed.append(path)
                memory.append(
                    action.action,
                    str(action.tool),
                    arguments,
                    response.model_dump(),
                    action.summary,
                    notes=action.notes,
                )
                if usage.output_tokens is None:
                    raise PermanentProviderError("unknown output usage prevents another model call")
                remaining -= usage.output_tokens
        emit("failed", Failed(reason="turn budget exhausted", retryable=True).model_dump())
    except (ProviderError, ValueError, KeyError, TypeError, TimeoutError) as exc:
        # Provider errors carry fixed messages (never bodies, prompts or credentials).
        # Other exceptions may embed model output or task data, so report only their class.
        if isinstance(exc, ProviderError):
            cause = f"provider: {str(exc)[:200]}"
        elif isinstance(exc, TimeoutError):
            cause = "task timeout"
        else:
            cause = f"invalid model action ({type(exc).__name__})"
        emit("failed", Failed(reason=f"coder stopped: {cause}", retryable=False).model_dump())
    finally:
        if heartbeat is not None:
            heartbeat.cancel()
            try:
                await heartbeat
            except asyncio.CancelledError:
                pass

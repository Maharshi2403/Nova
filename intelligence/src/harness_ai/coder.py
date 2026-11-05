"""Bounded structured coder. All repository tools and provider spend need Rust approval.

Uses the existing Responses adapter, never an SDK session or a database connection.
Contract: https://developers.openai.com/api/docs/guides/structured-outputs
"""

from __future__ import annotations

import asyncio
import json
from collections.abc import Callable
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

from harness_ai.assignment import Completed, Failed, StartTask, ToolResponse, Usage
from harness_ai.context import build_prompt, task_handoff
from harness_ai.plan import TaskProposal
from harness_ai.providers.base import ModelProvider, PermanentProviderError, ProviderError
from harness_ai.providers.retry import RetryingProvider


class Action(BaseModel):
    model_config = ConfigDict(extra="forbid")
    action: Literal["tool", "completed", "failed"]
    tool: str | None
    arguments_json: str | None
    summary: str = Field(max_length=2000)


Tool = Callable[[str, dict[str, Any]], ToolResponse]
Emit = Callable[[str, dict[str, Any]], None]


async def code_task(assignment: StartTask, provider: ModelProvider, tool: Tool, emit: Emit) -> None:
    """One bounded assignment, including retries. Unknown usage prevents further calls."""
    heartbeat: asyncio.Task[None] | None = None

    async def pulse() -> None:
        while True:
            await asyncio.sleep(2)
            emit("heartbeat", {})

    try:
        description = tool("task.describe", {})
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
        history = ""
        changed: list[str] = []
        schema = Action.model_json_schema()
        heartbeat = asyncio.create_task(pulse())
        async with asyncio.timeout(assignment.timeout_seconds):
            for _turn in range(min(16, assignment.max_tool_calls)):
                prompt = build_prompt(
                    "coder",
                    required,
                    previous_summary=history,
                    byte_cap=min(32768, int(runtime["context_byte_cap"])),
                )
                limit = min(remaining, int(runtime["max_output_tokens"]))
                if limit <= 0:
                    raise PermanentProviderError("output budget exhausted")

                bound = prompt.byte_size + len(json.dumps(schema).encode()) + 512

                async def authorize(
                    model: str, attempt: int, output_limit: int = limit, input_bound: int = bound
                ) -> bool:
                    response = tool(
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
                cost = None
                if usage.input_tokens is not None and usage.output_tokens is not None:
                    cost = (
                        usage.input_tokens * float(runtime["input_usd_per_million_tokens"])
                        + usage.output_tokens * float(runtime["output_usd_per_million_tokens"])
                    ) / 1_000_000
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
                ):
                    raise PermanentProviderError("model requested unauthorized tool")
                arguments = json.loads(action.arguments_json or "null")
                if not isinstance(arguments, dict):
                    raise PermanentProviderError("tool arguments must be an object")
                response = tool(str(action.tool), arguments)
                if response.ok and action.tool == "repo.patch":
                    path = str(arguments.get("path", ""))
                    if path not in changed:
                        changed.append(path)
                history = json.dumps(
                    {
                        "last_action": {"tool": action.tool, "summary": action.summary},
                        "tool_response": response.model_dump(),
                    },
                    ensure_ascii=False,
                )
                # Prompt compaction keeps only a bounded handoff, never a full transcript.
                if usage.output_tokens is None:
                    raise PermanentProviderError("unknown output usage prevents another model call")
                remaining -= usage.output_tokens
        raise PermanentProviderError("coder turn limit exhausted")
    except (ProviderError, ValueError, KeyError, TypeError, TimeoutError):
        # Never echo provider bodies, task contents, Pydantic inputs or credentials.
        emit(
            "failed",
            Failed(
                reason="coder stopped by provider, context, protocol or budget policy",
                retryable=False,
            ).model_dump(),
        )
    finally:
        if heartbeat is not None:
            heartbeat.cancel()
            try:
                await heartbeat
            except asyncio.CancelledError:
                pass

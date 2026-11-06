"""One credential-scoped provider invocation, supervised by Rust over versioned JSONL."""

from __future__ import annotations

import asyncio
import os
import sys
from typing import Any, BinaryIO, cast

from harness_ai.assignment import ToolResponse, _envelope
from harness_ai.framing import read_frame, write_frame
from harness_ai.handshake import build_ack
from harness_ai.orchestrator import PlanningError, plan_goal
from harness_ai.providers.base import (
    Message,
    ModelResult,
    PermanentProviderError,
    TransientProviderError,
)
from harness_ai.providers.multi import JSONProvider, Kind
from harness_ai.providers.openai import OpenAIProvider


class BrokerProvider:
    """Coder never receives credentials; Rust validates and forwards each complete request."""

    def __init__(self, tool: Any) -> None:
        self.tool = tool

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        response: ToolResponse = self.tool(
            "model.complete",
            {
                "messages": messages,
                "schema": schema,
                "model_id": model_id,
                "max_output_tokens": max_output_tokens,
                "timeout_seconds": timeout_seconds,
            },
        )
        if not response.ok:
            # "<category>: <detail>" where detail is a fixed or sanitized message.
            category, _, detail = (response.error or "unknown").partition(": ")
            reason = (detail or category)[:200]
            if category == "transient_provider":
                raise TransientProviderError(reason)
            raise PermanentProviderError(reason)
        return ModelResult.model_validate(response.result)


async def _plan_goal(
    kind: str, name: str, key: str, base: str, enabled: bool, request: Any
) -> dict[str, Any]:
    """Main-agent decomposition. Rust supplies model, caps and bounded repository context."""
    provider: Any = (
        OpenAIProvider(enabled=enabled, api_key=key)
        if kind == "openai"
        else JSONProvider(kind=cast(Kind, kind), enabled=enabled, api_key=key, base_url=base)
    )
    plan, result = await plan_goal(provider, **request.payload["arguments"])
    return {
        "plan": plan.model_dump(mode="json", exclude_none=True),
        "provider": name,
        "model_id": result.model_id,
        "usage": result.usage.model_dump(mode="json"),
    }


def _detail(exc: BaseException) -> str:
    # Provider errors carry fixed messages plus at most a sanitized provider error code.
    if isinstance(exc, TimeoutError):
        return "provider timeout"
    return str(exc)[:200]


def serve_provider(stdin: BinaryIO, stdout: BinaryIO) -> int:
    hello = read_frame(stdin)
    if hello is None:
        return 0
    write_frame(stdout, build_ack(hello))
    request = read_frame(stdin)
    if (
        request is None
        or request.kind != "tool_request"
        or request.payload.get("tool") not in ("model.complete", "models.list", "plan.goal")
    ):
        return 2
    try:
        kind = os.environ.get("NOVA_PROVIDER_KIND", "openai")
        name = os.environ.get("NOVA_PROVIDER_NAME", kind)
        base = os.environ.get(
            "NOVA_PROVIDER_BASE_URL",
            "https://api.openai.com/v1" if kind == "openai" else "https://api.anthropic.com/v1",
        )
        key = os.environ.get("NOVA_PROVIDER_API_KEY", os.environ.get("OPENAI_API_KEY", ""))
        enabled = os.environ.get("HARNESS_ENABLE_LIVE") == "1"
        if request.payload["tool"] == "models.list":
            catalog = JSONProvider(
                kind=cast(Kind, kind), enabled=enabled, api_key=key, base_url=base
            )
            response = ToolResponse(ok=True, result=asyncio.run(catalog.discover()))
        elif request.payload["tool"] == "plan.goal":
            response = ToolResponse(
                ok=True, result=asyncio.run(_plan_goal(kind, name, key, base, enabled, request))
            )
        else:
            provider = (
                OpenAIProvider(enabled=enabled, api_key=key)
                if kind == "openai"
                else JSONProvider(
                    kind=cast(Kind, kind), enabled=enabled, api_key=key, base_url=base
                )
            )
            result = asyncio.run(provider.complete(**request.payload["arguments"]))
            response = ToolResponse(
                ok=True, result=result.model_copy(update={"provider": name}).model_dump()
            )
    except PlanningError as exc:
        response = ToolResponse(ok=False, error=f"permanent_provider: {str(exc)[:200]}")
    except (TransientProviderError, TimeoutError) as exc:
        response = ToolResponse(ok=False, error=f"transient_provider: {_detail(exc)}")
    except PermanentProviderError as exc:
        response = ToolResponse(ok=False, error=f"permanent_provider: {_detail(exc)}")
    except (ValueError, TypeError, KeyError) as exc:
        # These may embed request values; report only the exception class.
        response = ToolResponse(ok=False, error=f"permanent_provider: adapter {type(exc).__name__}")
    write_frame(
        stdout, _envelope(request, "tool_response", response.model_dump(), request.message_id)
    )
    return 0


def run_stdio() -> int:
    output = sys.stdout.buffer
    sys.stdout = sys.stderr
    return serve_provider(sys.stdin.buffer, output)

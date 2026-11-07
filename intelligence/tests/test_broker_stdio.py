"""The worker must remain responsive while Rust owns an outstanding call."""

import asyncio
import io
import os
from typing import Any, BinaryIO, cast

import pytest

from harness_ai.assignment import _envelope
from harness_ai.broker_stdio import AsyncBroker
from harness_ai.framing import read_frame, write_frame
from harness_ai.provider_stdio import BrokerProvider
from test_assignment import _start


def test_heartbeat_continues_during_slow_model_call() -> None:
    async def exercise() -> None:
        start = _start()
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, "rb") as source, os.fdopen(write_fd, "wb") as peer:
            output = io.BytesIO()
            broker = AsyncBroker(start, source, output)
            pulses = 0

            async def pulse() -> None:
                nonlocal pulses
                while True:
                    await asyncio.sleep(0.01)
                    pulses += 1

            heartbeat = asyncio.create_task(pulse())
            call = asyncio.create_task(
                BrokerProvider(broker.tool).complete(
                    messages=[], schema={}, model_id="m", max_output_tokens=10, timeout_seconds=5
                )
            )
            await asyncio.sleep(0.08)
            assert pulses >= 2
            request = read_frame(io.BytesIO(output.getvalue()))
            assert request is not None
            write_frame(
                peer,
                _envelope(
                    start,
                    "tool_response",
                    {
                        "ok": True,
                        "result": {
                            "structured_output": {},
                            "usage": {},
                            "provider": "mock",
                            "model_id": "m",
                        },
                    },
                    request.message_id,
                ),
            )
            assert (await call).model_id == "m"
            heartbeat.cancel()
            with pytest.raises(asyncio.CancelledError):
                await heartbeat
        await asyncio.sleep(0)

    asyncio.run(exercise())


def test_asyncio_timeout_interrupts_model_wait() -> None:
    async def exercise() -> None:
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, "rb") as source, os.fdopen(write_fd, "wb"):
            broker = AsyncBroker(_start(), source, io.BytesIO())
            with pytest.raises(TimeoutError):
                async with asyncio.timeout(0.03):
                    await BrokerProvider(broker.tool).complete(
                        messages=[],
                        schema={},
                        model_id="m",
                        max_output_tokens=10,
                        timeout_seconds=5,
                    )
            assert not broker.pending
        await asyncio.sleep(0)

    asyncio.run(exercise())


def test_cancel_frame_during_tool_exits_cleanly() -> None:
    # Exercise the actual worker entry point, including its exit code and terminal frame.
    import subprocess
    import sys

    from harness_ai.framing import encode_frame

    start = _start("openai")
    payload: dict[str, Any] = dict(start.payload)
    payload["allowed_tools"] = ["model.complete", "task.describe"]
    start = start.model_copy(update={"payload": payload})
    process = subprocess.Popen(
        [sys.executable, "-m", "harness_ai", "worker"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdin is not None and process.stdout is not None
    source = cast(BinaryIO, process.stdout)
    target = cast(BinaryIO, process.stdin)
    try:
        # Use the same handshake envelope as other worker fixtures.
        hello = _envelope(start, "hello", {"supported_versions": ["1.0"]})
        process.stdin.write(encode_frame(hello))
        process.stdin.flush()
        assert read_frame(source) is not None
        write_frame(target, start)
        request = read_frame(source)
        assert request is not None and request.kind == "tool_request"
        write_frame(
            target,
            _envelope(start, "cancel", {}).model_copy(
                update={"run_id": None, "task_id": None, "attempt": None}
            ),
        )
        terminal = read_frame(source)
        assert terminal is not None and terminal.kind == "failed"
        assert terminal.payload["reason"] == "cancelled"
        assert process.wait(timeout=3) == 0
    finally:
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=3)

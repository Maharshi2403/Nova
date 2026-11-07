"""Non-blocking worker broker: one reader owns stdin, futures own requests."""

from __future__ import annotations

import asyncio
import io
import os
import select
import threading
from collections.abc import Awaitable, Callable
from typing import Any, BinaryIO

from harness_ai.assignment import AssignmentError, ToolResponse, _envelope
from harness_ai.framing import MAX_FRAME_BYTES, FrameError, read_frame, write_frame
from harness_ai.protocol import Envelope

Tool = Callable[[str, dict[str, Any]], ToolResponse | Awaitable[ToolResponse]]


async def call_tool(tool: Tool, name: str, arguments: dict[str, Any]) -> ToolResponse:
    result = tool(name, arguments)
    if isinstance(result, ToolResponse):
        return result
    return await result


class AsyncBroker:
    def __init__(self, start: Envelope, stdin: BinaryIO, stdout: BinaryIO) -> None:
        self.start = start
        self.stdin = stdin
        self.stdout = stdout
        self.loop = asyncio.get_running_loop()
        self.owner = asyncio.current_task()
        self.pending: dict[str, asyncio.Future[ToolResponse]] = {}
        self.started = False
        self.closed = False
        self.stopped = threading.Event()
        self.reader: threading.Thread | None = None

    async def tool(self, name: str, arguments: dict[str, Any]) -> ToolResponse:
        if self.closed:
            raise AssignmentError("broker stream closed")
        request = _envelope(self.start, "tool_request", {"tool": name, "arguments": arguments})
        key = str(request.message_id)
        future: asyncio.Future[ToolResponse] = self.loop.create_future()
        self.pending[key] = future
        try:
            write_frame(self.stdout, request)
            if not self.started:
                self.started = True
                self.reader = threading.Thread(target=self._read, daemon=True, name="broker-reader")
                self.reader.start()
            return await future
        finally:
            self.pending.pop(key, None)

    def _read(self) -> None:
        try:
            buffer = bytearray()
            while not self.stopped.is_set():
                if b"\n" not in buffer:
                    if os.name != "nt" and not select.select([self.stdin], [], [], 0.1)[0]:
                        continue
                    chunk = os.read(self.stdin.fileno(), 65536)
                    if not chunk:
                        self.loop.call_soon_threadsafe(self._receive, None)
                        return
                    buffer.extend(chunk)
                end = buffer.find(b"\n")
                if end < 0:
                    if len(buffer) > MAX_FRAME_BYTES:
                        raise FrameError("oversize frame")
                    continue
                if end > MAX_FRAME_BYTES:
                    raise FrameError("oversize frame")
                raw = bytes(buffer[: end + 1])
                del buffer[: end + 1]
                frame = read_frame(io.BytesIO(raw))
                self.loop.call_soon_threadsafe(self._receive, frame)
                if frame is None or frame.kind == "cancel":
                    return
        except (FrameError, ValueError, OSError):
            try:
                self.loop.call_soon_threadsafe(self._receive, None)
            except RuntimeError:
                pass  # Event loop already exited with the assignment.
        except RuntimeError:
            pass  # Assignment finished while the daemon reader was waiting.

    async def close(self) -> None:
        self.stopped.set()
        if self.reader is not None:
            await asyncio.to_thread(self.reader.join, 0.3)

    def _receive(self, frame: Envelope | None) -> None:
        if (
            frame is not None
            and frame.kind == "cancel"
            and (frame.run_id, frame.task_id, frame.attempt) == (None, None, None)
        ):
            self.closed = True
            if self.owner is not None:
                self.owner.cancel()
            return
        if frame is not None and (frame.run_id, frame.task_id, frame.attempt) != (
            self.start.run_id,
            self.start.task_id,
            self.start.attempt,
        ):
            frame = None
        if frame is not None and frame.kind == "cancel":
            self.closed = True
            if self.owner is not None:
                self.owner.cancel()
            return
        if frame is not None and frame.kind == "tool_response":
            future = self.pending.get(frame.correlation_id or "")
            if future is not None and not future.done():
                try:
                    response = ToolResponse.model_validate(frame.payload)
                except ValueError:
                    future.set_exception(AssignmentError("invalid tool response"))
                else:
                    future.set_result(response)
                return
        self.closed = True
        for future in self.pending.values():
            if not future.done():
                future.set_exception(AssignmentError("missing or mismatched tool response"))

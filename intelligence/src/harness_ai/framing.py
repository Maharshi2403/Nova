"""Bounded JSONL framing for worker IPC (mirrors harness-core::framing).

One envelope per line, at most MAX_FRAME_BYTES excluding the newline. stdout carries
protocol frames only; diagnostics belong on stderr.
"""

from __future__ import annotations

from typing import BinaryIO

from pydantic import ValidationError

from harness_ai.protocol import Envelope

MAX_FRAME_BYTES = 1024 * 1024


class FrameError(Exception):
    """Base class for framing failures."""


class OversizeFrameError(FrameError):
    """A frame exceeded MAX_FRAME_BYTES; the stream cannot be resynchronised."""


class BadFrameError(FrameError):
    """Not UTF-8, not JSON, an empty line, or not a valid envelope."""


class TruncatedFrameError(FrameError):
    """The stream ended before the terminating newline."""


def encode_frame(envelope: Envelope) -> bytes:
    body = envelope.model_dump_json().encode("utf-8")
    if len(body) > MAX_FRAME_BYTES:
        raise OversizeFrameError(f"frame exceeds {MAX_FRAME_BYTES} bytes")
    return body + b"\n"


def write_frame(stream: BinaryIO, envelope: Envelope) -> None:
    stream.write(encode_frame(envelope))
    stream.flush()


def decode_frame(line: bytes) -> Envelope:
    if len(line) > MAX_FRAME_BYTES:
        raise OversizeFrameError(f"frame exceeds {MAX_FRAME_BYTES} bytes")
    try:
        text = line.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise BadFrameError(f"invalid UTF-8: {exc}") from exc
    if not text.strip():
        raise BadFrameError("empty frame")
    try:
        return Envelope.model_validate_json(text)
    except ValidationError as exc:
        raise BadFrameError(str(exc)) from exc


def read_frame(stream: BinaryIO) -> Envelope | None:
    """Read the next frame; None is a clean end of stream."""
    # Allow the body, a possible CR and the newline, so an oversize line is detected
    # without ever buffering more than the limit.
    line = stream.readline(MAX_FRAME_BYTES + 2)
    if not line:
        return None
    if not line.endswith(b"\n"):
        if len(line) > MAX_FRAME_BYTES:
            raise OversizeFrameError(f"frame exceeds {MAX_FRAME_BYTES} bytes")
        raise TruncatedFrameError("stream ended mid-frame")
    body = line[:-1]
    if body.endswith(b"\r"):
        body = body[:-1]
    return decode_frame(body)

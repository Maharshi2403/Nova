"""Worker process main loop: JSONL over stdin/stdout, diagnostics on stderr only."""

from __future__ import annotations

import sys
from typing import BinaryIO

from harness_ai.framing import FrameError, read_frame, write_frame
from harness_ai.handshake import HandshakeError, build_ack


def _log(message: str) -> None:
    print(f"harness_ai worker: {message}", file=sys.stderr, flush=True)


def serve(stdin: BinaryIO, stdout: BinaryIO) -> int:
    """Handshake, then process frames until EOF or `cancel`. Returns a process exit code."""
    try:
        first = read_frame(stdin)
    except FrameError as exc:
        _log(f"bad first frame: {exc}")
        return 2
    if first is None:
        return 0
    try:
        ack = build_ack(first)
    except HandshakeError as exc:
        _log(f"handshake rejected: {exc}")
        return 2
    write_frame(stdout, ack)

    while True:
        try:
            envelope = read_frame(stdin)
        except FrameError as exc:
            _log(f"protocol error: {exc}")
            return 2
        if envelope is None or envelope.kind == "cancel":
            return 0
        # Task handling arrives in later tickets; unknown work is logged, never guessed at.
        _log(f"ignoring {envelope.kind} (not implemented)")


def run_stdio() -> int:
    # Keep the real stdout for protocol frames and point `sys.stdout` at stderr so a stray
    # print() can never corrupt the stream.
    protocol_out = sys.stdout.buffer
    sys.stdout = sys.stderr
    return serve(sys.stdin.buffer, protocol_out)

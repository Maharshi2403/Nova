import io
from pathlib import Path

from harness_ai.framing import encode_frame
from harness_ai.protocol import Envelope
from harness_ai.worker import serve

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "protocol" / "valid"


def load(name: str) -> Envelope:
    return Envelope.model_validate_json((FIXTURES / name).read_text(encoding="utf-8"))


def run(data: bytes) -> tuple[int, bytes]:
    out = io.BytesIO()
    code = serve(io.BytesIO(data), out)
    return code, out.getvalue()


def test_hello_is_acknowledged_then_eof_exits_cleanly() -> None:
    hello = load("hello.json")
    code, out = run(encode_frame(hello))
    assert code == 0
    assert out.count(b"\n") == 1
    assert b'"hello_ack"' in out and str(hello.message_id).encode() in out


def test_cancel_stops_the_worker() -> None:
    hello = load("hello.json")
    cancel = hello.model_copy(update={"kind": "cancel"})
    code, out = run(encode_frame(hello) + encode_frame(cancel) + encode_frame(hello))
    assert code == 0
    assert out.count(b"\n") == 1


def test_garbage_and_non_hello_first_frames_fail() -> None:
    assert run(b"not json\n")[0] == 2
    code, out = run(encode_frame(load("start_task.json")))
    assert (code, out) == (2, b"")


def test_garbage_after_handshake_fails_without_output_noise() -> None:
    code, out = run(encode_frame(load("hello.json")) + b"{oops\n")
    assert code == 2
    assert out.count(b"\n") == 1

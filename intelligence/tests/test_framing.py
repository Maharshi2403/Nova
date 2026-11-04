import io
import json
from pathlib import Path

import pytest

from harness_ai.framing import (
    MAX_FRAME_BYTES,
    BadFrameError,
    OversizeFrameError,
    TruncatedFrameError,
    decode_frame,
    encode_frame,
    read_frame,
    write_frame,
)
from harness_ai.protocol import Envelope

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "protocol"


def load(name: str) -> Envelope:
    return Envelope.model_validate_json((FIXTURES / "valid" / name).read_text(encoding="utf-8"))


def test_roundtrip_multiple_frames() -> None:
    a, b = load("hello.json"), load("completed.json")
    buf = io.BytesIO()
    write_frame(buf, a)
    write_frame(buf, b)
    assert buf.getvalue().count(b"\n") == 2
    buf.seek(0)
    assert read_frame(buf) == a
    assert read_frame(buf) == b
    assert read_frame(buf) is None


def test_python_frame_is_valid_json_line() -> None:
    line = encode_frame(load("hello.json"))
    assert line.endswith(b"\n")
    assert json.loads(line)["kind"] == "hello"


@pytest.mark.parametrize("bad", [b"{not json", b"[]", b"null", b'{"kind":"hello"}', b"   "])
def test_rejects_bad_json(bad: bytes) -> None:
    with pytest.raises(BadFrameError):
        read_frame(io.BytesIO(bad + b"\n"))


def test_rejects_invalid_utf8_and_contract_violations() -> None:
    with pytest.raises(BadFrameError):
        read_frame(io.BytesIO(b"\xff\xfe\n"))
    invalid = (FIXTURES / "invalid" / "extra_field.json").read_bytes().strip()
    with pytest.raises(BadFrameError):
        read_frame(io.BytesIO(invalid + b"\n"))


def test_rejects_oversize_frame() -> None:
    with pytest.raises(OversizeFrameError):
        read_frame(io.BytesIO(b"a" * (MAX_FRAME_BYTES + 1) + b"\n"))
    with pytest.raises(OversizeFrameError):
        read_frame(io.BytesIO(b"a" * (MAX_FRAME_BYTES + 1)))
    with pytest.raises(OversizeFrameError):
        decode_frame(b"x" * (MAX_FRAME_BYTES + 1))


def test_exact_limit_passes_size_check() -> None:
    with pytest.raises(BadFrameError):
        read_frame(io.BytesIO(b" " * MAX_FRAME_BYTES + b"\n"))


def test_oversize_envelope_cannot_be_written() -> None:
    env = load("hello.json")
    big = env.model_copy(update={"payload": {"blob": "a" * MAX_FRAME_BYTES}})
    with pytest.raises(OversizeFrameError):
        encode_frame(big)


def test_truncated_stream_and_crlf() -> None:
    raw = (FIXTURES / "valid" / "hello.json").read_bytes().strip()
    with pytest.raises(TruncatedFrameError):
        read_frame(io.BytesIO(raw))
    assert read_frame(io.BytesIO(raw + b"\r\n")) is not None

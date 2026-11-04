import json
from pathlib import Path
from typing import Any

import pytest
from jsonschema import Draft202012Validator
from pydantic import ValidationError

from harness_ai.protocol import PROTOCOL_VERSION, Envelope

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures" / "protocol"


def _files(kind: str) -> list[Path]:
    files = sorted((FIXTURES / kind).glob("*.json"))
    assert files
    return files


def _validator() -> Draft202012Validator:
    schema = json.loads((ROOT / "schemas" / "worker-envelope.schema.json").read_text("utf-8"))
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def test_protocol_version_constant() -> None:
    assert PROTOCOL_VERSION == "1.0"


@pytest.mark.parametrize("path", _files("valid"), ids=lambda p: p.name)
def test_valid_fixture_round_trips(path: Path) -> None:
    raw: dict[str, Any] = json.loads(path.read_text("utf-8"))
    assert _validator().is_valid(raw)
    env = Envelope.model_validate(raw)
    dumped = json.loads(env.model_dump_json())
    assert dumped["message_id"] == raw["message_id"]
    assert dumped["kind"] == raw["kind"]
    assert dumped["payload"] == raw["payload"]
    assert Envelope.model_validate(dumped) == env


@pytest.mark.parametrize("path", _files("invalid"), ids=lambda p: p.name)
def test_invalid_fixture_rejected(path: Path) -> None:
    raw = json.loads(path.read_text("utf-8"))
    assert not _validator().is_valid(raw)
    with pytest.raises(ValidationError):
        Envelope.model_validate(raw)


def test_schema_rejects_wrong_version() -> None:
    raw = json.loads((FIXTURES / "invalid" / "wrong_version.json").read_text("utf-8"))
    with pytest.raises(ValidationError):
        Envelope.model_validate(raw)

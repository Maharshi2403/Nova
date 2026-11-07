import json

from harness_ai.memory import WorkingMemory


def test_memory_retains_changed_files_after_compaction() -> None:
    memory = WorkingMemory()
    memory.append(
        "tool", "repo.patch", {"path": "src/changed.py"}, {"ok": True, "result": {}}, "fix defect"
    )
    for index in range(20):
        memory.append(
            "tool",
            "repo.read",
            {"path": f"src/{index}.py", "start_line": 10},
            {"ok": True, "result": {"end_line": 20}},
            "inspect",
        )
    data = json.loads(memory.render())
    assert "src/changed.py" in data["summary"]["files_changed"]
    assert data["summary"]["files_read"][0]["start_line"] == 10
    assert len(data["recent"]) == 6


def test_memory_block_within_cap() -> None:
    for cap in [1024, 4096, 24576]:
        memory = WorkingMemory(byte_cap=cap)
        for index in range(80):
            memory.append(
                "tool",
                "cmd.run",
                {"argv": ["python", str(index)]},
                {"ok": True, "result": {"exit_code": index % 2, "stdout": '\\"🔒' * 10000}},
                "hypothesis" * 500,
                notes="scratch" * 500,
            )
        assert len(memory.render().encode()) <= cap
        assert len(json.loads(memory.render())["recent"]) == 6


def test_last_k_turns_verbatim() -> None:
    memory = WorkingMemory()
    expected = []
    for index in range(12):
        memory.append(
            "tool",
            "repo.read",
            {"path": f"{index}.py"},
            {"ok": True, "result": {"content": f"result {index}"}},
            f"step {index}",
        )
        expected.append(json.loads(memory.render())["recent"][-1])
    assert json.loads(memory.render())["recent"] == expected[-6:]

    for index, record in zip(range(6, 12), json.loads(memory.render())["recent"], strict=True):
        assert record["summary"] == f"step {index}"
        assert json.loads(record["args_summary"]) == {"path": f"{index}.py"}
        assert json.loads(record["result_excerpt"]) == {
            "ok": True,
            "result": {"content": f"result {index}"},
        }


def test_compaction_preserves_command_exit_codes() -> None:
    memory = WorkingMemory()
    memory.append(
        "tool",
        "cmd.run",
        {"argv": ["python", "-m", "pytest"]},
        {"ok": True, "result": {"exit_code": 1}},
        "reproduction failed",
    )
    for index in range(10):
        memory.append(
            "tool", "repo.read", {"path": f"{index}.py"}, {"ok": True, "result": {}}, "inspect"
        )
    command = json.loads(memory.render())["summary"]["commands"][0]
    assert json.loads(command["argv"]) == ["python", "-m", "pytest"]
    assert command["exit_code"] == 1

"""Bounded turn records and deterministic older evidence, without model summarization."""

from __future__ import annotations

import json
from collections import deque
from typing import Any


def _json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _clip(text: str, cap: int) -> str:
    return text.encode()[:cap].decode(errors="ignore")


class WorkingMemory:
    def __init__(self, byte_cap: int = 24 * 1024) -> None:
        if not 1024 <= byte_cap <= 64 * 1024:
            raise ValueError("memory byte cap must be 1..64 KiB")
        self.byte_cap = byte_cap
        self.summary_cap = byte_cap // 4
        self.record_cap = (byte_cap - self.summary_cap - 64) // 6
        self.recent: deque[tuple[dict[str, str], dict[str, Any]]] = deque()
        self.facts: dict[str, list[Any]] = {
            key: [] for key in ("files_read", "files_changed", "commands", "hypotheses")
        }
        self.omitted = 0

    def append(
        self,
        action: str,
        tool: str,
        args: dict[str, Any],
        response: dict[str, Any],
        summary: str,
        *,
        notes: str = "",
    ) -> None:
        record = {
            "action": _clip(action, 16),
            "tool": _clip(tool, 48),
            "args_summary": _clip(_json(args), 512),
            "result_excerpt": _clip(_json(response), 8192),
            "summary": _clip(summary, 512),
            "notes": _clip(notes, 2048),
        }
        while len(_json(record).encode()) > self.record_cap:
            key = max(record, key=lambda key: len(record[key].encode()))
            record[key] = _clip(record[key], len(record[key].encode()) // 2)
        facts: dict[str, Any] = {"hypotheses": _clip(summary, 512)}
        result = response.get("result")
        if response.get("ok") and isinstance(result, dict):
            path = _clip(str(args.get("path", "")), 4096)
            if tool == "repo.read":
                facts["files_read"] = {
                    "path": path,
                    "start_line": result.get("start_line", args.get("start_line", 1)),
                    "end_line": result.get("end_line"),
                }
            elif tool == "repo.patch":
                facts["files_changed"] = path
            elif tool == "cmd.run":
                facts["commands"] = {
                    "argv": _clip(_json(args.get("argv", [])), 512),
                    "argv_truncated": len(_json(args.get("argv", [])).encode()) > 512,
                    "exit_code": result.get("exit_code"),
                }
        self.recent.append((record, facts))
        if len(self.recent) > 6:
            _, old = self.recent.popleft()
            for key, fact in old.items():
                if fact and fact not in self.facts[key]:
                    if len(self.facts[key]) < 64:
                        self.facts[key].append(fact)
                    else:
                        self.omitted += 1

    def render(self) -> str:
        summary: dict[str, Any] = {key: list(value) for key, value in self.facts.items()}
        summary["omitted_facts"] = self.omitted
        while len(_json(summary).encode()) > self.summary_cap:
            for key in ("hypotheses", "commands", "files_read", "files_changed"):
                if summary[key]:
                    summary[key].pop()
                    summary["omitted_facts"] += 1
                    break
        value = _json({"summary": summary, "recent": [record for record, _ in self.recent]})
        if len(value.encode()) > self.byte_cap:
            raise ValueError("memory serialization exceeds byte cap")
        return value

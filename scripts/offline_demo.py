"""Real CLI/daemon/worker/Git demonstration; disposable repositories, no live provider.

Run after cargo build --workspace and uv sync --locked. Python must be the intelligence
venv interpreter. SQLite is inspected in read-only mode; only Rust writes state.
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=Path("target/debug"))
    parser.add_argument("--tasks", type=int, default=3)
    parser.add_argument("--fault", action="store_true")
    parser.add_argument("--cancel", action="store_true")
    args = parser.parse_args()
    assert 1 <= args.tasks <= 200
    suffix = ".exe" if os.name == "nt" else ""
    cli = (args.bin_dir / f"harness{suffix}").resolve()
    daemon_bin = (args.bin_dir / f"harnessd{suffix}").resolve()
    with tempfile.TemporaryDirectory(prefix="nova-demo-") as temporary:
        root = Path(temporary)
        repo, data = root / "repo", root / "data"
        repo.mkdir()

        def git(*argv: str) -> str:
            return subprocess.check_output(["git", "-C", str(repo), *argv], text=True).strip()

        git("init", "-q", "-b", "main")
        git("config", "user.name", "Harness Fixture")
        git("config", "user.email", "fixture@example.test")
        (repo / "README.md").write_text("Disposable offline fixture.\n", encoding="utf-8")
        git("add", "README.md")
        git("commit", "-qm", "fixture")
        environment = {
            k: v
            for k, v in os.environ.items()
            if not any(s in k.upper() for s in ("API_KEY", "TOKEN", "SECRET"))
        }

        def start() -> subprocess.Popen[bytes]:
            return subprocess.Popen(
                [str(daemon_bin), "--data-dir", str(data)],
                env=environment,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )

        daemon = start()

        def call(*argv: str) -> dict | list:
            result = subprocess.check_output(
                [str(cli), "--daemon-dir", str(data), "--json", *argv], text=True
            )
            return json.loads(result)

        try:
            deadline = time.monotonic() + 10
            while not (data / "daemon.json").exists():
                assert daemon.poll() is None and time.monotonic() < deadline
                time.sleep(0.05)
            project = call("init", str(repo), "--name", "offline fixture", "--trust-local")
            run = call("run", project["id"])
            tasks, commands = [], []
            for n in range(args.tasks):
                ident, path = f"task-{n:03}", f"output/{n:03}.txt"
                code = (
                    f"from pathlib import Path; assert Path({path!r}).read_text().strip() "
                    f"== 'Mock task {ident}'"
                )
                if args.fault or args.cancel:
                    code = "import time; time.sleep(3); " + code
                command = [sys.executable, "-c", code]
                commands.append(command)
                tasks.append(
                    {
                        "id": ident,
                        "title": f"Create fixture {n}",
                        "description": "Create one synthetic offline fixture file.",
                        "complexity": "low",
                        "risk": "low",
                        "agent_priority": 100 - n % 100,
                        "manual_priority": None,
                        "dependencies": [f"task-{n - 2:03}"] if n >= 2 else [],
                        "candidate_paths": [path],
                        "acceptance": [{"kind": "test", "command": command}],
                        "limits": {
                            "max_tool_calls": 4,
                            "max_output_tokens": 1000,
                            "timeout_seconds": 30,
                            "cost_cap_usd": 0,
                        },
                    }
                )
            plan = {
                "schema_version": "1.0",
                "project_id": project["id"],
                "idea": "Demonstrate the real offline harness.",
                "assumptions": [],
                "requirements": ["Every fixture has evidence and an integration commit."],
                "architecture": {
                    "recommended": "Use isolated bounded task branches and real verification.",
                    "alternatives": [],
                    "risks": [],
                },
                "tasks": tasks,
            }
            plan_file, commands_file = root / "plan.json", root / "commands.json"
            plan_file.write_text(json.dumps(plan), encoding="utf-8")
            commands_file.write_text(json.dumps(commands), encoding="utf-8")
            call("plan", run["id"], str(plan_file))
            call("approve", run["id"], "1")
            call(
                "execute",
                run["id"],
                "--python",
                sys.executable,
                "--test-commands",
                str(commands_file),
            )
            deadline = time.monotonic() + max(30, args.tasks * 2)
            while True:
                status = call("status", run["id"])
                rows = call("task", run["id"])
                if args.cancel and any(t["state"] == "verifying" for t in rows):
                    call("cancel", run["id"])
                    time.sleep(3.5)
                    assert call("status", run["id"])["state"] == "cancelled"
                    with sqlite3.connect(
                        (data / "harness.db").as_uri() + "?mode=ro", uri=True
                    ) as db:
                        assert db.execute("SELECT COUNT(*) FROM git_artifacts").fetchone()[0] == 0
                    print(
                        "PASS: cancellation during verification retained changes without a commit"
                    )
                    return
                if args.fault and any(t["state"] == "verifying" for t in rows):
                    daemon.kill()
                    daemon.wait(timeout=10)
                    daemon = start()
                    # The discovery file already exists; poll the authenticated API.
                    for _ in range(100):
                        try:
                            rows = call("task", run["id"])
                            if any(t["state"] == "blocked" for t in rows):
                                break
                        except subprocess.CalledProcessError:
                            pass
                        time.sleep(0.05)
                    assert any(t["state"] == "blocked" for t in rows)
                    assert all(t["attempt"] <= 1 for t in rows)
                    print(
                        "PASS: killed daemon during verification; restart blocked uncertain "
                        "attempts, no replay"
                    )
                    return
                assert not any(t["state"] in ("blocked", "failed") for t in rows), rows
                if status["state"] == "completed":
                    break
                assert time.monotonic() < deadline, "execution deadline exceeded"
                time.sleep(0.05)
            with sqlite3.connect((data / "harness.db").as_uri() + "?mode=ro", uri=True) as db:
                records = db.execute(
                    "SELECT task_commit_sha, integration_commit_sha FROM git_artifacts"
                ).fetchall()
                assert len(records) == args.tasks and all(
                    len(a) == 40 and len(b) == 40 for a, b in records
                )
                assert len({a for a, _ in records}) == args.tasks
                assert (
                    db.execute("SELECT COUNT(*) FROM test_results WHERE exit_code=0").fetchone()[0]
                    >= args.tasks
                )
                assert (
                    db.execute("SELECT COUNT(*) FROM leases WHERE released_at IS NULL").fetchone()[
                        0
                    ]
                    == 0
                )
            events = call("logs", run["id"], "--limit", "1000")
            assert all(
                a["sequence"] < b["sequence"] for a, b in zip(events, events[1:], strict=False)
            )
            assert git("status", "--porcelain") == ""
            assert git("rev-list", "--count", "HEAD") == "1"
            print(
                f"PASS: {args.tasks} real worker tasks, unique commits, verifier evidence, "
                "replayable events, clean user checkout"
            )
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=10)
            if args.fault:
                # A verifier already running at SIGKILL may finish independently.
                # Wait for this fixture-only bounded command before temp cleanup.
                time.sleep(3.2)


if __name__ == "__main__":
    main()

"""Optional online preparation of hash-pinned packages; the installation smoke stays offline."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="nova-cache-warm-") as temporary:
        work = Path(temporary)
        requirements = root / "requirements.lock.txt"
        if not requirements.exists():
            requirements = work / "requirements.txt"
            with requirements.open("w") as output:
                subprocess.run(
                    [
                        "uv",
                        "export",
                        "--locked",
                        "--offline",
                        "--project",
                        "intelligence",
                        "--no-dev",
                        "--no-emit-project",
                        "--format",
                        "requirements-txt",
                    ],
                    cwd=root,
                    stdout=output,
                    check=True,
                )
        subprocess.run(
            ["uv", "venv", "--offline", "--python", sys.executable, str(work / "venv")], check=True
        )
        python = work / "venv" / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        subprocess.run(
            [
                "uv",
                "pip",
                "install",
                "--require-hashes",
                "--python",
                str(python),
                "-r",
                str(requirements),
            ],
            check=True,
        )
        print("PASS: hash-verified dependency cache prepared; no provider calls")


if __name__ == "__main__":
    main()

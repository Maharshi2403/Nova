"""Verify/extract a local bundle and run a fresh offline wheel installation + CLI demo."""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path, PurePosixPath


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", type=Path, nargs="?")
    parser.add_argument("--archive-dir", type=Path)
    args = parser.parse_args()
    archive = args.archive
    if archive is None:
        candidates = list(args.archive_dir.glob("nova-*.zip")) if args.archive_dir else []
        assert len(candidates) == 1, "specify one native archive"
        archive = candidates[0]
    expected = archive.with_suffix(".zip.sha256").read_text().split()[0]
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == expected
    with tempfile.TemporaryDirectory(prefix="nova-installed-") as temporary:
        root = Path(temporary)
        with zipfile.ZipFile(archive) as bundle:
            assert len(bundle.infolist()) <= 1000
            assert len(set(bundle.namelist())) == len(bundle.namelist())
            assert sum(e.file_size for e in bundle.infolist()) <= 256 * 1024 * 1024
            for entry in bundle.infolist():
                assert entry.file_size <= 64 * 1024 * 1024
                path = PurePosixPath(entry.filename)
                assert not path.is_absolute() and ".." not in path.parts
                assert "\\" not in entry.filename and ":" not in entry.filename
                assert not any(p in {".git", ".env", "api-token", "harness.db"} for p in path.parts)
                assert (entry.external_attr >> 16) & 0o170000 != 0o120000
            sums = bundle.read("SHA256SUMS").decode().splitlines()
            expected_files = {"SHA256SUMS"}
            for line in sums:
                digest, name = line.split("  ", 1)
                assert hashlib.sha256(bundle.read(name)).hexdigest() == digest
                expected_files.add(name)
            assert set(bundle.namelist()) == expected_files
            bundle.extractall(root)
        suffix = ".exe" if os.name == "nt" else ""
        for name in ("harness", "harnessd"):
            binary = root / "bin" / (name + suffix)
            if os.name != "nt":
                binary.chmod(0o755)
            subprocess.run([str(binary), "--version"], check=True)
        venv = root / "runtime"
        subprocess.run(
            ["uv", "venv", "--offline", "--python", sys.executable, str(venv)], check=True
        )
        python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        subprocess.run(
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--require-hashes",
                "--python",
                str(python),
                "-r",
                str(root / "requirements.lock.txt"),
            ],
            check=True,
        )
        wheels = list((root / "wheels").glob("*.whl"))
        assert len(wheels) == 1
        subprocess.run(
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--no-deps",
                "--python",
                str(python),
                str(wheels[0]),
            ],
            check=True,
        )
        subprocess.run([str(python), "-I", "-m", "harness_ai", "--version"], check=True)
        subprocess.run(
            [
                str(python),
                str(root / "scripts/offline_demo.py"),
                "--bin-dir",
                str(root / "bin"),
                "--tasks",
                "3",
            ],
            check=True,
        )
        print(
            "PASS: archive hashes/paths, fresh wheel installation, native binaries "
            "and offline CLI workflow"
        )


if __name__ == "__main__":
    main()

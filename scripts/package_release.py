"""Deterministic local bundle from locked native builds and a pinned Python wheel.

No publication, tag, global installation, secrets or runtime state are included.
Run cargo build --release --locked first. uv's dependency/build caches must be warm
for --offline packaging. Archive determinism does not assert cross-host binary identity.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import tempfile
import tomllib
import zipfile
from pathlib import Path


def output(argv: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> str:
    return subprocess.check_output(argv, cwd=cwd, env=env, text=True).strip()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=Path("target/release"))
    parser.add_argument("--out-dir", type=Path, default=Path("dist"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binaries = args.bin_dir.resolve()
    destination = args.out_dir.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    revision = output(["git", "rev-parse", "HEAD"], cwd=root)
    epoch = output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=root)
    environment = dict(os.environ, SOURCE_DATE_EPOCH=epoch)
    suffix = ".exe" if os.name == "nt" else ""
    files: dict[str, bytes] = {}
    for name in ("harness", "harnessd"):
        path = binaries / (name + suffix)
        assert path.is_file(), f"build the native release binary first: {path}"
        files["bin/" + path.name] = path.read_bytes()
    with tempfile.TemporaryDirectory(prefix="nova-package-") as temporary:
        wheel_dir = Path(temporary)
        subprocess.run(
            [
                "uv",
                "build",
                "--offline",
                "--wheel",
                "intelligence",
                "--build-constraints",
                "scripts/build-constraints.txt",
                "--out-dir",
                str(wheel_dir),
            ],
            cwd=root,
            env=environment,
            check=True,
        )
        wheels = list(wheel_dir.glob("*.whl"))
        assert len(wheels) == 1
        files["wheels/" + wheels[0].name] = wheels[0].read_bytes()
    requirements = output(
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
    )
    files["requirements.lock.txt"] = (requirements + "\n").encode()
    selected = [
        root / "README.md",
        root / "PROGRESS.md",
        root / "scripts/offline_demo.py",
        root / "scripts/smoke_bundle.py",
    ]
    selected += sorted((root / "docs").glob("*.md"))
    selected += sorted((root / "schemas").glob("*.json"))
    selected += [root / "scripts/prepare_release_cache.py"]
    for path in [
        "Cargo.lock",
        "Cargo.toml",
        "rust-toolchain.toml",
        "intelligence/pyproject.toml",
        "scripts/build-constraints.txt",
    ]:
        files["build-inputs/" + path] = (root / path).read_bytes()
    selected += [
        root / "config/live.example.json",
        root / "config/defaults.example.yaml",
        root / "intelligence/uv.lock",
    ]
    for path in selected:
        files[path.relative_to(root).as_posix()] = path.read_bytes()
    files["BUILD.json"] = (
        json.dumps(
            {
                "version": version,
                "source_revision": revision,
                "source_tracked_dirty": bool(
                    output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=root)
                ),
                "platform": platform.system(),
                "architecture": platform.machine(),
                "rustc": output(["rustc", "--version"], cwd=root),
                "source_date_epoch": int(epoch),
                "release_status": "local candidate; hosted platform and manual live gates required",
                "binary_reproducibility": (
                    "archive deterministic for identical inputs; "
                    "cross-host binary identity unverified"
                ),
            },
            sort_keys=True,
            indent=2,
        )
        + "\n"
    ).encode()
    files["SHA256SUMS"] = "".join(
        f"{hashlib.sha256(value).hexdigest()}  {name}\n" for name, value in sorted(files.items())
    ).encode()
    archive = (
        destination / f"nova-{version}-{platform.system().lower()}-{platform.machine().lower()}.zip"
    )
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for name, data in sorted(files.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (0o100755 if name.startswith("bin/") else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(info, data, compresslevel=9)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="utf-8")
    print(f"Created {archive.name}; SHA256 {checksum}")


if __name__ == "__main__":
    main()

"""Build the local macOS Nova.app candidate with a locked intelligence venv.

Requires the native release nova binary, macOS SDK, installed Python >=3.12 and a
warm uv cache. No publication, signing identity, keys or runtime state are included.
"""

from __future__ import annotations

import hashlib
import json
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path


def run(args: list[str], root: Path) -> None:
    subprocess.run(args, cwd=root, check=True)


def main() -> None:
    if sys.platform != "darwin":
        raise SystemExit("The current desktop app is macOS only.")
    root = Path(__file__).resolve().parents[1]
    binary = root / "target/release/nova"
    if not binary.is_file():
        raise SystemExit("Build cargo build --release --locked --workspace first.")
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    dist = root / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nova-desktop-build-", dir=dist) as temporary:
        staging = Path(temporary)
        app = staging / "Nova.app"
        contents = app / "Contents"
        resources = contents / "Resources"
        (contents / "MacOS").mkdir(parents=True)
        resources.mkdir()
        arch = "arm64" if platform.machine() == "arm64" else "x86_64"
        run(
            [
                "swiftc",
                "-swift-version",
                "5",
                "-parse-as-library",
                "-O",
                "-target",
                f"{arch}-apple-macos14.0",
                "apps/desktop/Nova.swift",
                "apps/desktop/Panel.swift",
                "-o",
                str(contents / "MacOS/Nova"),
            ],
            root,
        )
        iconset = staging / "Nova.iconset"
        run(["swift", "apps/desktop/Icon.swift", str(iconset)], root)
        run(["iconutil", "-c", "icns", str(iconset), "-o", str(resources / "Nova.icns")], root)
        shutil.copy2(binary, resources / "nova")
        shutil.copy2(root / "apps/desktop/NovaLogo.svg", resources / "NovaLogo.svg")
        shutil.copy2(
            root / "scripts/verify_documentation.py", resources / "verify_documentation.py"
        )
        runtime = resources / "runtime"
        run(
            ["uv", "venv", "--offline", "--relocatable", "--python", sys.executable, str(runtime)],
            root,
        )
        requirements = subprocess.check_output(
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
        lock = resources / "requirements.lock.txt"
        lock.write_bytes(requirements)
        python = runtime / "bin/python"
        run(
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--require-hashes",
                "--python",
                str(python),
                "-r",
                str(lock),
            ],
            root,
        )
        wheels = staging / "wheels"
        run(
            [
                "uv",
                "build",
                "--offline",
                "--wheel",
                "intelligence",
                "--build-constraints",
                "scripts/build-constraints.txt",
                "--out-dir",
                str(wheels),
            ],
            root,
        )
        wheel = next(wheels.glob("*.whl"))
        run(
            ["uv", "pip", "install", "--offline", "--no-deps", "--python", str(python), str(wheel)],
            root,
        )
        run([str(python), "-I", "-m", "harness_ai", "--version"], root)
        (contents / "Info.plist").write_bytes(
            plistlib.dumps(
                {
                    "CFBundleIdentifier": "dev.nova.desktop",
                    "CFBundleName": "Nova",
                    "CFBundleDisplayName": "Nova",
                    "CFBundleExecutable": "Nova",
                    "CFBundlePackageType": "APPL",
                    "CFBundleIconFile": "Nova.icns",
                    "CFBundleShortVersionString": version,
                    "CFBundleVersion": version,
                    "LSMinimumSystemVersion": "14.0",
                    "NSHighResolutionCapable": True,
                }
            )
        )
        (resources / "BUILD.json").write_text(
            json.dumps(
                {
                    "source_revision": subprocess.check_output(
                        ["git", "rev-parse", "HEAD"], cwd=root, text=True
                    ).strip(),
                    "native_architecture": arch,
                    "minimum_macos": "14.0",
                    "status": "local unsigned candidate; requires installed base Python",
                },
                indent=2,
            )
            + "\n"
        )
        # Generated venv caches and bytecode are not application inputs.
        for cache in runtime.rglob("__pycache__"):
            shutil.rmtree(cache)
        # Copying dereferences the installed Python symlink. pyvenv.cfg still selects
        # its installed base stdlib; the app is relocatable on this configured host.
        target = dist / "Nova.app"
        backup = dist / "Nova.previous.app"
        if backup.exists():
            shutil.rmtree(backup)
        if target.exists():
            target.rename(backup)
        shutil.copytree(app, target, symlinks=False)
        final_python = target / "Contents/Resources/runtime/bin/python"
        run([str(final_python), "-I", "-m", "harness_ai", "--version"], root)
        archive = dist / f"Nova-desktop-{version}-darwin-{arch}.zip"
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
            for path in sorted(target.rglob("*")):
                if path.is_file():
                    package.write(path, path.relative_to(dist))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n")
        print(f"Built {target}\nCreated {archive.name}; SHA256 {digest}")


if __name__ == "__main__":
    main()

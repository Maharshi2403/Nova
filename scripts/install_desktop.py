"""Install the locally built Nova desktop and CLI, retaining previous versions.

Run after scripts/build_desktop.py. Does not modify provider/project state, launch
agents, approve tasks or make provider requests. Requires write access to your home.
"""

from __future__ import annotations

import os
import plistlib
import shutil
import uuid
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    source = root / "dist/Nova.app"
    if not (source / "Contents/MacOS/Nova").is_file():
        raise SystemExit("Build scripts/build_desktop.py first.")
    destination = Path.home() / "Applications/Nova.app"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        info = plistlib.loads((destination / "Contents/Info.plist").read_bytes())
        if info.get("CFBundleIdentifier") != "dev.nova.desktop":
            raise SystemExit("An unrelated Nova.app exists; it was left untouched.")
    staging = destination.parent / f".Nova-{uuid.uuid4()}.app"
    shutil.copytree(source, staging)
    if destination.exists():
        destination.rename(destination.with_name(f"Nova.previous-{uuid.uuid4()}.app"))
    staging.rename(destination)
    binary = Path.home() / ".local/bin/nova"
    binary.parent.mkdir(parents=True, exist_ok=True)
    if binary.exists():
        shutil.copy2(binary, binary.with_name(f"nova.previous-{uuid.uuid4()}"))
    temporary = binary.with_name(f".nova-{uuid.uuid4()}")
    shutil.copy2(destination / "Contents/Resources/nova", temporary)
    temporary.chmod(0o755)
    os.replace(temporary, binary)
    print(f"Installed {destination}\nInstalled {binary}\nStart with nova ui.")


if __name__ == "__main__":
    main()

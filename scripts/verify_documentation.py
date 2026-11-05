"""Read-only, stdlib-only acceptance check for Nova documentation tasks."""

from __future__ import annotations

import argparse
import glob
from pathlib import Path


def verify(patterns: list[str]) -> str:
    root = Path.cwd().resolve()
    files: set[Path] = set()
    for pattern in patterns:
        for match in glob.iglob(pattern, recursive=True):
            path = Path(match)
            candidates = path.rglob("*.md") if path.is_dir() else [path]
            for candidate in candidates:
                resolved = candidate.resolve()
                if candidate.is_symlink() or not resolved.is_relative_to(root):
                    continue
                if resolved.is_file() and resolved.suffix.lower() == ".md":
                    files.add(resolved)
                    if len(files) > 100:
                        raise ValueError(
                            "Documentation scope exceeds 100 Markdown files; narrow it."
                        )
    if not files:
        raise ValueError("No Markdown documentation found under the approved paths.")
    if sum(path.stat().st_size for path in files) > 4 * 1024 * 1024:
        raise ValueError("Documentation exceeds the 4 MiB verification limit.")
    text = "\n".join(path.read_text(encoding="utf-8") for path in sorted(files))
    lower = text.lower()
    if len(text.split()) < 300:
        raise ValueError("Documentation must contain at least 300 words.")
    if text.count("## ") < 3:
        raise ValueError("Documentation needs at least three Markdown sections.")
    if "```mermaid" not in lower:
        raise ValueError("Add a fenced Mermaid system-design diagram.")
    if not any(word in lower for word in ("architecture", "system design")):
        raise ValueError("Add an architecture or system-design explanation.")
    if not any(word in lower for word in ("improvement", "roadmap")):
        raise ValueError("Add future improvements or a roadmap.")
    return f"Documentation verified: {len(files)} Markdown file(s), diagram and roadmap present."


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths", nargs="+", help="Approved Markdown files, folders or glob patterns"
    )
    args = parser.parse_args()
    try:
        print(verify(args.paths))
    except (ValueError, OSError, UnicodeError) as error:
        parser.exit(1, f"Documentation check failed: {error}\n")


if __name__ == "__main__":
    main()

"""Exercise the shipped stdlib checker with actual filesystem artifacts."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).resolve().parents[1] / "scripts/verify_documentation.py"
VALID = (
    "# Nova\n## Architecture\n"
    + "system component execution evidence " * 100
    + (
        "\n## System design\n```mermaid\nflowchart LR\n A-->B\n```\n"
        "## Future improvements\nA roadmap for implementation.\n"
    )
)


class DocumentationVerification(unittest.TestCase):
    def check(self, directory: Path, *paths: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), *paths],
            cwd=directory,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_valid_docs_pass_without_third_party_modules(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            docs = root / "Documents/Nova"
            docs.mkdir(parents=True)
            (docs / "README.md").write_text(VALID, encoding="utf-8")
            result = self.check(root, "Documents/Nova")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("diagram and roadmap present", result.stdout)

    def test_missing_or_incomplete_docs_fail(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertNotEqual(self.check(root, "docs").returncode, 0)
            (root / "docs").mkdir()
            (root / "docs/README.md").write_text(
                VALID.replace("```mermaid", "```text"), encoding="utf-8"
            )
            result = self.check(root, "docs/**")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Mermaid", result.stderr)

    def test_external_files_are_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work = root / "work"
            work.mkdir()
            outside = root / "outside.md"
            outside.write_text(VALID, encoding="utf-8")
            self.assertNotEqual(self.check(work, str(outside)).returncode, 0)


if __name__ == "__main__":
    unittest.main()

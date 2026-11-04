import subprocess
import sys

import harness_ai
from harness_ai.__main__ import main


def test_package_importable():
    assert harness_ai.__version__ == "0.1.0"


def test_main_returns_zero_and_keeps_stdout_clean(capsys):
    assert main([]) == 0
    captured = capsys.readouterr()
    assert captured.out == ""


def test_module_entrypoint_version():
    result = subprocess.run(
        [sys.executable, "-m", "harness_ai", "--version"],
        capture_output=True,
        text=True,
        check=True,
    )
    assert "harness_ai 0.1.0" in result.stdout

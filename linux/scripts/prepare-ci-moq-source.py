"""Prepare the Linux lockfile with the shared desktop CI helper."""

from pathlib import Path
import subprocess
import sys


if __name__ == "__main__":
    linux = Path(__file__).resolve().parents[1]
    subprocess.run(
        [sys.executable, linux.parent / ".github/scripts/prepare-ci-moq-source.py", linux / "Cargo.lock"],
        check=True,
    )

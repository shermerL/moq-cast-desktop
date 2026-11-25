"""Select the executable and its own build script's provenance from Cargo JSON."""
import json
import os
from pathlib import Path
import sys


def main():
    binaries = []
    build_dirs = {}
    for line in sys.stdin:
        message = json.loads(line)
        if message["reason"] == "build-script-executed":
            build_dirs.setdefault(message["package_id"], set()).add(message["out_dir"])
        elif message["reason"] == "compiler-artifact":
            target = message["target"]
            if target["name"] == "moq-cast-desktop" and "bin" in target["kind"] and message["executable"]:
                binaries.append((message["package_id"], message["executable"]))

    if len(binaries) != 1:
        sys.exit("Expected Cargo to report one moq-cast-desktop executable.")
    package, executable = binaries[0]
    directories = build_dirs.get(package, set())
    if len(directories) != 1:
        sys.exit("Expected Cargo to report one build script output for moq-cast-desktop.")
    provenance = Path(directories.pop()) / "moqcast-build-info.txt"
    for path in (executable, provenance):
        sys.stdout.buffer.write(os.fsencode(path) + b"\0")


if __name__ == "__main__":
    main()

"""Make the locked MoQ workspace available to Cargo in a native desktop CI job."""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

REPOSITORY = "https://github.com/moq-dev/moq.git"


def main():
    lockfile = Path(sys.argv[1])
    lock = lockfile.read_text(encoding="utf-8")
    pattern = r'^source = "git\+(https://github\.com/moq-dev/moq(?:\.git)?)\?rev=([0-9a-f]{40})#\2"$'
    sources = set(re.findall(pattern, lock, re.MULTILINE))
    if len(sources) != 1:
        raise SystemExit(f"Expected one exact MoQ revision in {lockfile}")
    source, revision = sources.pop()

    # The Rust workspace lives in the parent tree. Its retired editor submodule
    # is unavailable, so fetch just the commit named by the lockfile.
    checkout = Path(os.environ["RUNNER_TEMP"]) / "moq-source"
    subprocess.run(["git", "init", "--quiet", str(checkout)], check=True)
    subprocess.run(["git", "fetch", "--no-tags", "--depth=1", REPOSITORY, revision],
                   cwd=checkout, check=True)
    subprocess.run(["git", "-c", "core.autocrlf=false", "checkout", "--quiet", "--no-recurse-submodules",
                    "--detach", revision], cwd=checkout, check=True)

    crates = checkout / "rs"
    for manifest in crates.glob("*/Cargo.toml"):
        crate = manifest.parent
        files = {
            path.relative_to(crate).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in crate.rglob("*")
            if path.is_file() and path.name != ".cargo-checksum.json"
        }
        (crate / ".cargo-checksum.json").write_text(json.dumps({"files": files, "package": None}))

    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    cargo_home.mkdir(parents=True, exist_ok=True)
    config_path = cargo_home / "config.toml"
    replacement = f'''
[source.pinned-moq]
git = "{source}"
rev = "{revision}"
replace-with = "moq-workspace"

[source.moq-workspace]
directory = {json.dumps(str(crates))}
'''
    existing = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
    if replacement not in existing:
        with config_path.open("a", encoding="utf-8") as config:
            config.write(replacement)


if __name__ == "__main__":
    main()

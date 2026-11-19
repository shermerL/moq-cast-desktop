"""Make the locked MoQ workspace available to Cargo in an AppImage CI job."""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

REPOSITORY = "https://github.com/moq-dev/moq.git"


def main():
    lock = (Path(__file__).resolve().parents[1] / "Cargo.lock").read_text()
    pattern = rf'^source = "git\+{re.escape(REPOSITORY)}\?rev=([0-9a-f]{{40}})#\1"$'
    revisions = set(re.findall(pattern, lock, re.MULTILINE))
    if len(revisions) != 1:
        raise SystemExit("Expected one exact MoQ revision in linux/Cargo.lock")
    revision = revisions.pop()

    # The Rust workspace lives in the parent tree. Its retired editor submodule
    # is unavailable, so fetch just the commit named by the lockfile.
    checkout = Path(os.environ["RUNNER_TEMP"]) / "moq-source"
    subprocess.run(["git", "init", "--quiet", str(checkout)], check=True)
    subprocess.run(["git", "fetch", "--no-tags", "--depth=1", REPOSITORY, revision],
                   cwd=checkout, check=True)
    subprocess.run(["git", "checkout", "--quiet", "--no-recurse-submodules",
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
    with (cargo_home / "config.toml").open("a") as config:
        config.write(f'''
[source.pinned-moq]
git = "{REPOSITORY}"
rev = "{revision}"
replace-with = "moq-workspace"

[source.moq-workspace]
directory = {json.dumps(str(crates))}
''')


if __name__ == "__main__":
    main()

"""Prepare an isolated moq-video overlay without changing Desktop dependency pins."""

import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tomllib


BASE = "81d39f7bf04c82aae324a9ee4251b7f8aa08fb53"
SOURCE = "https://github.com/moq-dev/moq"


def inline(value):
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (str, int)):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(inline(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{inline(key)} = {inline(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {type(value).__name__}")


def write_toml(path, data):
    text = "\n".join(f"{inline(key)} = {inline(value)}" for key, value in data.items()) + "\n"
    assert tomllib.loads(text) == data
    path.write_text(text, encoding="utf-8")


def standalone(manifest, workspace):
    result = copy.deepcopy(manifest)
    for key, value in list(result["package"].items()):
        if isinstance(value, dict) and value.get("workspace"):
            result["package"][key] = workspace["package"][key]
    groups = [result, *result.get("target", {}).values()]
    for group in groups:
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, value in list(group.get(section, {}).items()):
                if not isinstance(value, dict) or not value.get("workspace"):
                    continue
                base = copy.deepcopy(workspace["dependencies"][name])
                base = {"version": base} if isinstance(base, str) else base
                features = base.pop("features", []) + value.get("features", [])
                base.update({k: v for k, v in value.items() if k not in ("workspace", "features")})
                if features:
                    base["features"] = sorted(set(features))
                if "path" in base:
                    del base["path"]
                    base.update(git=SOURCE, rev=BASE)
                group[section][name] = base
    if result.get("lints", {}).get("workspace"):
        result["lints"] = copy.deepcopy(workspace["lints"])
    result["workspace"] = {}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moq", required=True, type=Path, help="Local windows/wgc-capture checkout")
    parser.add_argument("--output", required=True, type=Path, help="New directory outside either repository")
    args = parser.parse_args()
    moq = args.moq.resolve()
    output = args.output.resolve()
    desktop = Path(__file__).resolve().parents[2]
    if output.is_relative_to(moq) or output.is_relative_to(desktop):
        parser.error("output must be outside the MoQ and Desktop repositories")
    head = subprocess.check_output(["git", "-C", str(moq), "rev-parse", "HEAD"], text=True).strip()
    if subprocess.run(["git", "-C", str(moq), "merge-base", "--is-ancestor", BASE, head]).returncode:
        parser.error("overlay source must descend from the pinned baseline")
    # No modified workspace dependency or sibling source may sneak into the overlay.
    changed = subprocess.check_output(["git", "-C", str(moq), "diff", "--name-only", BASE], text=True).splitlines()
    if any(not path.startswith("rs/moq-video/") for path in changed):
        parser.error("only moq-video changes are allowed against the pinned baseline")
    source = moq / "rs/moq-video"
    manifest = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))
    workspace = tomllib.loads((moq / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    resolved = standalone(manifest, workspace)
    # mkdir is exclusive: existing snapshots and files are never overwritten.
    output.mkdir(parents=True, exist_ok=False)
    overlay = output / "moq-video"
    overlay.mkdir()
    shutil.copytree(source / "src", overlay / "src")
    shutil.copytree(source / "examples", overlay / "examples")
    shutil.copy2(source / "README.md", overlay / "README.md")
    write_toml(overlay / "Cargo.toml", resolved)
    write_toml(output / "config.toml", {"patch": {SOURCE: {"moq-video": {"path": str(overlay)}}}})
    (output / "provenance.json").write_text(json.dumps({
        "base": BASE, "head": head, "package": "moq-video",
        "version": manifest["package"]["version"], "changed_tracked_files": changed,
        "snapshot_sha256": {
            str(path.relative_to(overlay)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(overlay.rglob("*")) if path.is_file()
        },
        "note": "Local snapshot, including uncommitted moq-video sources; no Desktop build or device validation.",
    }, indent=2) + "\n", encoding="utf-8")
    print(output / "config.toml")


if __name__ == "__main__":
    main()

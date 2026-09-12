"""Prepare and package an isolated Windows WGC CI build."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tomllib
import zipfile


BASE = "81d39f7bf04c82aae324a9ee4251b7f8aa08fb53"
SOURCE = f"git+https://github.com/moq-dev/moq?rev={BASE}#{BASE}"
CORE = {"moq-net", "moq-tokio", "moq-mux", "hang", "moq-audio"}
TARGET = "x86_64-pc-windows-msvc"


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def lock_packages(path):
    lock = tomllib.loads(path.read_text(encoding="utf-8"))
    return {(p["name"], p["version"], p.get("source", "")) for p in lock["package"]}


def check_lock(original, experiment):
    before, after = lock_packages(original), lock_packages(experiment)
    removed = before - after
    expected = {p for p in before if p[0] == "moq-video" and p[2] == SOURCE}
    if not expected or removed != expected:
        raise ValueError(f"Unexpected lock removals or upgrades: {sorted(removed - expected)}")
    additions = after - before
    if any(name in CORE for name, _, _ in additions):
        raise ValueError("Core MoQ lock entries must not change")
    return [dict(name=n, version=v, source=s or "local-overlay") for n, v, s in sorted(additions)]


def check_graph(metadata, overlay):
    selected = {name: [p for p in metadata["packages"] if p["name"] == name] for name in CORE | {"moq-video"}}
    if any(len(packages) != 1 for packages in selected.values()):
        raise ValueError("The full Desktop graph must contain one instance of each MoQ package")
    packages = {name: entries[0] for name, entries in selected.items()}
    for name in CORE:
        if packages[name]["source"] != SOURCE:
            raise ValueError(f"Unexpected {name} source: {packages[name]['source']}")
    video = packages["moq-video"]
    if video["source"] is not None or Path(video["manifest_path"]).resolve() != (overlay / "moq-video/Cargo.toml").resolve():
        raise ValueError("moq-video did not resolve to the exact WGC overlay")
    ids = {p["id"]: name for name, p in packages.items()}
    edges = {
        ids[node["id"]]: sorted(ids[d["pkg"]] for d in node["deps"] if d["pkg"] in ids)
        for node in metadata["resolve"]["nodes"] if node["id"] in ids
    }
    return {"packages": [{"name": n, "version": p["version"], "source": p["source"] or "local-overlay"}
                         for n, p in sorted(packages.items())], "edges": edges}


def prepare(args):
    desktop = Path(__file__).resolve().parents[2]
    moq, output = args.moq.resolve(), args.output.resolve()
    if not re.fullmatch(r"[0-9a-f]{40}", args.moq_sha):
        raise ValueError("moq-sha must be a full lowercase commit SHA")
    if run("git", "rev-parse", "HEAD", cwd=moq) != args.moq_sha:
        raise ValueError("The fork checkout does not match the requested SHA")
    if output.is_relative_to(desktop) or output.is_relative_to(moq):
        raise ValueError("The experiment must be outside both source checkouts")
    source_sha = run("git", "rev-parse", "HEAD", cwd=desktop)
    if run("git", "status", "--porcelain", "--untracked-files=no", cwd=desktop):
        raise ValueError("Desktop tracked files must be clean")
    if run("git", "status", "--porcelain", "--untracked-files=no", cwd=moq):
        raise ValueError("The MoQ fork tracked files must be clean")
    original_lock = desktop / "windows/Cargo.lock"
    original_hash = digest(original_lock)
    output.mkdir(parents=True, exist_ok=False)
    build = output / "desktop"
    shutil.copytree(desktop, build, ignore=shutil.ignore_patterns(".git", "target", "__pycache__"))
    overlay = output / "overlay"
    subprocess.run([sys.executable, str(desktop / "windows/scripts/prepare-wgc-overlay.py"),
                    "--moq", str(moq), "--output", str(overlay)], check=True)
    manifest = build / "windows/Cargo.toml"
    config = overlay / "config.toml"
    metadata = json.loads(run("cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET,
                              "--manifest-path", str(manifest), "--config", str(config), "--features", "wgc"))
    additions = check_lock(original_lock, manifest.with_name("Cargo.lock"))
    graph = check_graph(metadata, overlay)
    # From here on the experiment is frozen. Every build command must use --locked.
    json.loads(run("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET,
                   "--manifest-path", str(manifest), "--config", str(config), "--features", "wgc"))
    if digest(original_lock) != original_hash:
        raise ValueError("The committed product lockfile was modified")
    evidence = output / "evidence"
    evidence.mkdir()
    shutil.copy2(manifest.with_name("Cargo.lock"), evidence / "Cargo.lock")
    write_json(evidence / "dependency-graph.json", graph)
    shutil.copy2(overlay / "provenance.json", evidence / "overlay-provenance.json")
    version = tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]
    write_json(evidence / "build-provenance.json", {
        "schema": 1, "experimental": True, "version": version,
        "desktop_repository": "shermerL/moq-cast-desktop", "desktop_sha": source_sha,
        "moq_repository": "shermerL/moq", "moq_sha": args.moq_sha, "moq_base": BASE,
        "target": TARGET, "features": ["wgc"], "default_backend": "legacy",
        "rustc": run("rustc", "--version"), "cargo": run("cargo", "--version"),
        "product_lock_sha256": original_hash, "experiment_lock_sha256": digest(evidence / "Cargo.lock"),
        "added_lock_entries": additions,
        "lock_change_reason": "Replace only moq-video with the WGC snapshot; retain existing package versions. Added entries are required by the overlay feature graph.",
        "run_url": f"https://github.com/shermerL/moq-cast-desktop/actions/runs/{os.environ['GITHUB_RUN_ID']}",
        "device_validation": "Not performed; requires user testing.",
    })
    with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as env:
        env.write(f"WGC_MANIFEST={manifest}\nWGC_CONFIG={config}\n")


def package(args):
    output, exe = args.output.resolve(), args.exe.resolve()
    evidence = output / "evidence"
    if digest(output / "desktop/windows/Cargo.lock") != digest(evidence / "Cargo.lock"):
        raise ValueError("The frozen experimental lock changed during verification")
    data = exe.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", data, pe + 24 + 68)[0] != 2:
        raise ValueError("The release executable must use the Windows GUI subsystem")
    provenance_path = evidence / "build-provenance.json"
    provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    provenance["verified"] = ["desktop-check", "desktop-tests", "desktop-clippy", "release-build", "pe-gui-subsystem"]
    provenance["executable_sha256"] = digest(exe)
    write_json(provenance_path, provenance)
    bundle = output / "bundle"
    bundle.mkdir()
    shutil.copy2(exe, bundle / "moqcast-windows-wgc.exe")
    for path in evidence.iterdir():
        shutil.copy2(path, bundle / path.name)
    (bundle / "README.txt").write_text(
        "MoQCast Windows WGC experimental build\n\n"
        "Windows 10 2004 or later, x86-64. Not a stable release. No device validation has been performed.\n"
        "Extract the archive, run moqcast-windows-wgc.exe, then open Settings > Screen capture.\n"
        "Select Windows Graphics Capture (experimental). The default remains Legacy (DXGI).\n"
        "Choose the existing screen-sharing action; the Windows capture border may be visible.\n"
        "Test cursor visibility, moving/static content, repeated start/stop, exit, and remote playback.\n"
        "Also test source loss/size changes and optional system audio synchronization.\n"
        "Stop sharing before changing backends. Select Legacy to compare behavior.\n\n"
        "WGC is display-only, CPU I420/SDR. No window capture, automatic fallback, HDR or GPU NV12.\n"
        "A display size change ends the publication and requires starting sharing again.\n\n"
        "Reproduce: checkout the exact Desktop and fork MoQ SHAs in build-provenance.json.\n"
        "Generate a fresh overlay using windows/scripts/prepare-wgc-overlay.py.\n"
        "Use a separate Desktop source copy; install the bundled Cargo.lock into its windows directory.\n"
        "Run cargo build --locked --release --features wgc --config <generated-config> in that directory.\n",
        encoding="utf-8",
    )
    (bundle / "SHA256SUMS").write_text("".join(f"{digest(p)}  {p.name}\n" for p in sorted(bundle.iterdir()) if p.is_file()), encoding="utf-8")
    packages = output / "packages"
    packages.mkdir()
    archive = packages / f"moqcast-windows-wgc-{provenance['desktop_sha'][:12]}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
        for path in sorted(bundle.iterdir()):
            zipped.write(path, path.name)
    (packages / "SHA256SUMS").write_text(f"{digest(archive)}  {archive.name}\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--moq", type=Path, required=True)
    prep.add_argument("--moq-sha", required=True)
    prep.add_argument("--output", type=Path, required=True)
    pack = sub.add_parser("package")
    pack.add_argument("--output", type=Path, required=True)
    pack.add_argument("--exe", type=Path, required=True)
    args = parser.parse_args()
    {"prepare": prepare, "package": package}[args.command](args)


if __name__ == "__main__":
    main()


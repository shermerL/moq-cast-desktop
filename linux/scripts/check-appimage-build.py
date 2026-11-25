#!/usr/bin/env python3
"""Check that packaging installs the binary Cargo just built, using a tiny real crate."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class BuildOutputTests(unittest.TestCase):
    def check_output(self, mode, broken=False):
        root = Path(tempfile.mkdtemp(prefix="moqcast-package-test-"))
        self.addCleanup(lambda: print(f"Fixture retained: {root}"))
        for directory in ("scripts", "src", "target/release", "bin", ".cargo"):
            (root / directory).mkdir(parents=True, exist_ok=True)
        for name in ("build-appimage.sh", "build-info.sh", "appimage-artifacts.py"):
            shutil.copy2(SCRIPTS / name, root / "scripts" / name)
        (root / "Cargo.toml").write_text('[package]\nname="moq-cast-desktop"\nversion="0.0.0"\nedition="2021"\n')
        (root / "src/main.rs").write_text('invalid Rust' if broken else 'fn main() { println!("fresh-binary"); }')
        (root / "build.rs").write_text('''fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MOQCAST_PROVENANCE_OUTPUT");
    let record = "app_version=0.0.0\\nbuild_identity=linux-x86_64\\ntarget=x86_64-unknown-linux-gnu\\nsource_identity=fresh-build\\n";
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("moqcast-build-info.txt"), record).unwrap();
    if let Some(path) = std::env::var_os("MOQCAST_PROVENANCE_OUTPUT") {
        std::fs::write(path, record).unwrap();
    }
}''')
        stale = root / "target/release/moq-cast-desktop"
        stale.write_text("#!/bin/sh\necho stale-binary\n")
        stale.chmod(0o755)
        (stale.parent / "moqcast-build-info.txt").write_text("source_identity=stale-build\n")
        assets = ["packaging/appimage/AppRun", "packaging/appimage/dev.moq.moqcast.desktop.desktop",
                  "assets/fonts/LICENSE-NOTO", "vendor/moq-video/LICENSE-APACHE",
                  "vendor/moq-video/LICENSE-MIT", "vendor/libspa/LICENSE"]
        assets += [f"assets/icons/hicolor/{n}x{n}/apps/moqcast.png" for n in (16, 24, 32, 48, 64, 128, 256, 512)]
        for asset in assets:
            p = root / asset
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("fixture")
        # Only platform/packaging tools are replaced; Cargo compiles a real executable.
        tools = {
            "uname": '#!/bin/sh\necho Linux\n',
            "pkg-config": '#!/bin/sh\nif [ "$1" = --modversion ]; then echo 1.0; fi\n',
            "ldd": '#!/bin/sh\necho "ldd (fixture) 2.35"\n',
            "linuxdeploy": '#!/bin/sh\nexit 99\n',
            "sha256sum": '#!/bin/sh\nexit 98\n',
            "install": '''#!/usr/bin/env python3
from pathlib import Path
import shutil, sys
p = Path(sys.argv[-1]); p.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(sys.argv[-2], p)
''',
            "sed": '''#!/usr/bin/env python3
import os, sys
if sys.argv[-1] == '/etc/os-release':
    print('24.04' if 'VERSION_ID' in sys.argv[1] else 'ubuntu')
else:
    os.execv('/usr/bin/sed', ['sed'] + sys.argv[1:])
''',
        }
        for name, body in tools.items():
            p = root / "bin" / name
            p.write_text(body)
            p.chmod(0o755)
        env = dict(os.environ, PATH=str(root / "bin") + os.pathsep + os.environ["PATH"],
                   CARGO_NET_OFFLINE="true", MOQCAST_PACKAGE_DIR=str(root / "out"),
                   MOQCAST_PACKAGE_VARIANT="linux-x86_64", LINUXDEPLOY="linuxdeploy")
        for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "CARGO_BUILD_TARGET_DIR", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "MOQCAST_PROVENANCE_OUTPUT"):
            env.pop(name, None)
        if mode == "env":
            env["CARGO_TARGET_DIR"] = str(root / "custom target")
        if mode == "config":
            (root / ".cargo/config.toml").write_text('[build]\ntarget-dir="custom target"\n')
        if mode == "native-target":
            triple = subprocess.check_output(["rustc", "--print", "host-tuple"], text=True).strip()
            (root / ".cargo/config.toml").write_text(f'[build]\ntarget="{triple}"\n')
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, env=env, check=True, capture_output=True)
        result = subprocess.run(["bash", str(root / "scripts/build-appimage.sh")], cwd=root, env=env, capture_output=True, text=True)
        appdir = root / "out/MoQCast-0.0.0-linux-x86_64.AppDir"
        if broken:
            self.assertNotEqual(result.returncode, 0)
            self.assertNotEqual(result.returncode, 99)
            self.assertFalse(appdir.exists(), result.stderr)
            return
        self.assertEqual(result.returncode, 99, result.stderr)
        packaged = appdir / "usr/bin/moq-cast-desktop"
        self.assertEqual(subprocess.check_output([str(packaged)], text=True).strip(), "fresh-binary")
        self.assertIn("source_identity=fresh-build", (appdir / "usr/share/doc/moqcast/build-info.txt").read_text())

    def test_default_directory(self):
        self.check_output("default")

    def test_environment_directory(self):
        self.check_output("env")

    def test_cargo_config_directory(self):
        self.check_output("config")

    def test_explicit_native_target(self):
        self.check_output("native-target")

    def test_failed_build_does_not_package_stale_binary(self):
        self.check_output("env", broken=True)


if __name__ == "__main__":
    unittest.main()

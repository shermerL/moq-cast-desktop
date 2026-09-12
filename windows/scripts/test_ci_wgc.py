"""Checks for the experimental CI dependency boundary; no desktop build."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ci = load("ci-wgc")
overlay = load("prepare-wgc-overlay")


class DependencyBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.before = {"package": [
            {"name": "moq-video", "version": "0.0.18", "source": ci.SOURCE},
            {"name": "hang", "version": "0.20.6", "source": ci.SOURCE},
        ]}
        self.after = json.loads(json.dumps(self.before))
        self.after["package"][0].pop("source")

    def check_lock(self):
        before, after = self.root / "before.lock", self.root / "after.lock"
        overlay.write_toml(before, self.before)
        overlay.write_toml(after, self.after)
        return ci.check_lock(before, after)

    def test_only_video_changes_source(self):
        self.assertEqual(self.check_lock(), [{"name": "moq-video", "version": "0.0.18", "source": "local-overlay"}])

    def test_core_upgrade_is_rejected(self):
        self.after["package"][1]["version"] = "0.21.0"
        with self.assertRaises(ValueError):
            self.check_lock()

    def test_additional_core_identity_is_rejected(self):
        self.after["package"].append({"name": "hang", "version": "0.20.6"})
        with self.assertRaises(ValueError):
            self.check_lock()

    def graph(self):
        packages = [{"id": n, "name": n, "version": "0", "source": ci.SOURCE} for n in ci.CORE]
        packages.append({"id": "moq-video", "name": "moq-video", "version": "0.0.18", "source": None,
                         "manifest_path": str(self.root / "moq-video/Cargo.toml")})
        return {"packages": packages, "resolve": {"nodes": [
            {"id": p["id"], "deps": [{"pkg": "hang"}] if p["name"] == "moq-video" else []} for p in packages]}}

    def test_resolved_graph_keeps_shared_type_identity(self):
        graph = ci.check_graph(self.graph(), self.root)
        self.assertEqual(graph["edges"]["moq-video"], ["hang"])
        self.assertNotIn(str(self.root), json.dumps(graph))

    def test_duplicate_graph_identity_is_rejected(self):
        graph = self.graph()
        graph["packages"].append({"id": "other-hang", "name": "hang", "version": "0", "source": None})
        with self.assertRaises(ValueError):
            ci.check_graph(graph, self.root)


if __name__ == "__main__":
    unittest.main()

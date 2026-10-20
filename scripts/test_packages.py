import copy
import itertools
import unittest

import check_packages


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.packages = {}
        self.nodes = {}
        for name in check_packages.ALLOWED:
            self.package(name)
        self.members = list(self.packages)
        for name, dependencies in check_packages.ALLOWED.items():
            for dependency in dependencies:
                self.edge(name, dependency)
        self.package("mupdf")
        self.package("mupdf-sys")
        self.package("serde")
        self.edge("vpdf-pdf", "mupdf")
        self.edge("vpdf", "mupdf")
        self.edge("mupdf", "mupdf-sys")
        self.edge("vpdf-model", "serde")

    def package(self, name, identity=None):
        identity = identity or name
        self.packages[identity] = {
            "id": identity, "name": name,
            "manifest_path": f"/workspace/{identity}/Cargo.toml",
            "dependencies": [],
        }
        self.nodes[identity] = {"id": identity, "deps": []}

    def edge(self, owner, dependency, resolved=True, **options):
        declaration = {
            "name": self.packages[dependency]["name"],
            "path": f"/workspace/{dependency}",
            "kind": None, "target": None, "rename": None, "optional": False,
        }
        declaration.update(options)
        self.packages[owner]["dependencies"].append(declaration)
        if resolved:
            self.nodes[owner]["deps"].append({
                "pkg": dependency,
                "name": declaration["rename"] or declaration["name"],
                "dep_kinds": [{
                    "kind": declaration["kind"], "target": declaration["target"],
                }],
            })

    def metadata(self):
        return {
            "packages": list(self.packages.values()),
            "workspace_members": self.members,
            "resolve": {"nodes": list(self.nodes.values())},
        }

    def test_allowed_graph(self):
        self.assertEqual(check_packages.audit(self.metadata()), [])

    def test_gui_isolation(self):
        self.package("eframe")
        self.package("wgpu-core")
        self.package("helper")
        self.edge("eframe", "wgpu-core")
        self.edge("vpdf-gui", "eframe")
        self.assertEqual(check_packages.audit(self.metadata()), [])
        self.edge("vpdf", "helper")
        self.edge("helper", "eframe")
        self.assertIn("GUI transitive dependency: vpdf -> wgpu-core",
                      check_packages.audit(self.metadata()))
        self.edge("vpdf-ui", "eframe", optional=True, resolved=False)
        self.assertIn("GUI dependency: vpdf-ui -> eframe (normal)",
                      check_packages.audit(self.metadata()))

    def test_dependency_kinds(self):
        for kind, target, rename in itertools.product(
            (None, "dev", "build"), (None, "cfg(windows)"), (None, "document"),
        ):
            with self.subTest(kind=kind, target=target, rename=rename):
                self.setUp()
                self.edge("vpdf-ui", "vpdf-pdf", kind=kind,
                          target=target, rename=rename)
                errors = check_packages.audit(self.metadata())
                self.assertTrue(any(error.startswith("Forbidden dependency:")
                                    for error in errors), errors)

    def test_optional_declaration(self):
        for target in ("vpdf-pdf", "mupdf-sys"):
            with self.subTest(target=target):
                self.setUp()
                self.edge("vpdf-ui", target, resolved=False, optional=True)
                errors = check_packages.audit(self.metadata())
                self.assertTrue(any(f"vpdf-ui -> {target}" in error
                                    for error in errors), errors)

    def test_transitive_native(self):
        self.package("helper")
        self.edge("vpdf-terminal", "helper", kind="dev")
        self.edge("helper", "mupdf-sys", kind="build", target="cfg(windows)")
        errors = check_packages.audit(self.metadata())
        for owner in ("vpdf-terminal", "vpdf-ui"):
            self.assertIn(f"Native transitive dependency: {owner} -> mupdf-sys",
                          errors)

    def test_transitive_internal(self):
        self.package("helper")
        self.edge("vpdf-terminal", "helper")
        self.edge("helper", "vpdf-search")
        self.assertIn(
            "Forbidden transitive dependency: vpdf-terminal -> vpdf-search",
            check_packages.audit(self.metadata()),
        )

    def test_workspace_members(self):
        for change in ("missing", "extra", "duplicate"):
            with self.subTest(change=change):
                metadata = copy.deepcopy(self.metadata())
                if change == "missing":
                    metadata["workspace_members"].remove("vpdf-ui")
                else:
                    metadata["workspace_members"].append(
                        "serde" if change == "extra" else "vpdf-ui"
                    )
                self.assertTrue(check_packages.audit(metadata))

    def test_shadow_package(self):
        self.package("vpdf-model", "shadow")
        self.edge("vpdf-ui", "shadow")
        errors = check_packages.audit(self.metadata())
        self.assertTrue(any(error.startswith("Non-workspace dependency:")
                            for error in errors), errors)
        self.assertIn(
            "Non-workspace transitive dependency: vpdf-ui -> vpdf-model", errors
        )

    def test_registry_internal(self):
        self.edge("vpdf-ui", "vpdf-model", resolved=False, path=None)
        errors = check_packages.audit(self.metadata())
        self.assertTrue(any(error.startswith("Non-workspace dependency:")
                            for error in errors), errors)

    def test_incomplete_graph(self):
        for missing in ("workspace", "package", "node", "resolve"):
            with self.subTest(missing=missing):
                metadata = copy.deepcopy(self.metadata())
                if missing in ("workspace", "package"):
                    name = "vpdf-ui" if missing == "workspace" else "serde"
                    metadata["packages"] = [p for p in metadata["packages"]
                                            if p["id"] != name]
                elif missing == "node":
                    metadata["resolve"]["nodes"] = [
                        node for node in metadata["resolve"]["nodes"]
                        if node["id"] != "serde"
                    ]
                else:
                    metadata["resolve"] = None
                self.assertTrue(check_packages.audit(metadata))

    def test_external_cycle(self):
        self.package("helper")
        self.edge("serde", "helper")
        self.edge("helper", "serde")
        self.assertEqual(check_packages.audit(self.metadata()), [])

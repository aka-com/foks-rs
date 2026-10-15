"""Check scripts/check-crate-layers.py and the repository's crate-layer policy.

Run with: python3 -m unittest discover -s scripts -p 'test_check_crate_layers.py'
"""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

REPOSITORY = Path(__file__).resolve().parent.parent
SCRIPT = REPOSITORY / "scripts" / "check-crate-layers.py"

specification = importlib.util.spec_from_file_location("check_crate_layers", SCRIPT)
layers = importlib.util.module_from_spec(specification)
specification.loader.exec_module(layers)

POLICY = {
    "workspace": {"package_roots": ["crates/foks-*", "apps/desktop/src-tauri"]},
    "layer": {
        "core": {"packages": ["foks-proto"], "may_depend_on": []},
        "client": {"packages": ["foks-client"], "may_depend_on": ["core"]},
        "front-end": {"packages": ["foks-cli"], "may_depend_on": ["core"]},
        "server": {"packages": ["foks-server"], "may_depend_on": ["core"]},
        "test-support": {
            "packages": ["foks-testkit"],
            "may_depend_on": ["core", "client", "front-end", "server"],
            "test_only": True,
        },
    },
}


def dependency(root, name, kind=None, rename=None, path=None):
    return {
        "name": name,
        "kind": kind,
        "rename": rename,
        "path": str(root / (path or f"crates/{name}")),
    }


def metadata(root, graph, directories=None):
    """Build cargo metadata JSON for {package: [dependency, ...]}."""
    directories = directories or {}
    packages = []
    for name, dependencies in graph.items():
        directory = directories.get(name, f"crates/{name}")
        packages.append(
            {
                "id": f"path+file://{root / directory}#{name}@0.1.0",
                "name": name,
                "manifest_path": str(root / directory / "Cargo.toml"),
                "dependencies": dependencies,
            }
        )
    return {
        "packages": packages,
        "workspace_members": [package["id"] for package in packages],
    }


def write(root, path, text):
    path = root / path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class SyntheticGraphTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def graph(self, extra=None):
        """Return a conforming graph, with extra dependencies appended."""
        root = self.root
        graph = {
            "foks-proto": [],
            "foks-client": [dependency(root, "foks-proto")],
            "foks-cli": [dependency(root, "foks-proto")],
            "foks-server": [
                dependency(root, "foks-proto"),
                dependency(root, "foks-client", kind="dev"),
            ],
            "foks-testkit": [dependency(root, "foks-server"), dependency(root, "foks-client")],
        }
        for name, dependencies in (extra or {}).items():
            graph[name] = graph[name] + dependencies
        return graph

    def check(self, graph, policy=POLICY, directories=None):
        return layers.check(policy, metadata(self.root, graph, directories), self.root)

    def test_conforming_graph_passes_and_ignores_dev_dependencies(self):
        self.assertEqual(self.check(self.graph()), [])

    def test_disallowed_production_edge_fails(self):
        for kind in (None, "build"):
            edge = dependency(self.root, "foks-client", kind=kind)
            graph = self.graph({"foks-server": [edge]})
            self.assertEqual(
                self.check(graph),
                [
                    "foks-server (server) depends on foks-client (client); the layer rules "
                    "do not allow this edge and no exception covers it"
                ],
            )

    def test_layer_assignment_errors(self):
        graph = self.graph()
        graph["foks-extra"] = []
        policy = copy.deepcopy(POLICY)
        policy["layer"]["core"]["packages"].append("foks-missing")
        policy["layer"]["server"]["packages"].append("foks-proto")
        policy["layer"]["server"]["may_depend_on"].append("nowhere")
        self.assertEqual(
            sorted(self.check(graph, policy)),
            [
                "foks-extra (crates/foks-extra) is not assigned to a layer",
                "foks-proto is in layers core and server",
                "layer core lists foks-missing, which is not a workspace package",
                "layer server may depend on undefined layer nowhere",
            ],
        )

    def test_package_and_path_dependency_outside_workspace_fail(self):
        vendored = dependency(self.root, "vendored", kind="dev", path="vendor/vendored")
        graph = self.graph({"foks-client": [vendored]})
        errors = self.check(graph, directories={"foks-cli": "tools/foks-cli"})
        self.assertEqual(
            errors,
            [
                "foks-cli is defined outside the workspace package roots: tools/foks-cli",
                "foks-client has a path dependency outside the workspace: vendor/vendored",
            ],
        )

    def test_exception_fences_source_files(self):
        root = self.root
        write(root, "crates/foks-cli/src/main.rs", "use foks_client::Session;\n")
        write(root, "crates/foks-cli/src/state.rs", "// foks_client::Session is not used here\n")
        write(root, "crates/foks-cli/tests/seed.rs", "fn f() { foks_client::seed(); }\n")
        write(root, "crates/foks-cli/target/debug/build/out.rs", "use foks_client::Ignored;\n")
        graph = self.graph({"foks-cli": [dependency(root, "foks-client")]})
        policy = copy.deepcopy(POLICY)
        policy["exception"] = [
            {
                "package": "foks-cli",
                "dependency": "foks-client",
                "reason": "direct client",
                "sites": ["crates/foks-cli/src/main.rs"],
            },
            {
                "package": "foks-cli",
                "dependency": "foks-client",
                "reason": "test seeding",
                "sites": ["crates/foks-cli/tests/*.rs"],
            },
        ]
        self.assertEqual(self.check(graph, policy), [])

        write(root, "crates/foks-cli/src/new.rs", "use foks_client as client;\n")
        policy["exception"][0]["sites"].append("crates/foks-cli/src/gone.rs")
        self.assertEqual(
            self.check(graph, policy),
            [
                "crates/foks-cli/src/new.rs uses foks_client, but no foks-cli -> foks-client "
                "exception lists it as a site",
                "exception foks-cli -> foks-client site crates/foks-cli/src/gone.rs matches no "
                "file that uses foks_client",
            ],
        )

    def test_exception_uses_renamed_crate_identifier(self):
        write(self.root, "crates/foks-cli/src/main.rs", "use client_core::Session;\n")
        renamed = dependency(self.root, "foks-client", rename="client-core")
        graph = self.graph({"foks-cli": [renamed]})
        policy = copy.deepcopy(POLICY)
        policy["exception"] = [
            {
                "package": "foks-cli",
                "dependency": "foks-client",
                "reason": "renamed",
                "sites": ["crates/foks-cli/src/main.rs"],
            }
        ]
        self.assertEqual(self.check(graph, policy), [])

    def test_stale_unnecessary_and_incomplete_exceptions_fail(self):
        policy = copy.deepcopy(POLICY)
        policy["exception"] = [
            {"package": "foks-cli", "dependency": "foks-client", "reason": "gone", "sites": ["x"]},
            {"package": "foks-cli", "dependency": "foks-proto", "reason": "allowed", "sites": ["x"]},
            {"package": "foks-server", "dependency": "foks-client", "reason": "", "sites": []},
        ]
        self.assertEqual(
            self.check(self.graph(), policy),
            [
                "exception foks-server -> foks-client needs a reason and at least one site",
                "exception foks-cli -> foks-client names an edge that does not exist",
                "exception foks-cli -> foks-proto is unnecessary; the layer rules allow it",
                "exception foks-server -> foks-client names an edge that does not exist",
            ],
        )

    def test_transitive_production_dependency_on_test_only_package_fails(self):
        policy = copy.deepcopy(POLICY)
        policy["layer"]["client"]["may_depend_on"].append("test-support")
        graph = self.graph({"foks-client": [dependency(self.root, "foks-testkit")]})
        graph["foks-testkit"] = [dependency(self.root, "foks-proto")]
        self.assertEqual(
            self.check(graph, policy),
            ["foks-client has a production dependency on test-only foks-testkit"],
        )
        graph["foks-cli"].append(dependency(self.root, "foks-client"))
        policy["layer"]["front-end"]["may_depend_on"].append("client")
        self.assertEqual(
            self.check(graph, policy),
            [
                "foks-cli has a production dependency on test-only foks-testkit",
                "foks-client has a production dependency on test-only foks-testkit",
            ],
        )
        graph["foks-cli"][-1]["kind"] = "dev"
        graph["foks-client"][-1]["kind"] = "dev"
        self.assertEqual(self.check(graph, policy), [])


class RepositoryPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.policy = layers.load_policy(layers.DEFAULT_POLICY)
        cls.metadata = layers.cargo_metadata(REPOSITORY)

    def test_workspace_conforms_to_policy(self):
        self.assertEqual(layers.check(self.policy, self.metadata, REPOSITORY), [])

    def test_front_end_exceptions_are_required(self):
        policy = copy.deepcopy(self.policy)
        policy["exception"] = [
            exception
            for exception in policy["exception"]
            if exception["package"] != "foks-desktop-app"
        ]
        errors = layers.check(policy, self.metadata, REPOSITORY)
        self.assertIn(
            "foks-desktop-app (front-end) depends on foks-client-app (client); the layer "
            "rules do not allow this edge and no exception covers it",
            errors,
        )

    def test_server_layer_excludes_client_crates(self):
        server = self.policy["layer"]["server"]
        self.assertEqual(server["may_depend_on"], ["core"])
        self.assertEqual(sorted(server["packages"]), ["foks-server", "foks-server-db"])


if __name__ == "__main__":
    unittest.main()

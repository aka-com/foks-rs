"""Synthetic resolver evidence tests; these tests never invoke Cargo."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_graph as checker

NATIVE = "aarch64-apple-darwin"
WASM = "wasm32-unknown-unknown"


def fixture(target=NATIVE, profile="default"):
    root_features = [] if profile == "default" else ["qualification"]
    if profile == "test-support":
        root_features.append("test-support")
    names = [(checker.FIXTURE, root_features), ("foks-metadata-core", ["owned-list"])]
    if target != WASM:
        names += [("foks-hosted-transport", root_features), ("tokio", ["io-util", "sync", "time"])]
    packages = [{"id": name, "name": name} for name, _ in names]
    nodes = [{"id": name, "features": list(features), "deps": []} for name, features in names]
    nodes[0]["deps"] = [{"pkg": name, "dep_kinds": [{"kind": None, "target": None}]} for name, _ in names[1:]]
    return {"version": 1, "packages": packages, "resolve": {"root": checker.FIXTURE, "nodes": nodes}}


def add_dependency(metadata, name, kind=None, selector=None, identity=None):
    identity = identity or name
    metadata["packages"].append({"id": identity, "name": name})
    metadata["resolve"]["nodes"].append({"id": identity, "features": [], "deps": []})
    metadata["resolve"]["nodes"][0]["deps"].append({"pkg": identity, "dep_kinds": [{"kind": kind, "target": selector}]})


class GraphTests(unittest.TestCase):
    def test_all_native_feature_profiles_and_core_only_wasm(self):
        for target in (NATIVE, WASM):
            for profile in checker.PROFILES:
                with self.subTest(target=target, profile=profile):
                    result = checker.graph_report(fixture(target, profile), target, profile)
                    self.assertEqual(result["target"], target)
                    self.assertEqual(result["profile"], profile)

    def test_normal_and_build_forbidden_edges_are_not_hidden_by_cfg_selector(self):
        for name in checker.FORBIDDEN:
            for kind in (None, "build"):
                metadata = fixture()
                add_dependency(metadata, name, kind, 'cfg(target_os = "other")')
                with self.subTest(name=name, kind=kind), self.assertRaisesRegex(ValueError, "forbidden"):
                    checker.graph_report(metadata, NATIVE, "default")

    def test_dev_only_edges_do_not_enter_production_but_mixed_edges_do(self):
        metadata = fixture()
        add_dependency(metadata, "rusqlite", "dev")
        checker.graph_report(metadata, NATIVE, "default")
        metadata["resolve"]["nodes"][0]["deps"][-1]["dep_kinds"].append({"kind": "build", "target": None})
        with self.assertRaisesRegex(ValueError, "forbidden"):
            checker.graph_report(metadata, NATIVE, "default")

    def test_wasm_rejects_transport_and_tokio_edges(self):
        for name in ("tokio", "foks-hosted-transport"):
            metadata = fixture(WASM)
            add_dependency(metadata, name)
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "forbidden"):
                checker.graph_report(metadata, WASM, "default")

    def test_active_native_feature_or_wrong_public_profile_is_rejected(self):
        for name, feature in (("foks-metadata-core", "native"), ("foks-hosted-transport", "qualification"), (checker.FIXTURE, "test-support")):
            metadata = fixture()
            next(node for node in metadata["resolve"]["nodes"] if node["id"] == name)["features"].append(feature)
            with self.subTest(name=name), self.assertRaises(ValueError):
                checker.graph_report(metadata, NATIVE, "default")

    def test_multiple_versions_are_allowed_except_ambiguous_contract_package(self):
        metadata = fixture()
        add_dependency(metadata, "sha2", identity="sha2@0.10")
        add_dependency(metadata, "sha2", identity="sha2@0.11")
        result = checker.graph_report(metadata, NATIVE, "default")
        self.assertEqual(sum(package["name"] == "sha2" for package in result["packages"]), 2)
        add_dependency(metadata, "foks-metadata-core", identity="another-core")
        with self.assertRaises(ValueError):
            checker.graph_report(metadata, NATIVE, "default")

    def test_missing_or_unclassified_evidence_fails_closed(self):
        edits = [
            lambda data: data.pop("resolve"),
            lambda data: data["resolve"].pop("root"),
            lambda data: data["resolve"]["nodes"].pop(),
            lambda data: data["packages"].pop(),
            lambda data: data["resolve"]["nodes"][0].pop("features"),
            lambda data: data["resolve"]["nodes"][0].pop("deps"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0].pop("dep_kinds"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].pop("kind"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].pop("target"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].update(kind="unknown"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].update(target=17),
        ]
        for index, edit in enumerate(edits):
            metadata = fixture()
            edit(metadata)
            with self.subTest(index=index), self.assertRaises(ValueError):
                checker.graph_report(metadata, NATIVE, "default")
        for target in (None, "", "native", "wasm64-unknown-unknown"):
            with self.subTest(target=target), self.assertRaises(ValueError):
                checker.graph_report(fixture(), target, "default")

    def test_command_and_output_target_binding_cannot_be_relabelled(self):
        manifest = (Path(checker.__file__).resolve().parent / "Cargo.toml").resolve()
        commands = checker.commands(manifest, NATIVE, "default")
        evidence = {"format": 1, "manifest": str(manifest), "target": WASM, "profile": "default", "commands": {name: {"argv": argv, "exit_code": 0} for name, argv in commands.items()}}
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            (directory / "evidence.json").write_text(json.dumps(evidence))
            with self.assertRaisesRegex(ValueError, "selected target/profile"):
                checker.verify(directory)
            evidence["target"] = NATIVE
            for name, command in evidence["commands"].items():
                for stream in ("stdout", "stderr"):
                    (directory / (name + "." + stream)).write_bytes(b"original")
                    command[stream + "_sha256"] = checker.digest(b"original")
            (directory / "evidence.json").write_text(json.dumps(evidence))
            (directory / "metadata.stdout").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                checker.verify(directory)

    def test_collection_requires_new_directory_before_any_cargo_invocation(self):
        with tempfile.TemporaryDirectory() as name, patch.object(checker.subprocess, "run") as run:
            with self.assertRaises(FileExistsError):
                checker.collect(name, NATIVE, "default")
            run.assert_not_called()

    def test_failed_collection_keeps_command_exit_and_outputs_without_report(self):
        with tempfile.TemporaryDirectory() as name:
            repo = Path(name)
            fixture_dir = repo / "tools/foks-hosted-transport-check"
            fixture_dir.mkdir(parents=True)
            for path in (repo / "Cargo.toml", fixture_dir / "Cargo.toml", fixture_dir / "Cargo.lock"):
                path.write_text("fixture input")
            result = checker.subprocess.CompletedProcess([], 101, b"original stdout", b"original failure")
            with patch.object(checker, "__file__", str(fixture_dir / "check_graph.py")), patch.object(checker.subprocess, "run", return_value=result) as run:
                evidence_dir = repo / "evidence"
                with self.assertRaisesRegex(ValueError, "command failed"):
                    checker.collect(evidence_dir, NATIVE, "default")
                self.assertEqual(run.call_count, 1)
                evidence = json.loads((evidence_dir / "evidence.json").read_text())
                self.assertEqual(evidence["commands"]["metadata"]["exit_code"], 101)
                self.assertEqual((evidence_dir / "metadata.stderr").read_bytes(), b"original failure")
                self.assertFalse((evidence_dir / "report.json").exists())
                with self.assertRaisesRegex(ValueError, "missing command"):
                    checker.verify(evidence_dir)

    def test_local_input_inventory_rejects_escape_and_binds_root_manifest(self):
        manifest = (Path(checker.__file__).resolve().parent / "Cargo.toml").resolve()
        metadata = {"packages": [{"source": None, "manifest_path": str(manifest)}]}
        self.assertIn(manifest.parents[2] / "Cargo.toml", checker.input_paths(metadata, manifest))
        bad = copy.deepcopy(metadata)
        bad["packages"][0]["manifest_path"] = "/private/tmp/Cargo.toml"
        with self.assertRaisesRegex(ValueError, "escapes"):
            checker.input_paths(bad, manifest)


if __name__ == "__main__":
    unittest.main()

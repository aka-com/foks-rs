"""Closed graph/lock evidence tests; no Cargo invocation or dependency fetch."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_graph as c
from test_prepare_runtime import a2_fixture, historical_pins


def fixture(mode="identity", profile=c.preparation.A1):
    pins = historical_pins()
    if profile == c.preparation.A2:
        pins.update(format=2, profile=profile)
    names = {
        c.FIXTURE: ("0.0.0", ["lifecycle"] if mode == "lifecycle" else []), "celld": ("0.6.2", ["foks-guarded-qualification"] if mode == "lifecycle" else []),
        "foks-hosted-transport": ("0.4.0", ["qualification"]),
        "foks-metadata-core": ("0.4.0", ["owned-list"]),
        "tokio": ("1.53.1", ["full"]),
        "v8": ("152.1.0", ["default", "use_custom_libcxx"]),
        "rusqlite": ("0.31.0", ["backup", "bundled", "hooks", "serialize"]),
        "libsqlite3-sys": ("0.28.0", ["bundled"]), "sqlite-vec": ("0.1.9", []),
    }
    packages = [{"id": name, "name": name, "version": version, "manifest_path": "/fixture/" + name + "/Cargo.toml", "source": None if name in {c.FIXTURE, "celld", "foks-hosted-transport", "foks-metadata-core"} else c.preparation.CRATES_IO} for name, (version, _) in names.items()]
    next(package for package in packages if package["name"] == "libsqlite3-sys")["links"] = "sqlite3"
    nodes = [{"id": name, "features": features, "deps": []} for name, (_, features) in names.items()]
    metadata = {"version": 1, "packages": packages, "workspace_members": [c.FIXTURE], "resolve": {"root": c.FIXTURE, "nodes": nodes}}
    for child in ("celld", "foks-hosted-transport", "foks-metadata-core"):
        edge(metadata, c.FIXTURE, child)
    if mode == "lifecycle":
        edge(metadata, c.FIXTURE, "tokio")
    for child in ("v8", "rusqlite", "sqlite-vec", "tokio"):
        edge(metadata, "celld", child)
    edge(metadata, "rusqlite", "libsqlite3-sys")
    edge(metadata, "foks-hosted-transport", "foks-metadata-core")
    edge(metadata, "foks-hosted-transport", "tokio")
    return metadata, pins


def edge(metadata, parent, child, kind=None, target=None):
    node = next(item for item in metadata["resolve"]["nodes"] if item["id"] == parent)
    node["deps"].append({"pkg": child, "dep_kinds": [{"kind": kind, "target": target}]})


def add_package(metadata, name, version="1.0.0", identity=None, features=None, source=c.preparation.CRATES_IO):
    identity = identity or name
    metadata["packages"].append({"id": identity, "name": name, "version": version, "source": source, "manifest_path": "/fixture/" + identity + "/Cargo.toml"})
    metadata["resolve"]["nodes"].append({"id": identity, "features": features or [], "deps": []})
    return identity


class GraphTests(unittest.TestCase):
    def test_workspace_cannot_adopt_dependency_or_unreachable_packages(self):
        for members in ([], None, ["celld"], [c.FIXTURE, c.FIXTURE], [c.FIXTURE, "celld"], [c.FIXTURE, "unreachable-member"]):
            with self.subTest(members=members):
                metadata, pins = fixture()
                add_package(metadata, "unreachable-member", source=None)
                c.graph_report(metadata, pins)
                metadata["workspace_members"] = members
                with self.assertRaisesRegex(ValueError, "only the selected harness root"):
                    c.graph_report(metadata, pins)

    def test_celld_engine_is_allowed_but_foks_subclosure_is_sqlite_free(self):
        metadata, pins = fixture()
        report = c.graph_report(metadata, pins)
        self.assertIn("libsqlite3-sys", {item["name"] for item in report["packages"]})
        self.assertNotIn("libsqlite3-sys", report["foks_subclosure"])
        self.assertIn("tokio", report["foks_subclosure"])
        # Global Tokio feature unification with celld/full is legitimate. This
        # proof excludes native SQLite, not celld's own runtime features.
        edge(metadata, "foks-metadata-core", "rusqlite")
        with self.assertRaisesRegex(ValueError, "FOKS transport/core subclosure"):
            c.graph_report(metadata, pins)

    def test_full_server_and_alternate_sqlite_providers_are_rejected(self):
        for name in (*c.FORBIDDEN, "sqlx-sqlite", "other-native-engine"):
            with self.subTest(name=name):
                metadata, pins = fixture()
                identity = add_package(metadata, name)
                if name == "other-native-engine":
                    metadata["packages"][-1]["links"] = "sqlite3"
                edge(metadata, c.FIXTURE, identity, kind="build", target='cfg(target_os = "other")')
                with self.assertRaises(ValueError):
                    c.graph_report(metadata, pins)

    def test_wrong_engine_identity_features_or_source_is_rejected(self):
        for name in ("v8", "rusqlite", "libsqlite3-sys", "sqlite-vec"):
            for mutation in ("version", "source", "duplicate"):
                with self.subTest(name=name, mutation=mutation):
                    metadata, pins = fixture()
                    package = next(item for item in metadata["packages"] if item["name"] == name)
                    if mutation == "version":
                        package["version"] = "999.0.0"
                    elif mutation == "source":
                        package["source"] = None
                    else:
                        identity = add_package(metadata, name, identity="another-" + name)
                        edge(metadata, c.FIXTURE, identity)
                    with self.assertRaises(ValueError):
                        c.graph_report(metadata, pins)
        for feature in ("v8_enable_pointer_compression", "v8_enable_sandbox", "v8_enable_v8_checks"):
            metadata, pins = fixture()
            next(node for node in metadata["resolve"]["nodes"] if node["id"] == "v8")["features"].append(feature)
            with self.subTest(feature=feature), self.assertRaisesRegex(ValueError, "features"):
                c.graph_report(metadata, pins)
        metadata, pins = fixture()
        next(node for node in metadata["resolve"]["nodes"] if node["id"] == "rusqlite")["features"].remove("bundled")
        with self.assertRaisesRegex(ValueError, "SQLite construction"):
            c.graph_report(metadata, pins)

    def test_foks_native_and_test_support_features_cannot_enter(self):
        for name, feature in (("foks-metadata-core", "native"), ("foks-hosted-transport", "test-support"), (c.FIXTURE, "unreviewed")):
            metadata, pins = fixture()
            next(node for node in metadata["resolve"]["nodes"] if node["id"] == name)["features"].append(feature)
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "features"):
                c.graph_report(metadata, pins)

    def test_dev_only_edges_are_excluded_and_mixed_edges_are_followed(self):
        metadata, pins = fixture()
        identity = add_package(metadata, "foks-server")
        edge(metadata, c.FIXTURE, identity, kind="dev")
        c.graph_report(metadata, pins)
        metadata["resolve"]["nodes"][0]["deps"][-1]["dep_kinds"].append({"kind": "build", "target": None})
        with self.assertRaisesRegex(ValueError, "forbidden"):
            c.graph_report(metadata, pins)

    def test_missing_target_root_package_features_and_edges_fail_closed(self):
        mutations = [
            lambda data: data.pop("resolve"),
            lambda data: data.pop("workspace_members"),
            lambda data: data["resolve"].pop("root"),
            lambda data: data["packages"].pop(),
            lambda data: data["resolve"]["nodes"].pop(),
            lambda data: data["resolve"]["nodes"][0].pop("features"),
            lambda data: data["resolve"]["nodes"][0].pop("deps"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0].pop("dep_kinds"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].pop("target"),
            lambda data: data["resolve"]["nodes"][0]["deps"][0]["dep_kinds"][0].update(kind="unknown"),
        ]
        for index, mutation in enumerate(mutations):
            metadata, pins = fixture()
            mutation(metadata)
            with self.subTest(index=index), self.assertRaises(ValueError):
                c.graph_report(metadata, pins)
        metadata, pins = fixture()
        pins["target"] = "wasm32-unknown-unknown"
        with self.assertRaisesRegex(ValueError, "target/profile"):
            c.graph_report(metadata, pins)

    def test_verifier_rejects_relabelled_target_before_reading_commands(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "evidence.json").write_text(json.dumps({"format": 1, "target": "other", "profile": "a1-library-identity"}))
            with patch.object(c.preparation, "load_pins", return_value=historical_pins()), self.assertRaisesRegex(ValueError, "target/profile"):
                c.verify(path)

    def test_saved_evidence_requires_every_native_tool_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            pins = historical_pins()
            toolchain = path / "toolchain"
            environment = {"A1_FIXTURE_ENVIRONMENT": "fixed"}
            native_paths = tuple(str(path / name) for name in ("native-cc", "native-ld"))
            for name in native_paths:
                Path(name).write_bytes(b"fixed native input")
            def command_record(name, argv):
                stdout = "commit-hash: " + pins["toolchain"]["rustc_commit"] + "\n" if name == "rustc-version" else "fixed command output\n"
                (path / (name + ".stdout")).write_text(stdout)
                (path / (name + ".stderr")).write_bytes(b"")
                return {
                    "argv": list(map(str, argv)), "cwd": str(c.ROOT),
                    "exit_code": 0, "reason": "completed", "cleanup_established": True,
                    **{stream + "_sha256": c.preparation.file_hash(path / (name + "." + stream)) for stream in ("stdout", "stderr")},
                }
            with patch.object(c.preparation, "load_pins", return_value=pins), patch.object(c.preparation, "controlled_environment", return_value=(toolchain, environment)), patch.object(c.preparation, "config_inputs", return_value={}), patch.object(c.preparation, "network_command", side_effect=lambda argv: list(map(str, argv))), patch.object(c.preparation, "native_tool_paths", return_value=native_paths), patch.object(c, "graph_report") as graph:
                evidence = {
                    "format": 1, "target": c.TARGET, "profile": pins["profile"],
                    "manifest": str(c.ROOT / "Cargo.toml"), "cwd": str(c.ROOT),
                    "commands": {name: command_record(name, argv) for name, argv in c.commands(c.ROOT, toolchain).items()},
                    "tools": {
                        **{name: command_record(name, argv) for name, argv in c.preparation.identity_commands(c.ROOT, toolchain).items()},
                        "environment": environment, "cargo_configs": {},
                        # The retained entry is valid. Omitting the other tool
                        # must still reject this saved evidence before parsing.
                        "native_tools": {native_paths[0]: c.preparation.file_hash(native_paths[0])},
                    },
                }
                (path / "evidence.json").write_text(json.dumps(evidence))
                with self.assertRaisesRegex(ValueError, "native compiler/linker/SDK input inventory"):
                    c.verify(path)
                graph.assert_not_called()

    def test_collection_refuses_existing_directory_before_cargo(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(c.preparation, "run_owned") as run:
            with self.assertRaises(FileExistsError):
                c.collect(directory)
            run.assert_not_called()


class ModeTests(unittest.TestCase):
    def test_a2_modes_require_exact_harness_celld_and_direct_tokio_edges(self):
        for mode in ("identity", "lifecycle"):
            metadata, pins = fixture(mode, c.preparation.A2)
            report = c.graph_report(metadata, pins, mode)
            self.assertEqual(report["mode"], mode)
            self.assertEqual(report["profile"], c.preparation.A2)
            for package, feature in ((c.FIXTURE, "tokio"), ("celld", "unreviewed"), ("foks-metadata-core", "native"), ("foks-hosted-transport", "test-support")):
                altered = copy.deepcopy(metadata)
                next(node for node in altered["resolve"]["nodes"] if node["id"] == package)["features"].append(feature)
                with self.subTest(mode=mode, package=package), self.assertRaisesRegex(ValueError, "features"):
                    c.graph_report(altered, pins, mode)
            with self.assertRaisesRegex(ValueError, "features"):
                c.graph_report(metadata, pins, "lifecycle" if mode == "identity" else "identity")
            root = next(node for node in metadata["resolve"]["nodes"] if node["id"] == c.FIXTURE)
            if mode == "identity":
                edge(metadata, c.FIXTURE, "tokio")
            else:
                root["deps"] = [dependency for dependency in root["deps"] if dependency["pkg"] != "tokio"]
            with self.assertRaisesRegex(ValueError, "direct optional Tokio"):
                c.graph_report(metadata, pins, mode)

    def test_a2_requires_one_exact_tokio_version_and_registry_source(self):
        for mutation in ("version", "source", "duplicate"):
            metadata, pins = fixture("lifecycle", c.preparation.A2)
            tokio = next(package for package in metadata["packages"] if package["id"] == "tokio")
            if mutation == "version":
                tokio["version"] = "1.53.2"
            elif mutation == "source":
                tokio["source"] = None
            else:
                identity = add_package(metadata, "tokio", version="1.53.1", identity="second-tokio")
                edge(metadata, c.FIXTURE, identity)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                c.graph_report(metadata, pins, "lifecycle")

    def test_a1_cannot_select_lifecycle_and_unknown_mode_cannot_fall_back(self):
        metadata, pins = fixture()
        for mode in ("lifecycle", "unknown", "", None):
            with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, "mode"):
                c.graph_report(metadata, pins, mode)

    def test_metadata_and_tree_commands_bind_selected_target_and_mode(self):
        root, toolchain = Path("/fixture"), Path("/toolchain")
        for mode in ("identity", "lifecycle"):
            _, pins = fixture(mode, c.preparation.A2)
            with patch.object(c.preparation, "load_pins", return_value=pins), patch.object(c.preparation, "network_command", side_effect=lambda argv: list(map(str, argv))):
                commands = c.commands(root, toolchain, mode)
            for name, argv in commands.items():
                self.assertIn("--locked", argv)
                self.assertIn("--offline", argv)
                self.assertIn("--no-default-features", argv)
                target = "--filter-platform" if name == "metadata" else "--target"
                self.assertEqual(argv[argv.index(target) + 1], c.TARGET)
                self.assertEqual("--features" in argv, mode == "lifecycle")
                if mode == "lifecycle":
                    self.assertEqual(argv[argv.index("--features") + 1], "lifecycle")

    def test_a2_saved_evidence_cannot_omit_or_relabel_mode(self):
        for mode, recorded in (("identity", None), ("identity", "lifecycle"), ("lifecycle", None), ("lifecycle", "identity")):
            with self.subTest(mode=mode, recorded=recorded), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                _, pins = fixture(mode, c.preparation.A2)
                evidence = {"format": 1, "target": c.TARGET, "profile": c.preparation.A2, "manifest": str(c.ROOT / "Cargo.toml"), "cwd": str(c.ROOT)}
                if recorded is not None:
                    evidence["mode"] = recorded
                (path / "evidence.json").write_text(json.dumps(evidence))
                with patch.object(c.preparation, "load_pins", return_value=pins), patch.object(c.preparation, "controlled_environment") as environment, self.assertRaisesRegex(ValueError, "evidence mode"):
                    c.verify(path, mode=mode)
                environment.assert_not_called()

    def test_a1_implicit_identity_evidence_remains_historical_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            evidence = {"format": 1, "target": c.TARGET, "profile": c.preparation.A1, "manifest": str(c.ROOT / "Cargo.toml"), "cwd": str(c.ROOT)}
            (path / "evidence.json").write_text(json.dumps(evidence))
            with patch.object(c.preparation, "load_pins", return_value=historical_pins()), patch.object(c.preparation, "controlled_environment", side_effect=ValueError("reached actual environment validation")) as environment:
                with self.assertRaisesRegex(ValueError, "reached actual environment"):
                    c.verify(path)
                environment.assert_called_once_with(c.ROOT)
            evidence["mode"] = "lifecycle"
            (path / "evidence.json").write_text(json.dumps(evidence))
            with patch.object(c.preparation, "load_pins", return_value=historical_pins()), self.assertRaisesRegex(ValueError, "evidence mode"):
                c.verify(path)

    def test_a2_both_modes_bind_outer_lifecycle_sources_and_full_postimage(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory).resolve()
            root = repo / "tools/foks-celld-guarded"
            root.parent.mkdir()
            pins, archive, _, _, _ = a2_fixture(root)
            c.preparation.prepare(archive, root)
            for name in c.source_paths(pins):
                path = root / name
                if not path.exists():
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(("reviewed " + name + "\n").encode())
            (repo / "Cargo.toml").write_bytes(b"reviewed workspace\n")
            baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
            baseline.parent.mkdir(parents=True, exist_ok=True)
            baseline.write_bytes(b"reviewed baseline lock\n")
            for name in ("foks-metadata-core", "foks-hosted-transport"):
                path = repo / "crates" / name / "Cargo.toml"
                path.parent.mkdir(parents=True)
                path.write_bytes(b"reviewed native-disabled crate\n")
                path.with_name("lib.rs").write_bytes(b"reviewed source\n")
            inventories = []
            for mode in ("identity", "lifecycle"):
                metadata, graph_pins = fixture(mode, c.preparation.A2)
                for package in metadata["packages"]:
                    if package["source"] is None:
                        package["manifest_path"] = str(root / "Cargo.toml" if package["name"] == c.FIXTURE else root / "runtime/source/crates/celld/Cargo.toml" if package["name"] == "celld" else repo / "crates" / package["name"] / "Cargo.toml")
                reached = {package["id"] for package in c.graph_report(metadata, graph_pins, mode)["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    candidates = c.candidate_inputs(root)
                    selected = c.selected_inputs(metadata, reached, root)
                self.assertTrue(all(candidates.get(path) == checksum for path, checksum in selected.items()))
                for name in c.A2_SOURCE_PATHS:
                    self.assertIn(str(root / name), selected)
                for path in (root / "runtime/source").rglob("*"):
                    if path.is_file():
                        self.assertIn(str(path), selected)
                inventories.append(selected)
            self.assertEqual(inventories[0], inventories[1])
            changed = root / "tests/lifecycle.rs"
            changed.write_bytes(b"changed integration oracle\n")
            with patch.object(c.preparation, "config_inputs", return_value={}):
                current = c.selected_inputs(metadata, reached, root)
            self.assertNotEqual(current[str(changed)], inventories[0][str(changed)])


class NodeGraphTests(unittest.TestCase):
    def fixture(self, root=Path("/fixture")):
        metadata, pins = fixture("node-baseline", c.preparation.A2)
        celld = next(package for package in metadata["packages"] if package["name"] == "celld")
        celld["manifest_path"] = str(root / "runtime/source/crates/celld/Cargo.toml")
        celld["targets"] = [{"name": "celld", "kind": ["lib"], "crate_types": ["lib"], "src_path": str(root / "runtime/source/crates/celld/lib.rs")}, {"name": "celld", "kind": ["bin"], "crate_types": ["bin"], "src_path": str(root / "runtime/source/crates/celld/main.rs")}]
        return metadata, pins

    def test_node_requires_actual_prepared_bin_and_sqlite_free_foks_subclosure(self):
        root = Path("/fixture")
        metadata, pins = self.fixture(root)
        report = c.graph_report(metadata, pins, "node-baseline", root)
        self.assertEqual(report["node_target"], {"package_id": "celld", "name": "celld", "kind": ["bin"], "crate_types": ["bin"], "src_path": "/fixture/runtime/source/crates/celld/main.rs", "features": []})
        self.assertIn("rusqlite", report["node_subclosure"])
        self.assertNotIn("foks-hosted-transport", report["node_subclosure"])
        self.assertNotIn("rusqlite", report["foks_subclosure"])
        mutations = [
            lambda package: package.update(manifest_path="/different/Cargo.toml"),
            lambda package: package.update(source=c.preparation.CRATES_IO),
            lambda package: package.pop("targets"),
            lambda package: package["targets"].append(copy.deepcopy(package["targets"][1])),
            lambda package: package["targets"][1].update(src_path="/different/main.rs"),
            lambda package: package["targets"][1].update(required_features=["ignored"] , **{"required-features": ["foks-guarded-qualification"]}),
            lambda package: package["targets"][1].update(crate_types=["lib"]),
            lambda package: package["targets"][1].update(name="other"),
        ]
        for mutation in mutations:
            altered = copy.deepcopy(metadata)
            mutation(next(package for package in altered["packages"] if package["name"] == "celld"))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                c.graph_report(altered, pins, "node-baseline", root)
        edge(metadata, "celld", "foks-hosted-transport")
        with self.assertRaisesRegex(ValueError, "ordinary celld binary subclosure"):
            c.graph_report(metadata, pins, "node-baseline", root)

    def test_node_features_and_optional_root_tokio_cannot_enter(self):
        for package, feature in ((c.FIXTURE, "lifecycle"), ("celld", "foks-guarded-qualification")):
            metadata, pins = self.fixture()
            next(node for node in metadata["resolve"]["nodes"] if node["id"] == package)["features"].append(feature)
            with self.subTest(package=package), self.assertRaisesRegex(ValueError, "features"):
                c.graph_report(metadata, pins, "node-baseline", Path("/fixture"))
        metadata, pins = self.fixture()
        edge(metadata, c.FIXTURE, "tokio")
        with self.assertRaisesRegex(ValueError, "direct optional Tokio"):
            c.graph_report(metadata, pins, "node-baseline", Path("/fixture"))
        metadata, pins = fixture()
        with self.assertRaisesRegex(ValueError, "mode"):
            c.graph_report(metadata, pins, "node-baseline")

    def test_node_tree_selects_both_packages_but_metadata_has_no_package_filter(self):
        root, toolchain = Path("/fixture"), Path("/toolchain")
        _, pins = self.fixture()
        with patch.object(c.preparation, "load_pins", return_value=pins), patch.object(c.preparation, "network_command", side_effect=lambda args: list(map(str, args))):
            commands = c.commands(root, toolchain, "node-baseline")
        self.assertNotIn("--package", commands["metadata"])
        tree = commands["tree"]
        self.assertEqual([tree[index + 1] for index, value in enumerate(tree) if value == "--package"], [c.FIXTURE, "celld@0.6.2"])
        for argv in commands.values():
            self.assertNotIn("--features", argv)
            self.assertIn("--no-default-features", argv)
            self.assertIn("--locked", argv)
            self.assertIn("--offline", argv)
        self.assertEqual(commands["metadata"][commands["metadata"].index("--filter-platform") + 1], c.TARGET)
        self.assertEqual(tree[tree.index("--target") + 1], c.TARGET)

    def test_only_node_mode_extends_input_roster_and_requires_fixture(self):
        _, pins = self.fixture()
        prior = set(c.source_paths(pins, "identity"))
        self.assertEqual(prior, set(c.source_paths(pins, "lifecycle")))
        self.assertEqual(set(c.source_paths(pins, "node-baseline")) - prior, {"node.lock.json", "run-node.py", "test_run_node.py"})
        self.assertNotIn("node.lock.json", c.source_paths(historical_pins()))
        with self.assertRaisesRegex(ValueError, "mode"):
            c.source_paths(historical_pins(), "node-baseline")

    def test_saved_node_evidence_cannot_be_relabelled_as_library_identity(self):
        for requested, recorded in (("identity", "node-baseline"), ("lifecycle", "node-baseline"), ("node-baseline", "identity"), ("node-baseline", None)):
            with self.subTest(requested=requested, recorded=recorded), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                _, pins = self.fixture()
                evidence = {"format": 1, "target": c.TARGET, "profile": c.preparation.A2, "manifest": str(c.ROOT / "Cargo.toml"), "cwd": str(c.ROOT), "mode": recorded}
                (root / "evidence.json").write_text(json.dumps(evidence))
                with patch.object(c.preparation, "load_pins", return_value=pins), patch.object(c.preparation, "controlled_environment") as environment, patch.object(c.preparation, "node_inputs") as node_inputs:
                    with self.assertRaisesRegex(ValueError, "evidence mode"):
                        c.verify(root, mode=requested)
                    environment.assert_not_called()
                    node_inputs.assert_not_called()


def write_lock(path, packages):
    lines = ["version = 3", ""]
    for package in packages:
        lines.append("[[package]]")
        for field in ("name", "version", "source", "checksum", "dependencies"):
            if field in package:
                lines.append(field + " = " + json.dumps(package[field]))
        lines.append("")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines))


def lock_fixture(parent):
    root = parent / "tools/foks-celld-guarded"
    root.mkdir(parents=True)
    (root / "patches").mkdir()
    (root / "patches/series.json").write_bytes(b'{"format":1,"patches":[]}\n')
    pins = historical_pins()
    old = {"name": "sha2", "version": "0.10.9", "source": c.preparation.CRATES_IO, "checksum": "a" * 64}
    new = next(copy.deepcopy(item) for item in pins["foks_additional_versions"]["allowed"] if item["name"] == "sha2")
    engines = [{"name": name, "source": c.preparation.CRATES_IO, **expected} for name, expected in pins["registry_engines"].items()]
    before = [{"name": "celld", "version": "0.6.2", "dependencies": ["sha2"]}, old, *engines]
    after = [*before, new]
    write_lock(root / "runtime/source/Cargo.lock", before)
    write_lock(root / "Cargo.lock", after)
    baseline = parent / pins["foks_additional_versions"]["baseline_lock"]
    write_lock(baseline, [new])
    pins["foks_additional_versions"]["baseline_lock_sha256"] = c.preparation.file_hash(baseline)
    pins["foks_additional_versions"]["allowed"] = [new]
    (root / "runtime.lock.json").write_text(json.dumps(pins))
    return root, before, after, baseline


class LockTests(unittest.TestCase):
    def test_exact_approved_addition_retains_upstream_and_reports_pruning(self):
        with tempfile.TemporaryDirectory() as directory:
            root, before, after, _ = lock_fixture(Path(directory))
            old_extra = {"name": "unused-dev", "version": "1.0.0", "source": c.preparation.CRATES_IO, "checksum": "e" * 64}
            write_lock(root / "runtime/source/Cargo.lock", [*before, old_extra])
            report = c.preparation.compare_locks(root)
            self.assertEqual(report["added"][0]["version"], "0.11.0")
            self.assertEqual(report["removed_unreachable_candidates"][0]["name"], "unused-dev")
            self.assertIn({"name": "sha2", "version": "0.10.9", "source": c.preparation.CRATES_IO}, report["retained"])

    def test_wrong_additional_version_checksum_source_or_baseline_refuses(self):
        for mutation in ("version", "checksum", "source", "baseline", "missing-baseline", "unknown-name-version", "retained-checksum", "engine"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root, before, after, baseline = lock_fixture(Path(directory))
                if mutation == "version":
                    after[-1]["version"] = "0.11.1"
                elif mutation == "checksum":
                    after[-1]["checksum"] = "0" * 64
                elif mutation == "source":
                    after[-1]["source"] = "registry+https://example.invalid/index"
                elif mutation == "baseline":
                    baseline.write_text("version = 3\n")
                elif mutation == "missing-baseline":
                    baseline.unlink()
                elif mutation == "unknown-name-version":
                    before.append({"name": "unknown", "version": "1.0.0", "source": c.preparation.CRATES_IO, "checksum": "b" * 64})
                    after.append({"name": "unknown", "version": "2.0.0", "source": c.preparation.CRATES_IO, "checksum": "c" * 64})
                    write_lock(root / "runtime/source/Cargo.lock", before)
                elif mutation == "retained-checksum":
                    after[1] = {**after[1], "checksum": "0" * 64}
                else:
                    next(package for package in after if package["name"] == "v8")["version"] = "153.0.0"
                write_lock(root / "Cargo.lock", after)
                with self.assertRaises((ValueError, OSError)):
                    c.preparation.compare_locks(root)

    def test_allowed_duplicate_cannot_replace_an_actual_upstream_edge(self):
        with tempfile.TemporaryDirectory() as directory:
            root, _, _, _ = lock_fixture(Path(directory))
            packages = [
                {"id": "celld", "name": "celld", "version": "0.6.2", "source": None},
                {"id": "old-sha", "name": "sha2", "version": "0.10.9", "source": c.preparation.CRATES_IO},
                {"id": "new-sha", "name": "sha2", "version": "0.11.0", "source": c.preparation.CRATES_IO},
            ]
            nodes = [{"id": package["id"], "deps": []} for package in packages]
            metadata = {"packages": packages, "resolve": {"nodes": nodes}}
            edge(metadata, "celld", "old-sha")
            reached = {package["id"] for package in packages}
            self.assertEqual(c.preparation.upstream_edges(metadata, reached, root), [{"from": "celld", "to": "old-sha"}])
            nodes[0]["deps"][0]["pkg"] = "new-sha"
            with self.assertRaisesRegex(ValueError, "replaced or extended"):
                c.preparation.upstream_edges(metadata, reached, root)


if __name__ == "__main__":
    unittest.main()

"""Closed graph/lock evidence tests; no Cargo invocation or dependency fetch."""
import copy
from contextlib import ExitStack, contextmanager
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_graph as c
from test_prepare_runtime import a2_fixture, foundation_fixture, lifetimes_fixture, frontend_fixture, observations_fixture, process_fixture, main_work_fixture, main_work_predecessor_validation, contention_fixture, historical_pins, p


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


@contextmanager
def foundation_graph_fixture(root, mode, qualifier_revision=False):
    root.parent.mkdir(parents=True, exist_ok=True)
    stage = p.stage_for_mode(mode)
    fixture_source = {p.FOUNDATION: foundation_fixture, p.LIFETIMES: lifetimes_fixture, p.FRONTEND: frontend_fixture, p.OBSERVATIONS: observations_fixture, p.PROCESS: process_fixture, p.MAIN_WORK: main_work_fixture}[stage]
    if qualifier_revision:
        if stage != p.MAIN_WORK:
            raise ValueError("qualifier revision requires the main-work stage")
        fixture_source = contention_fixture
    with fixture_source(root) as data, ExitStack() as stack:
        for name in ("FOUNDATION_BASE_IMAGES", "FOUNDATION_LOCK_BEFORE", "LIFETIMES_BASE_IMAGES", "LIFETIMES_LOCK_SHA256", "LIFETIMES_ASSET_SHA256", "FRONTEND_BASE_IMAGES", "FRONTEND_ASSET_SHA256", "OBSERVATIONS_BASE_IMAGES", "OBSERVATIONS_ASSET_SHA256", "PROCESS_BASE_IMAGES", "PROCESS_ASSET_SHA256", "MAIN_WORK_BASE_IMAGES", "CONTENTION_BASE_IMAGES"):
            stack.enter_context(patch.object(c.preparation, name, getattr(p, name)))
        if stage == p.MAIN_WORK and not qualifier_revision:
            stack.enter_context(main_work_predecessor_validation(c.preparation))
            stack.enter_context(patch.object(c, "graph_report", c.graph_report_predecessor))
            stack.enter_context(patch.object(c, "source_paths", c.source_paths_predecessor))
        p.prepare(data["archive"], root, stage)
        (root / "Cargo.lock").write_bytes(data["expected_lock"])
        pins = c.preparation.load_pins(root, stage)
        metadata, _ = fixture("lifecycle" if mode.endswith("lifecycle") else "identity", p.A2)
        contract = p.NODE_MODES[mode]
        for node in metadata["resolve"]["nodes"]:
            if node["id"] == c.FIXTURE:
                node["features"] = contract["root_features"].copy()
            elif node["id"] == "celld":
                node["features"] = contract["celld_features"].copy()
        for package in metadata["packages"]:
            if package["source"] is None:
                package["manifest_path"] = str(root / "Cargo.toml" if package["name"] == c.FIXTURE else root / "runtime/source/crates/celld/Cargo.toml" if package["name"] == "celld" else root.parents[1] / "crates" / package["name"] / "Cargo.toml")
        celld = next(package for package in metadata["packages"] if package["name"] == "celld")
        celld["targets"] = [{"name": "celld", "kind": ["bin"], "crate_types": ["bin"], "src_path": str(root / "runtime/source/crates/celld/main.rs")}]
        if stage in (p.FRONTEND, p.OBSERVATIONS, p.PROCESS, p.MAIN_WORK):
            celld["targets"].append({"name": "celld", "kind": ["lib"], "crate_types": ["lib"], "src_path": str(root / "runtime/source/crates/celld/lib.rs")})
        celld["dependencies"] = [{"name": name, "source": None, "req": "*", "kind": None, "rename": None, "optional": True, "uses_default_features": False, "features": ["qualification" if name == "foks-hosted-transport" else "owned-list"], "target": None, "registry": None, "path": str(root.parents[1] / "crates" / name)} for name in p.FOUNDATION_EDGES]
        for name in contract["celld_foks_edges"]:
            edge(metadata, "celld", name)
        yield metadata, pins, data


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


class FoundationGraphTests(unittest.TestCase):
    def test_all_three_modes_separate_declarations_from_actual_edges(self):
        for mode, contract in p.FOUNDATION_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["profile"], p.FOUNDATION)
                    self.assertEqual(report["foundation"]["declared_foks_edges"], p.FOUNDATION_EDGES)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    self.assertEqual(report["foundation"]["cargo_lock_sha256"], p.file_hash(root / "Cargo.lock"))
                    if not mode.endswith("lifecycle"):
                        self.assertEqual(report["node_target"]["features"], contract["celld_features"])
                    self.assertFalse({"rusqlite", "libsqlite3-sys"} & set(report["foks_subclosure"]))
                    for other in (*p.FOUNDATION_MODES, "identity", "lifecycle", "node-baseline"):
                        if other != mode:
                            with self.subTest(other=other), self.assertRaises(ValueError):
                                c.graph_report(metadata, pins, other, root)

    def test_optional_declarations_reject_each_kind_path_and_feature_escape(self):
        mutations = (
            lambda entry: entry.update(optional=False),
            lambda entry: entry.update(optional=1),
            lambda entry: entry.update(uses_default_features=True),
            lambda entry: entry.update(features=["test-support"]),
            lambda entry: entry.update(source=p.CRATES_IO),
            lambda entry: entry.update(req="=0.5.0"),
            lambda entry: entry.update(registry=p.CRATES_IO),
            lambda entry: entry.update(kind="build"),
            lambda entry: entry.update(kind="dev"),
            lambda entry: entry.update(rename="other"),
            lambda entry: entry.update(target='cfg(target_os = "macos")'),
            lambda entry: entry.update(path="/unreviewed/crate"),
            lambda entry: entry.pop("path"),
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation-default") as (metadata, pins, _):
                for index, mutation in enumerate(mutations):
                    altered = copy.deepcopy(metadata)
                    celld = next(package for package in altered["packages"] if package["name"] == "celld")
                    mutation(celld["dependencies"][0])
                    with self.subTest(index=index), self.assertRaises(ValueError):
                        c.graph_report(altered, pins, "node-foundation-default", root)
                for operation in ("missing", "duplicate", "extra"):
                    altered = copy.deepcopy(metadata)
                    dependencies = next(package for package in altered["packages"] if package["name"] == "celld")["dependencies"]
                    if operation == "missing":
                        dependencies.pop()
                    else:
                        dependencies.append({**dependencies[0], **({"name": "foks-server"} if operation == "extra" else {})})
                    with self.subTest(operation=operation), self.assertRaises(ValueError):
                        c.graph_report(altered, pins, "node-foundation-default", root)

    def test_raw_manifest_rejects_hidden_or_mandatory_alternative(self):
        suffixes = (
            '\n[build-dependencies]\nfoks-metadata-core = "0.4"\n',
            '\n[dev-dependencies]\nalias = {package="foks-metadata-core", version="0.4"}\n',
            '\n[target.\'cfg(unix)\'.dependencies]\nfoks-hosted-transport = "0.4"\n',
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation") as (metadata, pins, _):
                path = root / "runtime/source/crates/celld/Cargo.toml"
                original = path.read_text()
                changes = [original + suffix for suffix in suffixes]
                changes += [original.replace("optional = true", "optional = false", 1), original.replace('"dep:foks-metadata-core"', '"foks-metadata-core"', 1), original.replace("../../../../../../crates/foks-metadata-core", "/other/foks-metadata-core", 1)]
                for index, changed in enumerate(changes):
                    path.write_text(changed)
                    with self.subTest(index=index), self.assertRaises(ValueError):
                        c.graph_report(metadata, pins, "node-foundation", root)
                path.write_text(original)
                outer = root / "Cargo.toml"
                outer.write_text(outer.read_text().replace('"celld/foks-guarded-node-qualification"', '"celld/foks-guarded-qualification"'))
                with self.assertRaisesRegex(ValueError, "outer feature"):
                    c.graph_report(metadata, pins, "node-foundation", root)

    def test_active_edges_cannot_disappear_leak_or_be_targeted(self):
        for mode in p.FOUNDATION_MODES:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    altered = copy.deepcopy(metadata)
                    node = next(row for row in altered["resolve"]["nodes"] if row["id"] == "celld")
                    if mode == "node-foundation":
                        node["deps"] = [entry for entry in node["deps"] if entry["pkg"] != "foks-metadata-core"]
                    else:
                        edge(altered, "celld", "foks-metadata-core")
                    with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, "active celld FOKS"):
                        c.graph_report(altered, pins, mode, root)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation") as (metadata, pins, _):
                for kind in ({"kind": "build", "target": None}, {"kind": None, "target": 'cfg(unix)'}, {"kind": "dev", "target": None}):
                    altered = copy.deepcopy(metadata)
                    node = next(row for row in altered["resolve"]["nodes"] if row["id"] == "celld")
                    next(entry for entry in node["deps"] if entry["pkg"] == "foks-metadata-core")["dep_kinds"] = [kind]
                    with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "active celld FOKS"):
                        c.graph_report(altered, pins, "node-foundation", root)
                for field, value in (("version", "0.5.0"), ("source", p.CRATES_IO), ("manifest_path", "/other/Cargo.toml")):
                    altered = copy.deepcopy(metadata)
                    next(row for row in altered["packages"] if row["name"] == "foks-metadata-core")[field] = value
                    with self.subTest(field=field), self.assertRaises(ValueError):
                        c.graph_report(altered, pins, "node-foundation", root)

    def test_graph_and_build_commands_share_the_exact_selected_stage(self):
        for mode in p.FOUNDATION_MODES:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode), patch.object(c.preparation, "network_command", side_effect=lambda argv: list(map(str, argv))):
                    commands = c.commands(root, Path("/toolchain"), mode)
                    for name, argv in commands.items():
                        self.assertIn("--locked", argv)
                        self.assertIn("--offline", argv)
                        self.assertIn("--no-default-features", argv)
                        target = "--filter-platform" if name == "metadata" else "--target"
                        self.assertEqual(argv[argv.index(target) + 1], c.TARGET)
                        self.assertEqual("--features" in argv, mode != "node-foundation-default")
                    self.assertEqual(commands["tree"].count("--package"), 2)
                    self.assertIn("celld@0.6.2", commands["tree"])

    def test_foundation_source_inventory_includes_new_and_preserved_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation") as (metadata, pins, _):
                for name in c.source_paths(pins, "node-foundation"):
                    path = root / name
                    if not path.exists():
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(("reviewed " + name + "\n").encode())
                (repo / "Cargo.toml").write_bytes(b"reviewed workspace\n")
                baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
                baseline.parent.mkdir(parents=True, exist_ok=True)
                baseline.write_bytes(b"reviewed baseline lock\n")
                for name in p.FOUNDATION_EDGES:
                    path = repo / "crates" / name / "Cargo.toml"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"reviewed FOKS manifest\n")
                report = c.graph_report(metadata, pins, "node-foundation", root)
                reached = {row["id"] for row in report["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    before = c.candidate_inputs(root, "node-foundation")
                    selected = c.selected_inputs(metadata, reached, root, "node-foundation")
                self.assertTrue(all(before.get(path) == checksum for path, checksum in selected.items()))
                for name in (*c.FOUNDATION_SOURCE_PATHS, *c.A2_SOURCE_PATHS):
                    self.assertIn(str(root / name), selected)
                self.assertNotIn("run-node.py", c.source_paths(pins, "node-foundation"))
                for historical in ("identity", "lifecycle", "node-baseline"):
                    with self.assertRaises(ValueError):
                        c.source_paths(pins, historical)

    def test_saved_graph_refuses_a_historical_profile_or_omitted_foundation_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation"):
                evidence = root / "evidence"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.FOUNDATION, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": "identity"}, {"profile": p.A2, "mode": "node-foundation"}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, "node-foundation")
                    environment.assert_not_called()

    def test_expected_lock_is_required_before_any_graph_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation") as (_, _, data):
                (root / "Cargo.lock").write_bytes(b'version = 3\n[[package]]\nname = "celld"\nversion = "0.6.2"\ndependencies = []\n')
                self.assertEqual(p.file_hash(root / "Cargo.lock"), data["profile"]["cargo_lock"]["before"]["sha256"])
                with patch.object(c.preparation, "run_owned") as execute, self.assertRaisesRegex(ValueError, "size/hash"):
                    c.collect(root / "unqualified-graph", root, "node-foundation")
                execute.assert_not_called()

    def test_upstream_exception_is_only_two_exact_selected_celld_edges(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-foundation") as (metadata, _, _):
                packages = {row["id"]: row for row in metadata["packages"]}
                locked = {}
                for name in ("celld", "v8", "rusqlite", "sqlite-vec", "tokio", "libsqlite3-sys"):
                    row = packages[name]
                    locked[(name, row["version"], row["source"])] = {"name": name, "version": row["version"], "source": row["source"]}
                locked[("celld", "0.6.2", None)]["dependencies"] = ["v8", "rusqlite", "sqlite-vec", "tokio"]
                locked[("rusqlite", "0.31.0", p.CRATES_IO)]["dependencies"] = ["libsqlite3-sys"]
                reached = set(packages)
                with patch.object(c.preparation, "lock_inventory", return_value=locked):
                    edges = c.preparation.upstream_edges(metadata, reached, root, "node-foundation")
                    self.assertTrue({("celld", name) for name in p.FOUNDATION_EDGES} <= {(entry["from"], entry["to"]) for entry in edges})
                    for mode in ("identity", "node-foundation-default", "node-foundation-lifecycle"):
                        with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, "replaced or extended"):
                            c.preparation.upstream_edges(metadata, reached, root, mode)
                    altered = copy.deepcopy(metadata)
                    edge(altered, "tokio", "foks-metadata-core")
                    with self.assertRaisesRegex(ValueError, "replaced or extended"):
                        c.preparation.upstream_edges(altered, reached, root, "node-foundation")


class LifetimesGraphTests(unittest.TestCase):
    def test_all_three_lifetimes_modes_bind_the_same_declared_edges_and_selected_activity(self):
        for mode, contract in p.LIFETIMES_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, data):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["profile"], p.LIFETIMES)
                    self.assertEqual(report["foundation"]["accepted_b1_commit"], p.LIFETIMES_BASE)
                    self.assertEqual(report["foundation"]["accepted_b1_images"], data["accepted_images"])
                    self.assertEqual(report["foundation"]["declared_foks_edges"], p.FOUNDATION_EDGES)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    self.assertEqual(report["foundation"]["cargo_lock_sha256"], p.file_hash(root / "Cargo.lock"))
                    self.assertFalse({"rusqlite", "libsqlite3-sys"} & set(report["foks_subclosure"]))
                    if mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                    else:
                        self.assertEqual(report["node_target"]["features"], contract["celld_features"])
                    for other in ("identity", "lifecycle", "node-baseline", *p.FOUNDATION_MODES, *p.LIFETIMES_MODES):
                        if other != mode:
                            with self.subTest(other=other), self.assertRaises(ValueError):
                                c.graph_report(metadata, pins, other, root)

    def test_lifetimes_modes_do_not_relax_declarations_active_edges_or_binary_target(self):
        for mode in p.LIFETIMES_MODES:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    def celld(data):
                        return next(package for package in data["packages"] if package["name"] == "celld")
                    mutations = (
                        lambda data: celld(data)["dependencies"][0].update(optional=False),
                        lambda data: celld(data)["dependencies"][0].update(optional=1),
                        lambda data: celld(data)["dependencies"][0].update(features=["test-support"]),
                        lambda data: celld(data)["dependencies"][0].update(kind="build"),
                        lambda data: celld(data)["dependencies"][0].update(path="/unreviewed"),
                        lambda data: next(row for row in data["resolve"]["nodes"] if row["id"] == "foks-metadata-core")["features"].append("native"),
                        lambda data: edge(data, "foks-metadata-core", "libsqlite3-sys"),
                    )
                    for index, mutate in enumerate(mutations):
                        altered = copy.deepcopy(metadata)
                        mutate(altered)
                        with self.subTest(mode=mode, index=index), self.assertRaises(ValueError):
                            c.graph_report(altered, pins, mode, root)
                    altered = copy.deepcopy(metadata)
                    if mode == "node-lifetimes":
                        deps = next(row for row in altered["resolve"]["nodes"] if row["id"] == "celld")["deps"]
                        deps[:] = [dep for dep in deps if dep["pkg"] != "foks-metadata-core"]
                    else:
                        edge(altered, "celld", "foks-metadata-core")
                    with self.assertRaisesRegex(ValueError, "active celld FOKS"):
                        c.graph_report(altered, pins, mode, root)
                    if not mode.endswith("lifecycle"):
                        altered = copy.deepcopy(metadata)
                        celld(altered)["targets"][0]["src_path"] = str(root / "runtime/source/crates/celld/other.rs")
                        with self.assertRaisesRegex(ValueError, "binary target"):
                            c.graph_report(altered, pins, mode, root)

    def test_commands_select_the_actual_locked_mode_and_exact_target(self):
        for mode in p.LIFETIMES_MODES:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode), patch.object(c.preparation, "network_command", side_effect=lambda argv: list(map(str, argv))):
                    commands = c.commands(root, Path("/toolchain"), mode)
                    for name, argv in commands.items():
                        self.assertIn("--locked", argv)
                        self.assertIn("--offline", argv)
                        self.assertIn("--no-default-features", argv)
                        self.assertEqual(argv[argv.index("--manifest-path") + 1], str(root / "Cargo.toml"))
                        target = "--filter-platform" if name == "metadata" else "--target"
                        self.assertEqual(argv[argv.index(target) + 1], c.TARGET)
                        self.assertEqual("--features" in argv, mode != "node-lifetimes-default")
                        if "--features" in argv:
                            self.assertEqual(argv[argv.index("--features") + 1], "foks-celld-guarded/" + p.LIFETIMES_MODES[mode]["root_features"][0])
                    self.assertEqual(commands["tree"].count("--package"), 2)
                    self.assertIn("celld@0.6.2", commands["tree"])

    def test_source_inputs_include_the_unchanged_prefix_and_all_accepted_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, "node-lifetimes") as (metadata, pins, _):
                for name in c.source_paths(pins, "node-lifetimes"):
                    path = root / name
                    if not path.exists():
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(("reviewed " + name + "\n").encode())
                (repo / "Cargo.toml").write_bytes(b"reviewed workspace\n")
                baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
                baseline.parent.mkdir(parents=True, exist_ok=True)
                baseline.write_bytes(b"reviewed baseline lock\n")
                for name in p.FOUNDATION_EDGES:
                    path = repo / "crates" / name / "Cargo.toml"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"reviewed FOKS manifest\n")
                report = c.graph_report(metadata, pins, "node-lifetimes", root)
                reached = {row["id"] for row in report["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    before = c.candidate_inputs(root, "node-lifetimes")
                    selected = c.selected_inputs(metadata, reached, root, "node-lifetimes")
                self.assertTrue(all(before.get(path) == checksum for path, checksum in selected.items()))
                for name in (*c.A2_SOURCE_PATHS, *c.FOUNDATION_SOURCE_PATHS, *c.LIFETIMES_SOURCE_PATHS):
                    self.assertIn(str(root / name), selected)
                self.assertNotIn("run-node.py", c.source_paths(pins, "node-lifetimes"))
                for old in ("identity", "lifecycle", "node-baseline", *p.FOUNDATION_MODES):
                    with self.subTest(old=old), self.assertRaises(ValueError):
                        c.source_paths(pins, old)

    def test_missing_mode_or_relabelled_graph_refuses_before_tool_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-lifetimes"):
                evidence = root / "evidence"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.LIFETIMES, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": "node-foundation"}, {"profile": p.FOUNDATION, "mode": "node-lifetimes"}, {"target": "other", "mode": "node-lifetimes"}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, "node-lifetimes")
                    environment.assert_not_called()

    def test_stale_lock_or_changed_preserved_prefix_refuses_before_cargo(self):
        for changed in ("Cargo.lock", p.LIFETIMES_PROFILE, p.LIFETIMES_SERIES, p.LIFETIMES_MANIFEST):
            with self.subTest(path=changed), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, "node-lifetimes"):
                    path = root / changed
                    path.write_bytes(path.read_bytes() + b"\n")
                    with patch.object(c.preparation, "run_owned") as execute, self.assertRaises(ValueError):
                        c.collect(root / "refused-graph", root, "node-lifetimes")
                    execute.assert_not_called()

    def test_lifetimes_upstream_edge_exception_still_has_only_two_exact_local_edges(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, "node-lifetimes") as (metadata, _, _):
                packages = {row["id"]: row for row in metadata["packages"]}
                locked = {(row["name"], row["version"], row["source"]): {"name": row["name"], "version": row["version"], "source": row["source"]} for row in packages.values() if row["name"] in ("celld", "v8", "rusqlite", "sqlite-vec", "tokio", "libsqlite3-sys")}
                locked[("celld", "0.6.2", None)]["dependencies"] = ["v8", "rusqlite", "sqlite-vec", "tokio"]
                locked[("rusqlite", "0.31.0", p.CRATES_IO)]["dependencies"] = ["libsqlite3-sys"]
                with patch.object(c.preparation, "lock_inventory", return_value=locked):
                    result = c.preparation.upstream_edges(metadata, set(packages), root, "node-lifetimes")
                    self.assertTrue({("celld", name) for name in p.FOUNDATION_EDGES} <= {(row["from"], row["to"]) for row in result})
                    for mode in ("node-lifetimes-default", "node-lifetimes-lifecycle"):
                        with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, "replaced or extended"):
                            c.preparation.upstream_edges(metadata, set(packages), root, mode)
                    altered = copy.deepcopy(metadata)
                    edge(altered, "tokio", "foks-metadata-core")
                    with self.assertRaisesRegex(ValueError, "replaced or extended"):
                        c.preparation.upstream_edges(altered, set(packages), root, "node-lifetimes")


class FrontendGraphTests(unittest.TestCase):
    def test_three_modes_keep_exact_optional_edges_and_bind_both_targets(self):
        for mode, contract in p.FRONTEND_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, data):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["profile"], p.FRONTEND)
                    self.assertEqual(report["foundation"]["accepted_b2_commit"], p.FRONTEND_BASE)
                    self.assertEqual(report["foundation"]["accepted_b2_images"], data["accepted_b2_images"])
                    self.assertEqual(report["foundation"]["declared_foks_edges"], p.FOUNDATION_EDGES)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    self.assertFalse({"rusqlite", "libsqlite3-sys"} & set(report["foks_subclosure"]))
                    if mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                        self.assertNotIn("node_test_target", report)
                    else:
                        self.assertEqual(report["node_target"]["kind"], ["bin"])
                        self.assertEqual(report["node_test_target"]["kind"], ["lib"])
                        self.assertEqual(report["node_test_target"]["features"], contract["celld_features"])
                    for old in (*p.FOUNDATION_MODES, *p.LIFETIMES_MODES, "identity", "lifecycle", "node-baseline"):
                        with self.subTest(old=old), self.assertRaises(ValueError):
                            c.graph_report(metadata, pins, old, root)

    def test_library_artifact_target_and_native_subclosure_cannot_be_relabelled(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.FRONTEND) as (metadata, pins, _):
                def celld(value):
                    return next(row for row in value["packages"] if row["name"] == "celld")
                mutations = (
                    lambda x: celld(x)["targets"].pop(),
                    lambda x: celld(x)["targets"].append(copy.deepcopy(celld(x)["targets"][1])),
                    lambda x: celld(x)["targets"][1].update(name="other"),
                    lambda x: celld(x)["targets"][1].update(src_path="/other/lib.rs"),
                    lambda x: celld(x)["targets"][1].update(crate_types=["cdylib"]),
                    lambda x: celld(x)["targets"][1].update({"required-features": ["other"]}),
                    lambda x: celld(x)["dependencies"][0].update(optional=False),
                    lambda x: celld(x)["dependencies"][0].update(features=["test-support"]),
                    lambda x: edge(x, "foks-metadata-core", "libsqlite3-sys"),
                    lambda x: next(row for row in x["resolve"]["nodes"] if row["id"] == "celld")["features"].append("unreviewed"),
                )
                for index, mutate in enumerate(mutations):
                    changed = copy.deepcopy(metadata)
                    mutate(changed)
                    with self.subTest(index=index), self.assertRaises(ValueError):
                        c.graph_report(changed, pins, p.FRONTEND, root)
        for mode in (p.FRONTEND + "-default", p.FRONTEND + "-lifecycle"):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    edge(metadata, "celld", "foks-metadata-core")
                    with self.assertRaises(ValueError):
                        c.graph_report(metadata, pins, mode, root)

    def test_frontend_graph_commands_keep_locked_offline_selected_workspace(self):
        for mode, contract in p.FRONTEND_MODES.items():
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode), patch.object(c.preparation, "network_command", side_effect=lambda argv: ["DENY-ALL", *map(str, argv)]):
                    commands = c.commands(root, Path("/toolchain"), mode)
                    for name, argv in commands.items():
                        self.assertEqual(argv[0], "DENY-ALL")
                        self.assertIn("--locked", argv)
                        self.assertIn("--offline", argv)
                        self.assertEqual(argv[argv.index("--manifest-path") + 1], str(root / "Cargo.toml"))
                        flag = "--filter-platform" if name == "metadata" else "--target"
                        self.assertEqual(argv[argv.index(flag) + 1], p.TARGET)
                        self.assertEqual("--features" in argv, bool(contract["root_features"]))
                        if contract["root_features"]:
                            self.assertEqual(argv[argv.index("--features") + 1], "foks-celld-guarded/" + contract["root_features"][0])
                    self.assertEqual(commands["tree"].count("--package"), 2)

    def test_inputs_include_prefix_assets_and_current_foks_source(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, p.FRONTEND) as (metadata, pins, _):
                for name in c.source_paths(pins, p.FRONTEND):
                    path = root / name
                    if not path.exists():
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(("reviewed " + name + "\n").encode())
                workspace = '[workspace]\nmembers = ' + json.dumps(["crates/" + name for name in p.FOUNDATION_EDGES] + ["apps/desktop"]) + '\n'
                (repo / "Cargo.toml").write_text(workspace)
                (repo / "Cargo.lock").write_bytes(b"root workspace lock\n")
                (repo / "apps/desktop").mkdir(parents=True)
                (repo / "apps/desktop/Cargo.toml").write_bytes(b"unselected member manifest\n")
                baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
                baseline.parent.mkdir(parents=True, exist_ok=True)
                baseline.write_bytes(b"reviewed lock\n")
                for name in p.FOUNDATION_EDGES:
                    path = repo / "crates" / name / "src/lib.rs"
                    path.parent.mkdir(parents=True)
                    path.write_bytes(b"reviewed actual FOKS source\n")
                    (path.parent.parent / "Cargo.toml").write_bytes(b"reviewed manifest\n")
                report = c.graph_report(metadata, pins, p.FRONTEND, root)
                reached = {row["id"] for row in report["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    before = c.candidate_inputs(root, p.FRONTEND)
                    selected = c.selected_inputs(metadata, reached, root, p.FRONTEND)
                self.assertTrue(all(before.get(path) == checksum for path, checksum in selected.items()))
                for name in (*c.A2_SOURCE_PATHS, *c.FOUNDATION_SOURCE_PATHS, *c.LIFETIMES_SOURCE_PATHS, *c.FRONTEND_SOURCE_PATHS):
                    self.assertIn(str(root / name), selected)
                self.assertIn(str(repo / "crates/foks-hosted-transport/src/lib.rs"), selected)
                self.assertIn(str(repo / "Cargo.lock"), selected)
                self.assertIn(str(repo / "apps/desktop/Cargo.toml"), selected)
                for members in ([], ["../outside"], ["crates/*"], ["crates/missing"], ["crates/a"] * 2, [1]):
                    (repo / "Cargo.toml").write_text('[workspace]\nmembers = ' + json.dumps(members) + '\n')
                    with self.subTest(members=members), self.assertRaises(ValueError):
                        c.frontend_transport_inputs(root, p.FRONTEND)
                (repo / "Cargo.toml").write_text(workspace)
                (repo / "apps/desktop/Cargo.toml").unlink()
                (repo / "apps/desktop/Cargo.toml").symlink_to(repo / "Cargo.lock")
                with self.assertRaises(ValueError):
                    c.frontend_transport_inputs(root, p.FRONTEND)
                self.assertNotIn("run-node.py", c.source_paths(pins, p.FRONTEND))

    def test_changed_predecessor_or_missing_mode_refuses_before_any_command(self):
        for changed in ("Cargo.lock", p.FRONTEND_PROFILE, p.FRONTEND_SERIES, p.FRONTEND_MANIFEST):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, p.FRONTEND):
                    path = root / changed
                    path.write_bytes(path.read_bytes() + b"\n")
                    with patch.object(c.preparation, "run_owned") as execute, self.assertRaises(ValueError):
                        c.collect(root / "refused", root, p.FRONTEND)
                    execute.assert_not_called()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.FRONTEND):
                evidence = root / "evidence"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.FRONTEND, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": p.LIFETIMES}, {"mode": p.FRONTEND, "profile": p.LIFETIMES}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, p.FRONTEND)
                    environment.assert_not_called()


class ObservationsGraphTests(unittest.TestCase):
    def test_three_modes_bind_b3_ancestry_and_exact_optional_edges(self):
        for mode, contract in p.OBSERVATIONS_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, data):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["profile"], p.OBSERVATIONS)
                    self.assertEqual(report["foundation"]["accepted_b3_commit"], p.OBSERVATIONS_BASE)
                    self.assertEqual(report["foundation"]["accepted_b3_images"], data["accepted_b3_images"])
                    self.assertEqual(report["foundation"]["accepted_b2_images"], data["accepted_b2_images"])
                    self.assertEqual(report["foundation"]["declared_foks_edges"], p.FOUNDATION_EDGES)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    self.assertFalse({"rusqlite", "libsqlite3-sys"} & set(report["foks_subclosure"]))
                    if mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                        self.assertNotIn("node_test_target", report)
                    else:
                        self.assertEqual(report["node_target"]["kind"], ["bin"])
                        self.assertEqual(report["node_test_target"]["kind"], ["lib"])
                        self.assertEqual(report["node_test_target"]["features"], contract["celld_features"])
                    for old in (*p.FOUNDATION_MODES, *p.LIFETIMES_MODES, *p.FRONTEND_MODES, "identity", "node-baseline", "node-process"):
                        with self.subTest(old=old), self.assertRaises(ValueError):
                            c.graph_report(metadata, pins, old, root)

    def test_targets_and_foks_subclosure_remain_closed_in_observations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.OBSERVATIONS) as (metadata, pins, _):
                def celld(value):
                    return next(row for row in value["packages"] if row["name"] == "celld")
                mutations = (
                    lambda x: celld(x)["targets"].pop(),
                    lambda x: celld(x)["targets"].append(copy.deepcopy(celld(x)["targets"][1])),
                    lambda x: celld(x)["targets"][1].update(src_path="/other/lib.rs"),
                    lambda x: celld(x)["dependencies"][0].update(optional=False),
                    lambda x: celld(x)["dependencies"][0].update(features=["test-support"]),
                    lambda x: edge(x, "foks-metadata-core", "libsqlite3-sys"),
                    lambda x: next(row for row in x["resolve"]["nodes"] if row["id"] == "celld")["features"].append("unreviewed"),
                )
                for index, mutate in enumerate(mutations):
                    changed = copy.deepcopy(metadata)
                    mutate(changed)
                    with self.subTest(index=index), self.assertRaises(ValueError):
                        c.graph_report(changed, pins, p.OBSERVATIONS, root)
        for mode in (p.OBSERVATIONS + "-default", p.OBSERVATIONS + "-lifecycle"):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    edge(metadata, "celld", "foks-metadata-core")
                    with self.assertRaises(ValueError):
                        c.graph_report(metadata, pins, mode, root)

    def test_commands_preserve_selected_features_and_isolated_a0_target(self):
        for mode, contract in p.OBSERVATIONS_MODES.items():
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode), patch.object(c.preparation, "network_command", side_effect=lambda argv: ["DENY-ALL", *map(str, argv)]):
                    for name, argv in c.commands(root, Path("/toolchain"), mode).items():
                        self.assertEqual(argv[0], "DENY-ALL")
                        self.assertIn("--locked", argv)
                        self.assertIn("--offline", argv)
                        self.assertEqual(argv[argv.index("--manifest-path") + 1], str(root / "Cargo.toml"))
                        flag = "--filter-platform" if name == "metadata" else "--target"
                        self.assertEqual(argv[argv.index(flag) + 1], p.TARGET)
                        self.assertEqual("--features" in argv, bool(contract["root_features"]))
                        if contract["root_features"]:
                            self.assertEqual(argv[argv.index("--features") + 1], "foks-celld-guarded/" + contract["root_features"][0])
                    for argv in c.preparation.frontend_transport_commands(root, Path("/toolchain"), "deadline").values():
                        self.assertEqual(argv[argv.index("--manifest-path") + 1], str(root.parents[1] / "Cargo.toml"))
                        self.assertEqual(argv[argv.index("--target-dir") + 1], str(root / "runtime/target/transport-regression"))
                        self.assertEqual(argv.count("--package"), 1)
                        self.assertEqual(argv[argv.index("--package") + 1], "foks-hosted-transport")

    def test_inputs_bind_all_prefix_images_and_workspace_resolver_without_controller(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, p.OBSERVATIONS) as (metadata, pins, _):
                for name in c.source_paths(pins, p.OBSERVATIONS):
                    path = root / name
                    if not path.exists():
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(("reviewed " + name + "\n").encode())
                (repo / "Cargo.toml").write_text('[workspace]\nmembers = ' + json.dumps(["crates/" + name for name in p.FOUNDATION_EDGES] + ["apps/desktop"]) + '\n')
                (repo / "Cargo.lock").write_bytes(b"root workspace lock\n")
                (repo / "apps/desktop").mkdir(parents=True)
                (repo / "apps/desktop/Cargo.toml").write_bytes(b"unselected resolver member\n")
                baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
                baseline.parent.mkdir(parents=True, exist_ok=True)
                baseline.write_bytes(b"reviewed lock\n")
                for name in p.FOUNDATION_EDGES:
                    path = repo / "crates" / name / "src/lib.rs"
                    path.parent.mkdir(parents=True)
                    path.write_bytes(b"reviewed actual FOKS source\n")
                    (path.parent.parent / "Cargo.toml").write_bytes(b"reviewed manifest\n")
                report = c.graph_report(metadata, pins, p.OBSERVATIONS, root)
                reached = {row["id"] for row in report["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    before = c.candidate_inputs(root, p.OBSERVATIONS)
                    selected = c.selected_inputs(metadata, reached, root, p.OBSERVATIONS)
                self.assertTrue(all(before.get(path) == checksum for path, checksum in selected.items()))
                for name in (*c.A2_SOURCE_PATHS, *c.FOUNDATION_SOURCE_PATHS, *c.LIFETIMES_SOURCE_PATHS, *c.FRONTEND_SOURCE_PATHS, *c.OBSERVATIONS_SOURCE_PATHS):
                    self.assertIn(str(root / name), selected)
                self.assertIn(str(repo / "Cargo.lock"), selected)
                self.assertIn(str(repo / "apps/desktop/Cargo.toml"), selected)
                self.assertIn(str(repo / "crates/foks-hosted-transport/src/lib.rs"), selected)
                self.assertNotIn("run-node.py", c.source_paths(pins, p.OBSERVATIONS))
                self.assertNotIn("run-guarded-node.py", c.source_paths(pins, p.OBSERVATIONS))
                for mode in (p.OBSERVATIONS + "-default", p.OBSERVATIONS + "-lifecycle"):
                    self.assertEqual(c.frontend_transport_inputs(root, mode), set())
                (repo / "apps/desktop/Cargo.toml").unlink()
                with self.assertRaises(ValueError):
                    c.frontend_transport_inputs(root, p.OBSERVATIONS)

    def test_changed_b3_image_or_relabelled_mode_refuses_before_any_command(self):
        for changed in ("Cargo.lock", p.OBSERVATIONS_PROFILE, p.OBSERVATIONS_SERIES, p.OBSERVATIONS_MANIFEST):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, p.OBSERVATIONS):
                    path = root / changed
                    path.write_bytes(path.read_bytes() + b"\n")
                    with patch.object(c.preparation, "run_owned") as execute, self.assertRaises(ValueError):
                        c.collect(root / "refused", root, p.OBSERVATIONS)
                    execute.assert_not_called()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.OBSERVATIONS):
                evidence = root / "evidence"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.OBSERVATIONS, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": p.FRONTEND}, {"mode": p.OBSERVATIONS, "profile": p.FRONTEND}, {"mode": "node-process"}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, p.OBSERVATIONS)
                    environment.assert_not_called()


class ProcessGraphTests(unittest.TestCase):
    def test_process_modes_have_exact_feature_edges_and_targets(self):
        for mode, contract in p.PROCESS_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["profile"], p.PROCESS)
                    self.assertEqual(report["foundation"]["declared_foks_edges"], p.FOUNDATION_EDGES)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    self.assertFalse({"rusqlite", "libsqlite3-sys"} & set(report["foks_subclosure"]))
                    if mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                        self.assertNotIn("node_test_target", report)
                    else:
                        self.assertEqual(report["node_target"]["kind"], ["bin"])
                        self.assertEqual(report["node_test_target"]["kind"], ["lib"])
                        self.assertEqual(report["node_test_target"]["features"], contract["celld_features"])
                    for old in (*p.FOUNDATION_MODES, *p.LIFETIMES_MODES, *p.FRONTEND_MODES, *p.OBSERVATIONS_MODES, "identity", "node-baseline", "node-process-unknown"):
                        with self.subTest(old=old), self.assertRaises(ValueError):
                            c.graph_report(metadata, pins, old, root)
                    def celld(value):
                        return next(row for row in value["packages"] if row["name"] == "celld")
                    mutations = [
                        lambda x: celld(x)["dependencies"][0].update(optional=False),
                        lambda x: celld(x)["dependencies"][0].update(features=["test-support"]),
                        lambda x: next(row for row in x["resolve"]["nodes"] if row["id"] == "celld")["features"].append("unreviewed"),
                    ]
                    if not mode.endswith("lifecycle"):
                        mutations.extend((
                            lambda x: celld(x)["targets"].pop(),
                            lambda x: celld(x)["targets"].append(copy.deepcopy(celld(x)["targets"][0])),
                            lambda x: celld(x)["targets"][1].update(src_path="/other/lib.rs"),
                        ))
                    if mode == p.PROCESS:
                        mutations.append(lambda x: edge(x, "foks-metadata-core", "libsqlite3-sys"))
                    for index, mutate in enumerate(mutations):
                        changed = copy.deepcopy(metadata)
                        mutate(changed)
                        with self.subTest(index=index), self.assertRaises(ValueError):
                            c.graph_report(changed, pins, mode, root)

    def test_process_graph_report_binds_all_accepted_ancestors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.PROCESS) as (metadata, pins, data):
                report = c.graph_report(metadata, pins, p.PROCESS, root)["foundation"]
                self.assertEqual(report["accepted_a3a_commit"], p.FOUNDATION_BASE)
                self.assertEqual(report["accepted_b1_commit"], p.LIFETIMES_BASE)
                self.assertEqual(report["accepted_b2_commit"], p.FRONTEND_BASE)
                self.assertEqual(report["accepted_b3_commit"], p.OBSERVATIONS_BASE)
                self.assertEqual(report["accepted_b4_p1_commit"], p.PROCESS_BASE)
                for key, fixture_key in (("accepted_b1_images", "accepted_images"), ("accepted_b2_images", "accepted_b2_images"), ("accepted_b3_images", "accepted_b3_images"), ("accepted_b4_p1_images", "accepted_p1_images")):
                    self.assertEqual(report[key], data[fixture_key])
                self.assertEqual(report["profile_sha256"], p.file_hash(root / p.FOUNDATION_PROFILE))
                self.assertEqual(report["series_sha256"], pins["foundation"]["node_series"]["sha256"])

    @staticmethod
    def input_fixture(repo, root, pins):
        for name in c.source_paths(pins, p.PROCESS):
            path = root / name
            if not path.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(("reviewed " + name + "\n").encode())
        (repo / "Cargo.toml").write_text('[workspace]\nmembers = ' + json.dumps(["crates/" + name for name in p.FOUNDATION_EDGES] + ["apps/desktop"]) + '\n')
        (repo / "Cargo.lock").write_bytes(b"root workspace lock\n")
        (repo / "apps/desktop").mkdir(parents=True)
        (repo / "apps/desktop/Cargo.toml").write_bytes(b"unselected resolver member\n")
        baseline = repo / pins["foks_additional_versions"]["baseline_lock"]
        baseline.parent.mkdir(parents=True, exist_ok=True)
        baseline.write_bytes(b"reviewed lock\n")
        for name in p.FOUNDATION_EDGES:
            path = repo / "crates" / name / "src/lib.rs"
            path.parent.mkdir(parents=True)
            path.write_bytes(b"reviewed actual FOKS source\n")
            (path.parent.parent / "Cargo.toml").write_bytes(b"reviewed manifest\n")

    def test_process_bindings_include_sixth_patch_and_every_prepared_source(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, p.PROCESS) as (metadata, pins, data):
                self.input_fixture(repo, root, pins)
                report = c.graph_report(metadata, pins, p.PROCESS, root)
                reached = {row["id"] for row in report["packages"]}
                with patch.object(c.preparation, "config_inputs", return_value={}):
                    before = c.candidate_inputs(root, p.PROCESS)
                    selected = c.selected_inputs(metadata, reached, root, p.PROCESS)
                self.assertTrue(all(before.get(path) == checksum for path, checksum in selected.items()))
                for name in (*c.A2_SOURCE_PATHS, *c.FOUNDATION_SOURCE_PATHS, *c.LIFETIMES_SOURCE_PATHS, *c.FRONTEND_SOURCE_PATHS, *c.OBSERVATIONS_SOURCE_PATHS, *c.PROCESS_SOURCE_PATHS):
                    self.assertIn(str(root / name), selected)
                for row in data["rows"]:
                    self.assertIn(str(root / "runtime/source" / row["path"]), before)
                    self.assertEqual(before[str(root / "runtime/source" / row["path"])], row["sha256"])
                self.assertIn(str(root / "runtime/source/crates/celld/guarded_node/qualification.rs"), before)
                self.assertNotIn("run-node.py", c.source_paths(pins, p.PROCESS))
                self.assertNotIn("run-guarded-node.py", c.source_paths(pins, p.PROCESS))
                for name in (p.PROCESS_PATCH, p.PROCESS_PROFILE, p.PROCESS_MANIFEST):
                    path = root / name
                    original = path.read_bytes()
                    path.write_bytes(original + b"\n")
                    with patch.object(c.preparation, "config_inputs", return_value={}):
                        if name == p.PROCESS_PATCH:
                            with self.assertRaises(ValueError):
                                c.candidate_inputs(root, p.PROCESS)
                        else:
                            with self.assertRaises(ValueError):
                                c.selected_inputs(metadata, reached, root, p.PROCESS)
                    path.write_bytes(original)

    def test_process_default_and_lifecycle_reject_accidental_enabled_edges(self):
        for mode in (p.PROCESS + "-default", p.PROCESS + "-lifecycle"):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    edge(metadata, "celld", "foks-metadata-core")
                    with self.assertRaises(ValueError):
                        c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(c.frontend_transport_inputs(root, mode), set())
                    with patch.object(c.preparation, "network_command", side_effect=lambda argv: ["DENY-ALL", *map(str, argv)]):
                        for name, argv in c.commands(root, Path("/toolchain"), mode).items():
                            self.assertEqual(argv[0], "DENY-ALL")
                            self.assertIn("--locked", argv)
                            self.assertIn("--offline", argv)
                            self.assertEqual("--features" in argv, mode.endswith("lifecycle"))

    def test_process_graph_rejects_cross_stage_and_changed_predecessor_inputs(self):
        for changed in ("Cargo.lock", p.PROCESS_PROFILE, p.PROCESS_SERIES, p.PROCESS_MANIFEST, p.OBSERVATIONS_PROFILE):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, p.PROCESS):
                    path = root / changed
                    path.write_bytes(path.read_bytes() + b"\n")
                    with patch.object(c.preparation, "run_owned") as execute, self.assertRaises(ValueError):
                        c.collect(root / "refused", root, p.PROCESS)
                    execute.assert_not_called()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.PROCESS):
                evidence = root / "evidence"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.PROCESS, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": p.OBSERVATIONS}, {"mode": p.PROCESS, "profile": p.OBSERVATIONS}, {"mode": "node-process-unknown"}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, p.PROCESS)
                    environment.assert_not_called()

    def test_process_transport_regression_keeps_exact_root_workspace_bindings(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = repo / "tools/fixture"
            with foundation_graph_fixture(root, p.PROCESS) as (metadata, pins, _):
                self.input_fixture(repo, root, pins)
                expected = {repo / "Cargo.lock", repo / "apps/desktop/Cargo.toml", *(repo / "crates" / name / "Cargo.toml" for name in p.FOUNDATION_EDGES)}
                self.assertEqual(c.frontend_transport_inputs(root, p.PROCESS), expected)
                for argv in c.preparation.frontend_transport_commands(root, Path("/toolchain"), "deadline").values():
                    self.assertEqual(argv[argv.index("--manifest-path") + 1], str(repo / "Cargo.toml"))
                    self.assertEqual(argv[argv.index("--target-dir") + 1], str(root / "runtime/target/transport-regression"))
                    self.assertEqual(argv.count("--package"), 1)
                    self.assertEqual(argv[argv.index("--package") + 1], "foks-hosted-transport")
                (repo / "apps/desktop/Cargo.toml").unlink()
                with self.assertRaises(ValueError):
                    c.frontend_transport_inputs(root, p.PROCESS)


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


class MainWorkGraphTests(unittest.TestCase):
    def test_main_work_modes_bind_binary_or_library_target_with_exact_features(self):
        for mode, contract in p.MAIN_WORK_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode) as (metadata, pins, _):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    if mode == p.MAIN_WORK_BIN_TEST:
                        self.assertEqual(report["node_bin_test_target"]["kind"], ["bin"])
                        self.assertEqual(report["node_bin_test_target"]["src_path"], str(root / "runtime/source/crates/celld/main.rs"))
                        self.assertNotIn("node_test_target", report)
                    elif mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                    else:
                        self.assertEqual(report["node_target"]["kind"], ["bin"])
                        self.assertEqual(report["node_test_target"]["kind"], ["lib"])
                    for old in p.PROCESS_MODES:
                        with self.subTest(old=old), self.assertRaises(ValueError):
                            c.graph_report(metadata, pins, old, root)
                    changed = copy.deepcopy(metadata)
                    next(row for row in changed["resolve"]["nodes"] if row["id"] == "celld")["features"].append("unreviewed")
                    with self.assertRaises(ValueError):
                        c.graph_report(changed, pins, mode, root)

    def test_main_work_graph_binds_exact_p2_and_every_prior_ancestor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.MAIN_WORK) as (metadata, pins, _):
                report = c.graph_report(metadata, pins, p.MAIN_WORK, root)["foundation"]
                self.assertEqual(report["accepted_b4_p2_commit"], p.MAIN_WORK_BASE)
                self.assertEqual(report["accepted_b4_p1_commit"], p.PROCESS_BASE)
                self.assertEqual(report["accepted_b4_p2_images"], {name: pins["foundation"]["base"][name] for name in p.MAIN_WORK_BASE_IMAGES})
                paths = set(c.source_paths(pins, p.MAIN_WORK))
                self.assertTrue(set(c.MAIN_WORK_SOURCE_PATHS).issubset(paths))
                self.assertTrue(set(c.PROCESS_SOURCE_PATHS).issubset(paths))
                self.assertIn(p.PROCESS_PATCH, paths)
                self.assertIn(p.MAIN_WORK_PATCH, paths)
                self.assertNotEqual(p.MAIN_WORK_PROFILE, p.FOUNDATION_PROFILE)

    def test_main_work_rejects_relabelled_evidence_before_any_tool(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            evidence = root / "graph"
            evidence.mkdir(parents=True)
            with foundation_graph_fixture(root, p.MAIN_WORK):
                record = {"format": 1, "target": p.TARGET, "profile": p.MAIN_WORK, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                for changed in ({}, {"mode": p.PROCESS}, {"mode": p.MAIN_WORK, "profile": p.PROCESS}, {"mode": p.MAIN_WORK_BIN_TEST}):
                    (evidence / "evidence.json").write_text(json.dumps({**record, **changed}))
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaises(ValueError):
                        c.verify(evidence, root, p.MAIN_WORK)
                    environment.assert_not_called()

    def test_main_work_transport_binding_applies_only_to_123_library_lane(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            root.mkdir(parents=True)
            repo = root.parents[1]
            (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/foks-hosted-transport"]\n')
            (repo / "Cargo.lock").write_text("lock\n")
            member = repo / "crates/foks-hosted-transport"
            member.mkdir(parents=True)
            (member / "Cargo.toml").write_text('[package]\nname="foks-hosted-transport"\n')
            expected = {repo / "Cargo.lock", member / "Cargo.toml"}
            self.assertEqual(c.frontend_transport_inputs(root, p.MAIN_WORK), expected)
            for mode in (p.MAIN_WORK_BIN_TEST, p.MAIN_WORK + "-default", p.MAIN_WORK + "-lifecycle"):
                self.assertEqual(c.frontend_transport_inputs(root, mode), set())
            (member / "Cargo.toml").unlink()
            with self.assertRaises(ValueError):
                c.frontend_transport_inputs(root, p.MAIN_WORK)




class QualifierContentionRevisionGraphTests(unittest.TestCase):
    def test_revision_modes_keep_actual_targets_and_features(self):
        for mode, contract in p.MAIN_WORK_MODES.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "tools/fixture"
                with foundation_graph_fixture(root, mode, qualifier_revision=True) as (metadata, pins, _):
                    report = c.graph_report(metadata, pins, mode, root)
                    self.assertEqual(report["foundation"]["celld_foks_edges"], contract["celld_foks_edges"])
                    if mode == p.MAIN_WORK_BIN_TEST:
                        self.assertEqual(report["node_bin_test_target"]["kind"], ["bin"])
                        self.assertNotIn("node_test_target", report)
                    elif mode.endswith("lifecycle"):
                        self.assertNotIn("node_target", report)
                    else:
                        self.assertEqual(report["node_target"]["src_path"], str(root / "runtime/source/crates/celld/main.rs"))
                        self.assertEqual(report["node_test_target"]["kind"], ["lib"])
                    changed = copy.deepcopy(metadata)
                    next(row for row in changed["resolve"]["nodes"] if row["id"] == "celld")["features"].append("unreviewed")
                    with self.assertRaises(ValueError):
                        c.graph_report(changed, pins, mode, root)

    def test_revision_graph_binds_main_work_and_all_ancestors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.MAIN_WORK, qualifier_revision=True) as (metadata, pins, _):
                report = c.graph_report(metadata, pins, p.MAIN_WORK, root)["foundation"]
                self.assertEqual(report["accepted_b4_main_work_commit"], p.CONTENTION_BASE)
                self.assertEqual(report["accepted_b4_main_work_images"], {name: pins["foundation"]["base"][name] for name in p.CONTENTION_BASE_IMAGES})
                self.assertEqual(report["accepted_b4_p2_commit"], p.MAIN_WORK_BASE)
                self.assertEqual(report["accepted_b4_p1_commit"], p.PROCESS_BASE)
                paths = set(c.source_paths(pins, p.MAIN_WORK))
                self.assertTrue(set(c.CONTENTION_SOURCE_PATHS).issubset(paths))
                self.assertTrue(set(c.MAIN_WORK_SOURCE_PATHS).issubset(paths))
                self.assertTrue(set(c.PROCESS_SOURCE_PATHS).issubset(paths))
                self.assertNotEqual(p.CONTENTION_PROFILE, p.FOUNDATION_PROFILE)

    def test_revision_rejects_same_stage_old_graph_before_tool(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            with foundation_graph_fixture(root, p.MAIN_WORK, qualifier_revision=True) as (metadata, pins, data):
                changed = copy.deepcopy(pins)
                changed["foundation"] = data["accepted_main_work_profile"]
                with self.assertRaises(ValueError):
                    c.graph_report(metadata, changed, p.MAIN_WORK, root)
                with self.assertRaises(ValueError):
                    c.source_paths(changed, p.MAIN_WORK)
                for mutate in (lambda x: x.pop("accepted_b4_main_work_commit"), lambda x: x["qualification"].pop("qualifier_contention"), lambda x: x["qualification"]["qualifier_contention"].update(revision=True)):
                    changed = copy.deepcopy(pins)
                    mutate(changed["foundation"])
                    with self.assertRaises(ValueError):
                        c.graph_report(metadata, changed, p.MAIN_WORK, root)
                evidence = root / "graph"
                evidence.mkdir()
                record = {"format": 1, "target": p.TARGET, "profile": p.MAIN_WORK, "mode": p.MAIN_WORK, "manifest": str(root / "Cargo.toml"), "cwd": str(root)}
                (evidence / "evidence.json").write_text(json.dumps(record))
                profile_path, series_path = root / p.FOUNDATION_PROFILE, root / p.FOUNDATION_SERIES
                current_profile, current_series = profile_path.read_bytes(), series_path.read_bytes()
                try:
                    profile_path.write_bytes((root / p.CONTENTION_PROFILE).read_bytes())
                    series_path.write_bytes((root / p.CONTENTION_SERIES).read_bytes())
                    with patch.object(c.preparation, "controlled_environment") as environment, self.assertRaisesRegex(ValueError, "invalid revised main-work profile fields"):
                        c.verify(evidence, root, p.MAIN_WORK)
                    environment.assert_not_called()
                finally:
                    profile_path.write_bytes(current_profile)
                    series_path.write_bytes(current_series)
                self.assertEqual(profile_path.read_bytes(), current_profile)
                self.assertEqual(series_path.read_bytes(), current_series)

    def test_revision_transport_binding_only_enabled_library_lane(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "tools/fixture"
            root.mkdir(parents=True)
            repo = root.parents[1]
            (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/foks-hosted-transport"]\n')
            (repo / "Cargo.lock").write_text("lock\n")
            member = repo / "crates/foks-hosted-transport"
            member.mkdir(parents=True)
            (member / "Cargo.toml").write_text('[package]\nname="foks-hosted-transport"\n')
            self.assertEqual(c.frontend_transport_inputs(root, p.MAIN_WORK), {repo / "Cargo.lock", member / "Cargo.toml"})
            for mode in (p.MAIN_WORK_BIN_TEST, p.MAIN_WORK + "-default", p.MAIN_WORK + "-lifecycle"):
                self.assertEqual(c.frontend_transport_inputs(root, mode), set())
            (member / "Cargo.toml").unlink()
            with self.assertRaises(ValueError):
                c.frontend_transport_inputs(root, p.MAIN_WORK)


if __name__ == "__main__":
    unittest.main()

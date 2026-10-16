"""Closed graph/lock evidence tests; no Cargo invocation or dependency fetch."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_graph as c


def fixture():
    pins = c.preparation.load_pins()
    names = {
        c.FIXTURE: ("0.0.0", []), "celld": ("0.6.2", []),
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
            with self.assertRaisesRegex(ValueError, "target/profile"):
                c.verify(path)

    def test_saved_evidence_requires_every_native_tool_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            pins = c.preparation.load_pins()
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
            with patch.object(c.preparation, "controlled_environment", return_value=(toolchain, environment)), patch.object(c.preparation, "config_inputs", return_value={}), patch.object(c.preparation, "network_command", side_effect=lambda argv: list(map(str, argv))), patch.object(c.preparation, "native_tool_paths", return_value=native_paths), patch.object(c, "graph_report") as graph:
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
    (root / "patches/series.json").write_bytes((c.ROOT / "patches/series.json").read_bytes())
    pins = c.preparation.load_pins()
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

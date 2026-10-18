#!/usr/bin/env python3
"""Target/mode-bound engine and SQLite-free FOKS subclosure qualification.

collect is an explicit, locked/offline Cargo operation after source/lock review.
verify only reads saved evidence and current inputs; it never resolves a graph.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("celld_preparation", ROOT / "prepare-runtime.py")
preparation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preparation)
FIXTURE = "foks-celld-guarded"
TARGET = "aarch64-apple-darwin"
FORBIDDEN = {"foks-server", "foks-server-db", "foks-server-testkit"}
SQLITE = {"rusqlite", "libsqlite3-sys", "sqlite-vec"}
SOURCE_PATHS = (".gitignore", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "runtime.lock.json", "upstream-tree.json", "prepare-runtime.py", "test_prepare_runtime.py", "check_graph.py", "test_check_graph.py", "patches/series.json", "src/main.rs")
A2_SOURCE_PATHS = ("src/lifecycle.rs", "tests/lifecycle.rs", preparation.PATCH_PATH)
FOUNDATION_SOURCE_PATHS = (preparation.FOUNDATION_PROFILE, preparation.FOUNDATION_SERIES, preparation.FOUNDATION_PATCH, "node.lock.json", preparation.FOUNDATION_MANIFEST)
LIFETIMES_SOURCE_PATHS = (preparation.LIFETIMES_PATCH, preparation.LIFETIMES_PROFILE, preparation.LIFETIMES_SERIES, preparation.LIFETIMES_MANIFEST, preparation.LIFETIMES_ASSETS, "runtime/assets-node-foundation.json")
FRONTEND_SOURCE_PATHS = (preparation.FRONTEND_PATCH, preparation.FRONTEND_PROFILE, preparation.FRONTEND_SERIES, preparation.FRONTEND_MANIFEST, preparation.FRONTEND_ASSETS, "runtime/assets-node-lifetimes.json")
OBSERVATIONS_SOURCE_PATHS = (preparation.OBSERVATIONS_PATCH, preparation.OBSERVATIONS_PROFILE, preparation.OBSERVATIONS_SERIES, preparation.OBSERVATIONS_MANIFEST, preparation.OBSERVATIONS_ASSETS, "runtime/assets-node-qualification.json")
PROCESS_SOURCE_PATHS = (preparation.PROCESS_PATCH, preparation.PROCESS_PROFILE, preparation.PROCESS_SERIES, preparation.PROCESS_MANIFEST, preparation.PROCESS_ASSETS, "runtime/assets-node-observations.json")
MAIN_WORK_SOURCE_PATHS = (preparation.MAIN_WORK_PATCH, preparation.MAIN_WORK_PROFILE, preparation.MAIN_WORK_SERIES, preparation.MAIN_WORK_MANIFEST, preparation.MAIN_WORK_ASSETS, "runtime/assets-node-process.json")
CONTROL_RETIREMENT_SOURCE_PATHS = (preparation.CONTROL_RETIREMENT_PATCH, preparation.CONTROL_RETIREMENT_PROFILE, preparation.CONTROL_RETIREMENT_SERIES, preparation.CONTROL_RETIREMENT_MANIFEST, preparation.CONTROL_RETIREMENT_ASSETS)
CONTENTION_SOURCE_PATHS = (preparation.CONTENTION_PATCH, preparation.CONTENTION_PROFILE, preparation.CONTENTION_SERIES, preparation.CONTENTION_MANIFEST, preparation.CONTENTION_ASSETS, "runtime/assets-node-main-work.json")


def fail(message):
    raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def commands(root, toolchain, mode="identity"):
    features = preparation.selected_mode(preparation.load_pins(root, preparation.stage_for_mode(mode)), mode)
    shared = ["--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", *features]
    cargo = str(toolchain / "bin/cargo")
    return {
        "metadata": preparation.network_command([cargo, "metadata", *shared, "--format-version", "1", "--filter-platform", TARGET]),
        "tree": preparation.network_command([cargo, "tree", *shared, *(["--package", FIXTURE, "--package", "celld@0.6.2"] if mode == "node-baseline" or mode in preparation.NODE_MODES else []), "--target", TARGET, "--edges", "normal,build,features"]),
    }


def foundation_declarations(package, outer, source_root):
    """Declarations remain optional in all modes; activity is checked separately."""
    expected_manifest = source_root / "runtime/source/crates/celld/Cargo.toml"
    if package["manifest_path"] != str(expected_manifest) or package["source"] is not None or outer["manifest_path"] != str(source_root / "Cargo.toml") or outer["source"] is not None:
        fail("foundation package declaration source changed")
    manifest = tomllib.loads(expected_manifest.read_text())
    outer_manifest = tomllib.loads((source_root / "Cargo.toml").read_text())
    features = {"foks-hosted-transport": ["qualification"], "foks-metadata-core": ["owned-list"]}
    for name, selected in features.items():
        expected = {"path": "../../../../../../crates/" + name, "optional": True, "default-features": False, "features": selected}
        preparation.exact_json(manifest.get("dependencies", {}).get(name), expected, "foundation optional declaration")
    # No renamed, target-specific, build or dev alternative can hide alongside
    # the exact normal declarations above.
    sections = [("normal", manifest.get("dependencies", {})), ("build", manifest.get("build-dependencies", {})), ("dev", manifest.get("dev-dependencies", {}))]
    for target in manifest.get("target", {}).values():
        for section in ("dependencies", "build-dependencies", "dev-dependencies"):
            sections.append(("target", target.get(section, {})))
    for kind, dependencies in sections:
        for name, value in dependencies.items():
            actual = value.get("package", name) if isinstance(value, dict) else name
            if name.startswith("foks-") or actual.startswith("foks-"):
                if kind != "normal" or name != actual or name not in features:
                    fail("unreviewed foundation FOKS dependency declaration")
    feature = ["foks-guarded-qualification", "dep:foks-hosted-transport", "dep:foks-metadata-core"]
    actual_feature = manifest.get("features", {}).get("foks-guarded-node-qualification")
    if not isinstance(actual_feature, list) or sorted(actual_feature) != sorted(feature):
        fail("foundation celld feature declaration changed")
    if outer_manifest.get("features", {}).get("node-qualification") != ["celld/foks-guarded-node-qualification"]:
        fail("foundation outer feature declaration changed")
    declarations = package.get("dependencies")
    if not isinstance(declarations, list):
        fail("missing foundation dependency declarations")
    selected = [entry for entry in declarations if isinstance(entry, dict) and (str(entry.get("name", "")).startswith("foks-") or str(entry.get("rename", "")).startswith("foks-"))]
    if len(selected) != 2 or sorted(entry.get("name", "") for entry in selected) != preparation.FOUNDATION_EDGES:
        fail("foundation metadata declaration roster changed")
    for entry in selected:
        name = entry["name"]
        expected = {"source": None, "req": "*", "kind": None, "rename": None, "optional": True, "uses_default_features": False, "features": features[name], "target": None, "registry": None, "path": str(source_root.parents[1] / "crates" / name)}
        for field, value in expected.items():
            if field not in entry:
                fail("missing foundation declaration field")
            preparation.exact_json(entry[field], value, "foundation declaration " + field)
    return preparation.FOUNDATION_EDGES.copy()


def graph_report_predecessor(metadata, pins, mode="identity", source_root=ROOT):
    if pins.get("target") != TARGET or pins.get("profile") not in (preparation.A1, preparation.A2, *preparation.NODE_STAGES):
        fail("missing or unexpected target/profile")
    preparation.selected_mode(pins, mode)
    if not isinstance(metadata, dict) or metadata.get("version") != 1:
        fail("unsupported metadata format")
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("root"), str):
        fail("missing resolved package root")
    packages = {}
    for package in metadata.get("packages", []):
        if not isinstance(package, dict) or any(not isinstance(package.get(field), str) for field in ("id", "name", "version", "manifest_path")):
            fail("malformed package identity")
        if "source" not in package or package["source"] not in (None, preparation.CRATES_IO):
            fail("missing or unreviewed package source")
        if package["id"] in packages:
            fail("duplicate package identity")
        packages[package["id"]] = package
    nodes = {}
    for node in resolve.get("nodes", []):
        if not isinstance(node, dict) or not isinstance(node.get("id"), str) or node["id"] in nodes:
            fail("malformed or duplicate resolved node")
        nodes[node["id"]] = node
    root = resolve["root"]
    if packages.get(root, {}).get("name") != FIXTURE:
        fail("unexpected executable root")
    if metadata.get("workspace_members") != [root]:
        fail("workspace must contain only the selected harness root")

    def closure(start):
        reached = set()
        pending = list(start)
        while pending:
            current = pending.pop()
            if current in reached:
                continue
            if current not in packages or current not in nodes:
                fail("missing reachable package/node")
            reached.add(current)
            node = nodes[current]
            features = node.get("features")
            if not isinstance(features, list) or any(not isinstance(value, str) for value in features) or len(features) != len(set(features)):
                fail("missing or malformed active features")
            dependencies = node.get("deps")
            if not isinstance(dependencies, list):
                fail("missing dependency edge inventory")
            for dependency in dependencies:
                if not isinstance(dependency, dict) or not isinstance(dependency.get("pkg"), str):
                    fail("malformed dependency edge")
                kinds = dependency.get("dep_kinds")
                if not isinstance(kinds, list) or not kinds:
                    fail("unclassified dependency edge")
                production = False
                for kind in kinds:
                    if not isinstance(kind, dict) or "kind" not in kind or "target" not in kind:
                        fail("missing dependency kind/target")
                    if kind["kind"] not in (None, "build", "dev"):
                        fail("unknown dependency kind")
                    if kind["target"] is not None and (not isinstance(kind["target"], str) or not kind["target"]):
                        fail("ambiguous dependency target")
                    # Cargo's recorded filter selects the platform. Present
                    # normal/build edges must never be skipped by a cfg guess.
                    production |= kind["kind"] in (None, "build")
                if production:
                    pending.append(dependency["pkg"])
        return reached

    reached = closure([root])
    by_name = {}
    for identity in reached:
        package = packages[identity]
        by_name.setdefault(package["name"], []).append(identity)
        name = package["name"]
        if name in FORBIDDEN:
            fail("forbidden full native FOKS dependency: " + name)
        if "sqlite" in name.lower() and name not in SQLITE:
            fail("unreviewed SQLite provider: " + name)
        if package.get("links") == "sqlite3" and name != "libsqlite3-sys":
            fail("alternate SQLite native link provider")

    def exact(name, features=None, version=None):
        identities = by_name.get(name)
        if not identities or len(identities) != 1:
            fail("missing or ambiguous contract package: " + name)
        identity = identities[0]
        if features is not None and set(nodes[identity]["features"]) != set(features):
            fail("unexpected active features: " + name)
        if version is not None and packages[identity]["version"] != version:
            fail("unexpected engine/package version: " + name)
        return identity

    foundation = preparation.NODE_MODES.get(mode)
    exact(FIXTURE, foundation["root_features"] if foundation else ["lifecycle"] if mode == "lifecycle" else [])
    celld = exact("celld", foundation["celld_features"] if foundation else ["foks-guarded-qualification"] if mode == "lifecycle" else [], pins["upstream"]["version"])
    core = exact("foks-metadata-core", ["owned-list"])
    transport = exact("foks-hosted-transport", ["qualification"])
    exact("v8", pins["v8"]["features"], pins["v8"]["version"])
    for name, expected in pins["registry_engines"].items():
        identity = exact(name, version=expected["version"])
        if packages[identity]["source"] != preparation.CRATES_IO:
            fail("engine is not the pinned registry source")
    if not {"backup", "bundled", "hooks", "serialize"}.issubset(set(nodes[exact("rusqlite")]["features"])):
        fail("missing upstream SQLite construction features")
    if packages[exact("libsqlite3-sys")].get("links") != "sqlite3":
        fail("missing actual SQLite native link provider")
    fokscore = closure([core, transport])
    for identity in fokscore:
        name = packages[identity]["name"]
        if name in FORBIDDEN or "sqlite" in name.lower() or packages[identity].get("links") == "sqlite3":
            fail("SQLite/native dependency in FOKS transport/core subclosure")
    if "tokio" not in {packages[identity]["name"] for identity in fokscore}:
        fail("missing actual transport runtime dependency")
    if pins["profile"] in (preparation.A2, *preparation.NODE_STAGES):
        tokio = exact("tokio", version="1.53.1")
        if packages[tokio]["source"] != preparation.CRATES_IO:
            fail("Tokio is not the pinned registry source")
        direct_tokio = any(dependency["pkg"] == tokio and any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]) for dependency in nodes[root]["deps"])
        if direct_tokio != (foundation["direct_root_tokio"] if foundation else mode == "lifecycle"):
            fail("direct optional Tokio edge does not match selected mode")
    inventory = [{"id": identity, "name": packages[identity]["name"], "version": packages[identity]["version"], "source": packages[identity]["source"], "features": sorted(nodes[identity]["features"])} for identity in sorted(reached)]
    report = {"format": 1, "target": TARGET, "profile": pins["profile"], "mode": mode, "packages": inventory, "foks_subclosure": sorted(fokscore)}
    if foundation:
        declared = foundation_declarations(packages[celld], packages[root], source_root)
        actual_edges = []
        for dependency in nodes[celld]["deps"]:
            package = packages[dependency["pkg"]]
            if package["name"].startswith("foks-"):
                if package["name"] not in preparation.FOUNDATION_EDGES or package["source"] is not None or package["version"] != "0.4.0" or package["manifest_path"] != str(source_root.parents[1] / "crates" / package["name"] / "Cargo.toml") or dependency["dep_kinds"] != [{"kind": None, "target": None}]:
                    fail("unreviewed active celld FOKS edge")
                actual_edges.append(package["name"])
        if sorted(actual_edges) != foundation["celld_foks_edges"]:
            fail("active celld FOKS edges do not match selected mode")
        if not foundation["celld_foks_edges"] and any(packages[identity]["name"].startswith("foks-") for identity in closure([celld])):
            fail("default/lifecycle celld subclosure includes FOKS integration")
        preparation.foundation_lock(source_root, pins)
        report["foundation"] = {"profile_sha256": preparation.file_hash(source_root / preparation.FOUNDATION_PROFILE), "series_sha256": pins["foundation"]["node_series"]["sha256"], "accepted_a3a_commit": preparation.FOUNDATION_BASE, "cargo_lock_sha256": preparation.file_hash(source_root / "Cargo.lock"), "celld_foks_edges": sorted(actual_edges), "declared_foks_edges": declared}
        if pins["profile"] in (preparation.LIFETIMES, preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK):
            report["foundation"].update(accepted_b1_commit=preparation.LIFETIMES_BASE, accepted_b1_images={name: pins["foundation"]["base"][name] for name in preparation.LIFETIMES_BASE_IMAGES})
        if pins["profile"] in (preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK):
            report["foundation"].update(accepted_b2_commit=preparation.FRONTEND_BASE, accepted_b2_images={name: pins["foundation"]["base"][name] for name in preparation.FRONTEND_BASE_IMAGES})
        if pins["profile"] in (preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK):
            report["foundation"].update(accepted_b3_commit=preparation.OBSERVATIONS_BASE, accepted_b3_images={name: pins["foundation"]["base"][name] for name in preparation.OBSERVATIONS_BASE_IMAGES})
        if pins["profile"] in (preparation.PROCESS, preparation.MAIN_WORK):
            report["foundation"].update(accepted_b4_p1_commit=preparation.PROCESS_BASE, accepted_b4_p1_images={name: pins["foundation"]["base"][name] for name in preparation.PROCESS_BASE_IMAGES})
        if pins["profile"] == preparation.MAIN_WORK:
            report["foundation"].update(accepted_b4_p2_commit=preparation.MAIN_WORK_BASE, accepted_b4_p2_images={name: pins["foundation"]["base"][name] for name in preparation.MAIN_WORK_BASE_IMAGES})
    if mode == "node-baseline" or mode in preparation.NODE_BINARY_MODES or mode == preparation.MAIN_WORK_BIN_TEST:
        celld_closure = closure([celld])
        if (not foundation or not foundation["celld_foks_edges"]) and any(packages[identity]["name"].startswith("foks-") for identity in celld_closure):
            fail("ordinary celld binary subclosure includes FOKS integration")
        package = packages[celld]
        expected = source_root / "runtime/source/crates/celld"
        if package["source"] is not None or package["manifest_path"] != str(expected / "Cargo.toml"):
            fail("ordinary celld package is not the actual prepared source")
        targets = package.get("targets")
        if not isinstance(targets, list):
            fail("missing actual celld target inventory")
        binaries = [target for target in targets if isinstance(target, dict) and target.get("kind") == ["bin"]]
        if len(binaries) != 1:
            fail("missing or ambiguous ordinary celld binary target")
        target = binaries[0]
        if target.get("name") != "celld" or target.get("crate_types") != ["bin"] or target.get("src_path") != str(expected / "main.rs") or target.get("required-features", []) != []:
            fail("ordinary celld binary target changed")
        report["node_target"] = {"package_id": celld, **{key: target[key] for key in ("name", "kind", "crate_types", "src_path")}, "features": foundation["celld_features"] if foundation else []}
        report["node_subclosure"] = sorted(celld_closure)
        if mode == preparation.MAIN_WORK_BIN_TEST:
            report["node_bin_test_target"] = report["node_target"].copy()
        if pins["profile"] in (preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK) and mode != preparation.MAIN_WORK_BIN_TEST:
            libraries = [item for item in targets if isinstance(item, dict) and item.get("kind") == ["lib"]]
            if len(libraries) != 1:
                fail("missing or ambiguous frontend library target")
            library = libraries[0]
            if library.get("name") != "celld" or library.get("crate_types") != ["lib"] or library.get("src_path") != str(expected / "lib.rs") or library.get("required-features", []) != []:
                fail("frontend library target changed")
            report["node_test_target"] = {"package_id": celld, **{key: library[key] for key in ("name", "kind", "crate_types", "src_path")}, "features": foundation["celld_features"]}
    return report


def graph_report_contention_predecessor(metadata, pins, mode="identity", source_root=ROOT):
    # The archived seven-patch helper is explicit; canonical graphs never fall
    # back to it merely because the revision marker is absent.
    if pins.get("profile") == preparation.MAIN_WORK:
        preparation.require_main_work_revision(pins.get("foundation"))
    report = graph_report_predecessor(metadata, pins, mode, source_root)
    if pins.get("profile") == preparation.MAIN_WORK:
        report["foundation"].update(
            accepted_b4_main_work_commit=preparation.CONTENTION_BASE,
            accepted_b4_main_work_images={name: pins["foundation"]["base"][name] for name in preparation.CONTENTION_BASE_IMAGES},
        )
    return report


def graph_report(metadata, pins, mode="identity", source_root=ROOT):
    if pins.get("profile") == preparation.MAIN_WORK:
        preparation.require_control_retirement_revision(pins.get("foundation"))
    report = graph_report_contention_predecessor(metadata, pins, mode, source_root)
    if pins.get("profile") == preparation.MAIN_WORK:
        report["foundation"].update(
            accepted_b4_qualifier_contention_commit=preparation.CONTROL_RETIREMENT_BASE,
            accepted_b4_qualifier_contention_images={name: pins["foundation"]["base"][name] for name in preparation.CONTROL_RETIREMENT_BASE_IMAGES},
        )
    return report


def regular_files(directory):
    directory = Path(directory)
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            fail("symlink in local source inventory")
        if path.is_file():
            yield path


def frontend_transport_inputs(root, mode):
    if mode not in (preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK):
        return set()
    repo = root.parents[1]
    members = tomllib.loads((repo / "Cargo.toml").read_text()).get("workspace", {}).get("members")
    if not isinstance(members, list) or not 1 <= len(members) <= 64 or any(not isinstance(member, str) or len(member) > 256 or not re.fullmatch(r"[A-Za-z0-9_-]+(?:/[A-Za-z0-9_-]+)+", member) for member in members) or len(set(members)) != len(members):
        fail("unsupported root workspace member inventory")
    paths = {repo / "Cargo.lock", *(repo / member / "Cargo.toml" for member in members)}
    for path in paths:
        if path.is_symlink() or not path.is_file() or path.resolve() != path:
            fail("non-regular root workspace test input")
    return paths


def candidate_inputs(root, mode="identity"):
    """Snapshot before Cargo; the selected local subset must already be bound."""
    repo = root.parents[1]
    pins = preparation.load_pins(root, preparation.stage_for_mode(mode))
    paths = {repo / "Cargo.toml", *(root / name for name in source_paths(pins, mode)), *frontend_transport_inputs(root, mode)}
    paths.add(repo / pins["foks_additional_versions"]["baseline_lock"])
    for directory in (repo / "crates").iterdir():
        if directory.is_dir() and (directory / "Cargo.toml").is_file():
            paths.update(regular_files(directory))
    paths.update(regular_files(root / "runtime/source"))
    paths.update(Path(path) for path in preparation.config_inputs(root))
    return {str(path): preparation.file_hash(path) for path in sorted(paths)}


def source_paths_predecessor(pins, mode="identity"):
    preparation.selected_mode(pins, mode)
    return (
        SOURCE_PATHS
        + (A2_SOURCE_PATHS if pins["profile"] in (preparation.A2, *preparation.NODE_STAGES) else ())
        + (FOUNDATION_SOURCE_PATHS if pins["profile"] in preparation.NODE_STAGES else ())
        + (LIFETIMES_SOURCE_PATHS if pins["profile"] in (preparation.LIFETIMES, preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK) else ())
        + (FRONTEND_SOURCE_PATHS if pins["profile"] in (preparation.FRONTEND, preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK) else ())
        + (OBSERVATIONS_SOURCE_PATHS if pins["profile"] in (preparation.OBSERVATIONS, preparation.PROCESS, preparation.MAIN_WORK) else ())
        + (PROCESS_SOURCE_PATHS if pins["profile"] in (preparation.PROCESS, preparation.MAIN_WORK) else ())
        + (MAIN_WORK_SOURCE_PATHS if pins["profile"] == preparation.MAIN_WORK else ())
        + (("node.lock.json", "run-node.py", "test_run_node.py") if mode == "node-baseline" else ())
    )


def source_paths_contention_predecessor(pins, mode="identity"):
    if pins.get("profile") == preparation.MAIN_WORK:
        preparation.require_main_work_revision(pins.get("foundation"))
    paths = source_paths_predecessor(pins, mode)
    return paths + (CONTENTION_SOURCE_PATHS if pins.get("profile") == preparation.MAIN_WORK else ())


def source_paths(pins, mode="identity"):
    if pins.get("profile") == preparation.MAIN_WORK:
        preparation.require_control_retirement_revision(pins.get("foundation"))
    paths = source_paths_contention_predecessor(pins, mode)
    return paths + (CONTROL_RETIREMENT_SOURCE_PATHS if pins.get("profile") == preparation.MAIN_WORK else ())


def selected_inputs(metadata, reached, root, mode="identity"):
    repo = root.parents[1]
    pins = preparation.load_pins(root, preparation.stage_for_mode(mode))
    paths = {repo / "Cargo.toml", *(root / name for name in source_paths(pins, mode)), root / "runtime/source/Cargo.toml", root / "runtime/source/Cargo.lock", *frontend_transport_inputs(root, mode)}
    paths.add(repo / pins["foks_additional_versions"]["baseline_lock"])
    for package in metadata["packages"]:
        if package["id"] not in reached or package["source"] is not None:
            continue
        manifest = Path(package["manifest_path"]).resolve()
        directory = manifest.parent
        if manifest.name != "Cargo.toml" or not directory.is_relative_to(repo):
            fail("local package escapes reviewed repository")
        if directory == root:
            continue
        if directory.is_relative_to(root / "runtime/source/crates"):
            paths.update(regular_files(root / "runtime/source"))
        elif directory.parent == repo / "crates":
            paths.update(regular_files(directory))
        else:
            fail("unreviewed local package directory")
    paths.update(Path(path) for path in preparation.config_inputs(root))
    return {str(path): preparation.file_hash(path) for path in sorted(paths)}


def verify(directory, root=ROOT, mode="identity"):
    directory = Path(directory).resolve()
    evidence = preparation.read_json(directory / "evidence.json", 16 * 1024 * 1024)
    stage = preparation.stage_for_mode(mode)
    pins = preparation.load_pins(root, stage)
    preparation.selected_mode(pins, mode)
    preparation.foundation_lock(root, pins)
    if evidence.get("format") != 1 or evidence.get("target") != TARGET or evidence.get("profile") != pins["profile"] or evidence.get("manifest") != str(root / "Cargo.toml") or evidence.get("cwd") != str(root):
        fail("evidence target/profile/working-directory mismatch")
    # Historical A1 identity evidence predates explicit modes. A2 may never
    # inherit that implicit default, nor may either profile cross modes.
    recorded_mode = evidence.get("mode", "identity" if pins["profile"] == preparation.A1 else None)
    if recorded_mode != mode:
        fail("evidence mode mismatch")
    toolchain, environment = preparation.controlled_environment(root)
    selected_commands = commands(root, toolchain, mode)
    captured = evidence.get("commands")
    if not isinstance(captured, dict) or set(captured) != set(selected_commands):
        fail("missing command provenance")
    for name, argv in selected_commands.items():
        command = captured[name]
        if not isinstance(command, dict) or command.get("argv") != argv or command.get("cwd") != str(root) or command.get("exit_code") != 0 or command.get("reason") != "completed" or command.get("cleanup_established") is not True:
            fail("command did not complete with exact selected target/features")
        for stream in ("stdout", "stderr"):
            if command.get(stream + "_sha256") != preparation.file_hash(directory / (name + "." + stream)):
                fail("command output hash mismatch")
    tools = evidence.get("tools")
    if not isinstance(tools, dict) or tools.get("environment") != environment or tools.get("cargo_configs") != preparation.config_inputs(root):
        fail("toolchain environment/config evidence changed")
    for name, argv in preparation.identity_commands(root, toolchain).items():
        record = tools.get(name)
        if not isinstance(record, dict) or record.get("exit_code") != 0 or record.get("cleanup_established") is not True or record.get("argv") != preparation.network_command(argv) or record.get("cwd") != str(root):
            fail("missing actual tool identity evidence")
        for stream in ("stdout", "stderr"):
            if record.get(stream + "_sha256") != preparation.file_hash(directory / (name + "." + stream)):
                fail("tool identity output changed")
    compiler = (directory / "rustc-version.stdout").read_text()
    if "commit-hash: " + pins["toolchain"]["rustc_commit"] not in compiler.splitlines():
        fail("wrong actual compiler commit")
    if not isinstance(tools.get("native_tools"), dict) or set(tools["native_tools"]) != set(preparation.native_tool_paths(root)):
        fail("native compiler/linker/SDK input inventory incomplete or unexpected")
    if any(preparation.file_hash(path) != expected for path, expected in tools["native_tools"].items()):
        fail("native compiler/linker/SDK inputs changed")
    metadata = preparation.read_json(directory / "metadata.stdout", 32 * 1024 * 1024)
    report = graph_report(metadata, pins, mode, root)
    reached = {package["id"] for package in report["packages"]}
    current = selected_inputs(metadata, reached, root, mode)
    if evidence.get("inputs") != current:
        fail("local manifest/lock/source inputs changed or incomplete")
    if evidence.get("source") != preparation.verify_source(root, stage):
        fail("prepared runtime source changed")
    if evidence.get("lock_delta") != preparation.compare_locks(root):
        fail("lock delta changed")
    if evidence.get("upstream_edges") != preparation.upstream_edges(metadata, reached, root, mode):
        fail("selected upstream dependency provenance changed")
    if evidence.get("registry_inputs") != preparation.registry_inputs(metadata, reached, root):
        fail("registry manifest/build/MSRV inputs changed")
    if mode == "node-baseline":
        node_inputs = preparation.node_inputs(root)
        if evidence.get("node_inputs") != node_inputs:
            fail("node profile/controller/fixture inputs changed or incomplete")
        report["node_inputs"] = node_inputs
    elif "node_inputs" in evidence:
        fail("node input evidence is invalid for a historical mode")
    report["lock_delta"] = evidence["lock_delta"]
    report["upstream_edges"] = evidence["upstream_edges"]
    report["registry_inputs"] = evidence["registry_inputs"]
    report["evidence_sha256"] = preparation.file_hash(directory / "evidence.json")
    report["metadata_sha256"] = preparation.file_hash(directory / "metadata.stdout")
    report["local_inputs_sha256"] = digest(json.dumps(current, sort_keys=True).encode())
    return report


def collect(directory, root=ROOT, mode="identity"):
    directory = Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=False)
    stage = preparation.stage_for_mode(mode)
    pins = preparation.load_pins(root, stage)
    preparation.selected_mode(pins, mode)
    preparation.foundation_lock(root, pins)
    source = preparation.verify_source(root, stage)
    lock_delta = preparation.compare_locks(root)
    toolchain, environment = preparation.controlled_environment(root)
    before = candidate_inputs(root, mode)
    evidence = {"format": 1, "target": TARGET, "profile": pins["profile"], "mode": mode, "manifest": str(root / "Cargo.toml"), "cwd": str(root), "source": source, "lock_delta": lock_delta, "commands": {}}
    if mode == "node-baseline":
        evidence["node_inputs"] = preparation.node_inputs(root)
    with preparation.BuildSlot(root) as slot:
        try:
            evidence["tools"] = preparation.tool_identity(root, directory, slot, environment, toolchain)
            for name, argv in commands(root, toolchain, mode).items():
                evidence["commands"][name] = preparation.run_owned(argv, root, environment, directory, name, slot, 120)
                preparation.write_json(directory / "evidence.json", evidence)
            metadata = preparation.read_json(directory / "metadata.stdout", 32 * 1024 * 1024)
            report = graph_report(metadata, pins, mode, root)
            reached = {package["id"] for package in report["packages"]}
            current = selected_inputs(metadata, reached, root, mode)
            if any(before.get(path) != value for path, value in current.items()):
                fail("local input changed during graph collection")
            evidence["inputs"] = current
            evidence["upstream_edges"] = preparation.upstream_edges(metadata, reached, root, mode)
            evidence["registry_inputs"] = preparation.registry_inputs(metadata, reached, root)
            if mode == "node-baseline" and evidence["node_inputs"] != preparation.node_inputs(root):
                fail("node profile/controller/fixture changed during graph collection")
        finally:
            preparation.write_json(directory / "evidence.json", evidence)
    report = verify(directory, root, mode)
    preparation.write_json(directory / "report.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    for name in ("collect", "verify"):
        operation = sub.add_parser(name)
        operation.add_argument("directory", type=Path)
        operation.add_argument("--mode", choices=("identity", "lifecycle", "node-baseline", *preparation.NODE_MODES), default="identity")
    args = parser.parse_args()
    try:
        report = collect(args.directory, mode=args.mode) if args.operation == "collect" else verify(args.directory, mode=args.mode)
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, TypeError, KeyError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Collect or verify target-bound, native-disabled external-consumer evidence.

Collection invokes Cargo only when explicitly requested after source review.
Verification reads saved output; it never guesses a target or drops target edges.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

PROFILES = ("default", "qualification", "test-support")
FORBIDDEN = {"foks-server", "foks-server-db", "foks-server-testkit", "rusqlite", "libsqlite3-sys"}
FIXTURE = "foks-hosted-transport-check"
FORMAT = 1


def fail(message):
    raise ValueError(message)


def digest(content):
    return hashlib.sha256(content).hexdigest()


def target_kind(target):
    if not isinstance(target, str) or not re.fullmatch(r"[A-Za-z0-9_]+(?:-[A-Za-z0-9_]+){2,}", target):
        fail("missing or ambiguous selected target")
    if target == "wasm32-unknown-unknown":
        return "wasm"
    if target.startswith("wasm"):
        fail("unsupported Wasm target")
    return "native"


def commands(manifest, target, profile):
    target_kind(target)
    if profile not in PROFILES:
        fail("unknown feature profile")
    shared = ["--locked", "--offline", "--manifest-path", str(manifest), "--no-default-features"]
    if profile != "default":
        shared += ["--features", profile]
    return {
        "metadata": ["cargo", "metadata", *shared, "--format-version", "1", "--filter-platform", target],
        "tree": ["cargo", "tree", *shared, "--target", target, "--edges", "normal,build,features"],
    }


def graph_report(metadata, target, profile):
    """All normal/build edges left by Cargo's selected-target filter are followed."""
    kind = target_kind(target)
    if profile not in PROFILES:
        fail("unknown feature profile")
    if not isinstance(metadata, dict) or metadata.get("version") != 1:
        fail("unsupported metadata evidence")
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("root"), str):
        fail("missing resolved fixture root")
    packages = {}
    for package in metadata.get("packages", []):
        if not isinstance(package, dict) or not isinstance(package.get("id"), str) or not isinstance(package.get("name"), str):
            fail("malformed package")
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
        fail("unexpected fixture root")
    pending = [root]
    reached = set()
    features = {}
    inventory = []
    while pending:
        current = pending.pop()
        if current in reached:
            continue
        if current not in packages or current not in nodes:
            fail("missing reachable package or resolved node")
        reached.add(current)
        package, node = packages[current], nodes[current]
        name = package["name"]
        active = node.get("features")
        if not isinstance(active, list) or any(not isinstance(feature, str) for feature in active) or len(active) != len(set(active)):
            fail("missing or malformed active features")
        features.setdefault(name, []).append(set(active))
        inventory.append({"id": current, "name": name, "features": sorted(active)})
        if name in FORBIDDEN or (kind == "wasm" and name in {"foks-hosted-transport", "tokio"}):
            fail("forbidden production dependency: " + name)
        deps = node.get("deps")
        if not isinstance(deps, list):
            fail("missing resolved dependency edges")
        for dependency in deps:
            if not isinstance(dependency, dict) or not isinstance(dependency.get("pkg"), str):
                fail("malformed resolved edge")
            edge_kinds = dependency.get("dep_kinds")
            if not isinstance(edge_kinds, list) or not edge_kinds:
                fail("unclassified resolved edge")
            production = False
            for edge in edge_kinds:
                if not isinstance(edge, dict) or "kind" not in edge or "target" not in edge:
                    fail("unclassified dependency kind or target")
                if edge["kind"] not in (None, "build", "dev"):
                    fail("unknown dependency kind")
                if edge["target"] is not None and (not isinstance(edge["target"], str) or not edge["target"]):
                    fail("ambiguous edge target")
                # Cargo already selected the target. Never guess at cfg syntax
                # or skip a present production edge because it has a selector.
                production |= edge["kind"] in (None, "build")
            if production:
                pending.append(dependency["pkg"])
    expected_root = set() if profile == "default" else {"qualification"}
    if profile == "test-support":
        expected_root.add("test-support")
    if features.get(FIXTURE) != [expected_root]:
        fail("fixture feature profile mismatch")
    if features.get("foks-metadata-core") != [{"owned-list"}]:
        fail("metadata core is not exactly native-disabled owned-list")
    if kind == "native":
        if features.get("foks-hosted-transport") != [expected_root]:
            fail("transport feature profile mismatch")
        if "tokio" not in features:
            fail("missing native transport runtime dependency")
    return {"target": target, "profile": profile, "kind": kind, "packages": sorted(inventory, key=lambda package: package["id"])}


def input_paths(metadata, manifest):
    repo = manifest.parents[2]
    paths = {repo / "Cargo.toml", manifest, manifest.with_name("Cargo.lock")}
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        fail("missing local dependency package inventory")
    for package in metadata["packages"]:
        if not isinstance(package, dict) or "source" not in package:
            fail("missing dependency source classification")
        if package["source"] is not None and (not isinstance(package["source"], str) or not package["source"]):
            fail("ambiguous dependency source classification")
        if package["source"] is None:
            value = package.get("manifest_path")
            if not isinstance(value, str):
                fail("missing local dependency manifest path")
            path = Path(value).resolve()
            if not path.is_relative_to(repo) or path.name != "Cargo.toml":
                fail("local dependency manifest escapes fixture repository")
            paths.add(path)
    return sorted(paths)


def verify(directory):
    directory = Path(directory)
    evidence = json.loads((directory / "evidence.json").read_text())
    if evidence.get("format") != FORMAT:
        fail("unsupported evidence envelope")
    target, profile = evidence.get("target"), evidence.get("profile")
    manifest = (Path(__file__).resolve().parent / "Cargo.toml").resolve()
    if evidence.get("manifest") != str(manifest):
        fail("evidence belongs to another fixture")
    expected = commands(manifest, target, profile)
    captured = evidence.get("commands")
    if not isinstance(captured, dict) or set(captured) != set(expected):
        fail("missing command evidence")
    for name, argv in expected.items():
        command = captured[name]
        if not isinstance(command, dict) or command.get("argv") != argv or command.get("exit_code") != 0:
            fail("command did not succeed with the selected target/profile")
        for stream in ("stdout", "stderr"):
            content = (directory / (name + "." + stream)).read_bytes()
            if command.get(stream + "_sha256") != digest(content):
                fail("command output hash mismatch")
    metadata = json.loads((directory / "metadata.stdout").read_text())
    expected_inputs = input_paths(metadata, manifest)
    inputs = evidence.get("inputs")
    if not isinstance(inputs, dict) or set(inputs) != {str(path) for path in expected_inputs}:
        fail("missing dependency manifest/lock inputs")
    for path in expected_inputs:
        if inputs[str(path)] != digest(path.read_bytes()):
            fail("dependency manifest/lock input changed")
    return graph_report(metadata, target, profile)


def collect(directory, target, profile):
    manifest = (Path(__file__).resolve().parent / "Cargo.toml").resolve()
    selected = commands(manifest, target, profile)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=False)
    # Snapshot local manifests before resolution, so an edit during collection
    # cannot bind old Cargo output to a newer dependency declaration.
    repo = manifest.parents[2]
    before_paths = {repo / "Cargo.toml", manifest, manifest.with_name("Cargo.lock"), *repo.glob("crates/*/Cargo.toml")}
    before = {str(path.resolve()): digest(path.read_bytes()) for path in before_paths}
    evidence = {"format": FORMAT, "manifest": str(manifest), "target": target, "profile": profile, "commands": {}}
    for name, argv in selected.items():
        result = subprocess.run(argv, cwd=manifest.parent, capture_output=True, check=False)
        command = {"argv": argv, "exit_code": result.returncode}
        for stream in ("stdout", "stderr"):
            content = getattr(result, stream)
            (directory / (name + "." + stream)).write_bytes(content)
            command[stream + "_sha256"] = digest(content)
        evidence["commands"][name] = command
        (directory / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
        if result.returncode:
            fail(name + " command failed; output retained")
    metadata = json.loads((directory / "metadata.stdout").read_text())
    selected_inputs = input_paths(metadata, manifest)
    for path in selected_inputs:
        if before.get(str(path)) != digest(path.read_bytes()):
            fail("dependency declaration changed or was unbound during collection")
    evidence["inputs"] = {str(path): before[str(path)] for path in selected_inputs}
    (directory / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    report = verify(directory)
    (directory / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    collection = sub.add_parser("collect")
    collection.add_argument("directory")
    collection.add_argument("--target", required=True)
    collection.add_argument("--profile", choices=PROFILES, required=True)
    verification = sub.add_parser("verify")
    verification.add_argument("directory")
    args = parser.parse_args()
    try:
        report = collect(args.directory, args.target, args.profile) if args.operation == "collect" else verify(args.directory)
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, TypeError, KeyError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

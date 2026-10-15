#!/usr/bin/env python3
"""Check the workspace dependency graph against tools/crate-layers.toml.

Usage: scripts/check-crate-layers.py [--policy PATH] [--metadata PATH]

Without --metadata the script runs
`cargo metadata --offline --locked --no-deps --format-version 1` at the
repository root. The declared dependencies in that output include optional,
target-specific and feature-gated dependencies, so no feature combination can
hide an edge. Requires Python 3.11 or newer for tomllib.

The script reports, and exits with status 1 on:
- a workspace package outside `package_roots`, in no layer, or in more than
  one layer, and a layer entry that names no workspace package;
- a path dependency that does not resolve to a workspace package;
- a normal or build dependency on a workspace package that the layer rules do
  not allow and no `[[exception]]` covers;
- an exception for an edge that does not exist or that the layer rules already
  allow;
- for an exception edge, a Rust source file in the depending package that
  names the dependency's crate but matches none of the exception's `sites`,
  and a `sites` pattern that matches no such file;
- a production (normal or build) path, direct or transitive, from any package
  to a package in a `test_only` layer.
"""
import argparse
import fnmatch
import json
import os
from pathlib import Path
import re
import subprocess
import sys

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 and older
    sys.exit("check-crate-layers.py requires Python 3.11 or newer (tomllib)")

REPOSITORY = Path(__file__).resolve().parent.parent
DEFAULT_POLICY = REPOSITORY / "tools" / "crate-layers.toml"
PRODUCTION_KINDS = (None, "build")
LINE_COMMENT = re.compile(r"//.*")


def load_policy(path):
    with open(path, "rb") as file:
        return tomllib.load(file)


def cargo_metadata(root):
    output = subprocess.run(
        ["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version", "1"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(output)


def relative(path, root):
    path = Path(path)
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def crate_identifier(dependency):
    return (dependency.get("rename") or dependency["name"]).replace("-", "_")


def workspace_graph(metadata, root):
    """Return {package: directory} and the list of path-dependency edges.

    Each edge is (package, dependency package or None, kind, crate identifier,
    dependency path). The dependency package is None when the path is not a
    workspace package directory.
    """
    members = set(metadata["workspace_members"])
    directories = {}
    for package in metadata["packages"]:
        if package["id"] in members:
            directories[package["name"]] = relative(Path(package["manifest_path"]).parent, root)
    by_directory = {directory: name for name, directory in directories.items()}
    edges = []
    for package in metadata["packages"]:
        if package["id"] not in members:
            continue
        for dependency in package["dependencies"]:
            if dependency.get("path") is None:
                continue
            path = relative(dependency["path"], root)
            edges.append(
                (
                    package["name"],
                    by_directory.get(path),
                    dependency["kind"],
                    crate_identifier(dependency),
                    path,
                )
            )
    return directories, edges


def names_crate(path, identifier):
    """Report whether a Rust source file names a crate outside line comments."""
    pattern = re.compile(rf"\b{re.escape(identifier)}\b")
    text = path.read_text(encoding="utf-8", errors="replace")
    return any(pattern.search(LINE_COMMENT.sub("", line)) for line in text.splitlines())


def rust_sources(directory):
    for parent, subdirectories, files in os.walk(directory):
        subdirectories[:] = sorted(
            name for name in subdirectories if name != "target" and not name.startswith(".")
        )
        for name in sorted(files):
            if name.endswith(".rs"):
                yield Path(parent) / name


def check(policy, metadata, root):
    """Return a list of policy violations, empty when the graph conforms."""
    root = Path(root)
    errors = []
    directories, edges = workspace_graph(metadata, root)
    layers = policy.get("layer", {})

    layer_of = {}
    for layer, definition in layers.items():
        for allowed in definition.get("may_depend_on", []):
            if allowed not in layers:
                errors.append(f"layer {layer} may depend on undefined layer {allowed}")
        for package in definition.get("packages", []):
            if package in layer_of:
                errors.append(f"{package} is in layers {layer_of[package]} and {layer}")
            elif package not in directories:
                errors.append(f"layer {layer} lists {package}, which is not a workspace package")
            else:
                layer_of[package] = layer

    roots = policy.get("workspace", {}).get("package_roots", [])
    for package, directory in sorted(directories.items()):
        if package not in layer_of:
            errors.append(f"{package} ({directory}) is not assigned to a layer")
        if not any(fnmatch.fnmatchcase(directory, pattern) for pattern in roots):
            errors.append(f"{package} is defined outside the workspace package roots: {directory}")

    def allowed(package, dependency):
        source, target = layer_of.get(package), layer_of.get(dependency)
        if source is None or target is None:
            return True  # already reported as unassigned
        return source == target or target in layers[source].get("may_depend_on", [])

    exceptions = {}
    for exception in policy.get("exception", []):
        key = (exception["package"], exception["dependency"])
        if not exception.get("reason") or not exception.get("sites"):
            errors.append(f"exception {key[0]} -> {key[1]} needs a reason and at least one site")
        exceptions.setdefault(key, []).append(exception)

    production = {}
    identifiers = {}
    for package, dependency, kind, identifier, path in edges:
        if dependency is None:
            errors.append(f"{package} has a path dependency outside the workspace: {path}")
            continue
        if kind not in PRODUCTION_KINDS:
            continue
        production.setdefault(package, set()).add(dependency)
        identifiers[(package, dependency)] = identifier

    for package in sorted(production):
        for dependency in sorted(production[package]):
            if not allowed(package, dependency) and (package, dependency) not in exceptions:
                errors.append(
                    f"{package} ({layer_of[package]}) depends on {dependency} "
                    f"({layer_of[dependency]}); the layer rules do not allow this edge "
                    "and no exception covers it"
                )

    for (package, dependency), entries in sorted(exceptions.items()):
        if dependency not in production.get(package, set()):
            errors.append(f"exception {package} -> {dependency} names an edge that does not exist")
            continue
        if allowed(package, dependency):
            errors.append(
                f"exception {package} -> {dependency} is unnecessary; the layer rules allow it"
            )
            continue
        identifier = identifiers[(package, dependency)]
        sources = [
            relative(path, root)
            for path in rust_sources(root / directories[package])
            if names_crate(path, identifier)
        ]
        patterns = [pattern for entry in entries for pattern in entry.get("sites", [])]
        for source in sources:
            if not any(fnmatch.fnmatchcase(source, pattern) for pattern in patterns):
                errors.append(
                    f"{source} uses {identifier}, but no {package} -> {dependency} "
                    "exception lists it as a site"
                )
        for pattern in patterns:
            if not any(fnmatch.fnmatchcase(source, pattern) for source in sources):
                errors.append(
                    f"exception {package} -> {dependency} site {pattern} "
                    f"matches no file that uses {identifier}"
                )

    test_only = {
        package
        for layer, definition in layers.items()
        if definition.get("test_only")
        for package in definition.get("packages", [])
    }
    for package in sorted(directories):
        if package in test_only:
            continue
        reached, pending = set(), [package]
        while pending:
            for dependency in production.get(pending.pop(), set()):
                if dependency not in reached:
                    reached.add(dependency)
                    pending.append(dependency)
        for dependency in sorted(reached & test_only):
            errors.append(f"{package} has a production dependency on test-only {dependency}")

    return errors


def main(arguments=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--metadata", type=Path, help="read cargo metadata JSON from this file")
    options = parser.parse_args(arguments)
    if options.metadata is None:
        metadata = cargo_metadata(REPOSITORY)
    else:
        metadata = json.loads(options.metadata.read_text(encoding="utf-8"))
    errors = check(load_policy(options.policy), metadata, REPOSITORY)
    for error in errors:
        print(f"crate layers: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())

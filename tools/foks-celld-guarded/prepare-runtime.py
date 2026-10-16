#!/usr/bin/env python3
"""Prepare and qualify pinned A1 inputs; no command downloads or resolves dependencies.

verify/prepare accept the reviewed local celld archive. assets accepts the two
reviewed local V8 assets. graph collection and build are explicit later actions,
after source/lock review, and share one fail-closed process-group build slot.
"""
import argparse
import contextlib
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parent
TARGET = "aarch64-apple-darwin"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
CHUNK = 64 * 1024


def fail(message):
    raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_hash(path, algorithm="sha256"):
    result = hashlib.new(algorithm)
    with Path(path).open("rb") as stream:
        while chunk := stream.read(CHUNK):
            result.update(chunk)
    return result.hexdigest()


def read_json(path, maximum=2 * 1024 * 1024):
    path = Path(path)
    if not path.is_file() or path.is_symlink() or path.stat().st_size > maximum:
        fail("invalid or oversized JSON input: " + str(path))
    return json.loads(path.read_text())


def write_json(path, value):
    path = Path(path)
    data = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    temporary = path.with_name(path.name + ".new")
    with temporary.open("xb") as stream:
        stream.write(data)
    os.replace(temporary, path)


def checked_file(path, expected):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        fail("expected a regular input file: " + str(path))
    if path.stat().st_size != expected["bytes"] or file_hash(path) != expected["sha256"]:
        fail("input size/hash mismatch: " + str(path))
    return path


def relative_name(value):
    if not isinstance(value, str) or not value or len(value.encode()) > 1024:
        fail("invalid relative path")
    parts = value.split("/")
    if any(part in ("", ".", "..") for part in parts) or "\\" in value:
        fail("unsafe relative path")
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        fail("control byte in relative path")
    if PurePosixPath(value).is_absolute():
        fail("absolute archive path")
    return value


def load_pins(root=ROOT):
    pins = read_json(root / "runtime.lock.json")
    if pins.get("format") != 1 or pins.get("profile") != "a1-library-identity" or pins.get("target") != TARGET:
        fail("unsupported input profile")
    expected = {"format", "profile", "target", "foks_baseline", "upstream", "patch_series_sha256", "toolchain", "v8", "registry_engines", "build", "cargo_config_sha256", "sdk", "foks_additional_versions"}
    if set(pins) != expected:
        fail("unrecognized input-record field")
    series = root / "patches/series.json"
    if file_hash(series) != pins["patch_series_sha256"] or read_json(series) != {"format": 1, "patches": []}:
        fail("A1 requires the exact empty patch series")
    return pins


def inventory(root, pins):
    path = root / "upstream-tree.json"
    if file_hash(path) != pins["upstream"]["inventory_sha256"]:
        fail("upstream inventory hash mismatch")
    value = read_json(path)
    upstream = pins["upstream"]
    if value.get("truncated") is not False or value.get("sha") != upstream["commit"]:
        fail("incomplete or misidentified inventory")
    entries = {}
    for entry in value.get("tree", []):
        name = relative_name(entry.get("path"))
        mode = entry.get("mode")
        if name in entries or mode not in ("100644", "100755", "040000"):
            fail("duplicate or unsupported inventory entry")
        if entry.get("type") != ("tree" if mode == "040000" else "blob"):
            fail("inventory mode/type mismatch")
        if not re.fullmatch(r"[0-9a-f]{40}", entry.get("sha", "")):
            fail("invalid Git object identity")
        if mode != "040000" and (type(entry.get("size")) is not int or entry["size"] < 0):
            fail("invalid inventory file length")
        entries[name] = entry
    files = [entry for entry in entries.values() if entry["type"] == "blob"]
    directories = [name for name, entry in entries.items() if entry["type"] == "tree"]
    for name in entries:
        parent = name.rpartition("/")[0]
        if parent and (parent not in entries or entries[parent]["type"] != "tree"):
            fail("inventory parent directory missing")
    if len(files) != upstream["files"] or len(directories) != upstream["directories"] or sum(entry["size"] for entry in files) != upstream["payload_bytes"]:
        fail("inventory resource bounds mismatch")
    # Git orders a directory as if its basename ended in '/', not a plain name.
    for directory in sorted(["", *directories], key=lambda name: name.count("/"), reverse=True):
        children = []
        for name, entry in entries.items():
            parent, _, leaf = name.rpartition("/")
            if parent == directory:
                children.append((leaf, entry))
        children.sort(key=lambda item: (item[0] + ("/" if item[1]["type"] == "tree" else "")).encode())
        body = b"".join((entry["mode"].lstrip("0") + " " + leaf).encode() + b"\0" + bytes.fromhex(entry["sha"]) for leaf, entry in children)
        actual = hashlib.sha1(b"tree " + str(len(body)).encode() + b"\0" + body).hexdigest()
        expected = upstream["tree"] if not directory else entries[directory]["sha"]
        if actual != expected:
            fail("inventory Git tree mismatch")
    return entries


def inspect_archive(archive, root=ROOT, destination=None):
    pins = load_pins(root)
    upstream = pins["upstream"]
    expected = inventory(root, pins)
    checked_file(archive, {"bytes": upstream["archive_bytes"], "sha256": upstream["archive_sha256"]})
    seen = set()
    manifest = []
    with tarfile.open(archive, "r|gz") as source:
        for member in source:
            name = relative_name(member.name)
            if name in seen or len(seen) >= len(expected) + 1:
                fail("duplicate or excessive archive entries")
            seen.add(name)
            prefix = upstream["archive_root"]
            if name == prefix:
                if not member.isdir() or member.mode != upstream["directory_mode"] or member.size:
                    fail("invalid archive root")
                continue
            if not name.startswith(prefix + "/"):
                fail("archive root mismatch")
            name = relative_name(name[len(prefix) + 1:])
            entry = expected.get(name)
            if entry is None:
                fail("unexpected archive entry")
            if member.isdir():
                if entry["type"] != "tree" or member.mode != upstream["directory_mode"] or member.size:
                    fail("archive directory mismatch")
                if destination is not None:
                    path = destination / name
                    path.mkdir(mode=0o755, parents=True, exist_ok=True)
                    path.chmod(0o755)
                continue
            if not member.isfile() or entry["type"] != "blob" or member.mode != upstream["file_mode"] or member.size != entry["size"]:
                fail("archive type/mode/length mismatch")
            data = source.extractfile(member)
            if data is None:
                fail("missing archive file")
            git_hash = hashlib.sha1(b"blob " + str(member.size).encode() + b"\0")
            sha = hashlib.sha256()
            remaining = member.size
            with contextlib.ExitStack() as stack:
                output = None
                if destination is not None:
                    path = destination / name
                    path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
                    output = stack.enter_context(path.open("xb"))
                    path.chmod(0o755 if entry["mode"] == "100755" else 0o644)
                while remaining:
                    chunk = data.read(min(CHUNK, remaining))
                    if not chunk:
                        fail("truncated archive payload")
                    remaining -= len(chunk)
                    git_hash.update(chunk)
                    sha.update(chunk)
                    if output is not None:
                        output.write(chunk)
                if data.read(1) or git_hash.hexdigest() != entry["sha"]:
                    fail("archive Git blob mismatch")
            manifest.append({"path": name, "mode": entry["mode"], "bytes": member.size, "sha256": sha.hexdigest(), "git_sha1": entry["sha"]})
        if source.pax_headers != {"comment": upstream["commit"]}:
            fail("archive commit header mismatch")
    wanted = {upstream["archive_root"], *(upstream["archive_root"] + "/" + name for name in expected)}
    if seen != wanted:
        fail("missing archive entries")
    # Detect a replacement or edit during inspection rather than binding a newer
    # input path to earlier parsed bytes.
    checked_file(archive, {"bytes": upstream["archive_bytes"], "sha256": upstream["archive_sha256"]})
    return {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "archive_sha256": upstream["archive_sha256"], "tree": upstream["tree"], "patch_series_sha256": pins["patch_series_sha256"], "files": sorted(manifest, key=lambda item: item["path"])}


def runtime_directory(root=ROOT):
    path = root / "runtime"
    if path.is_symlink():
        fail("runtime directory is a symlink")
    path.mkdir(mode=0o700, exist_ok=True)
    if not path.is_dir():
        fail("runtime path is not a directory")
    return path


def verify_source(root=ROOT):
    pins = load_pins(root)
    expected = inventory(root, pins)
    runtime = runtime_directory(root)
    source = runtime / "source"
    recorded = read_json(runtime / "source-manifest.json")
    if source.is_symlink() or not source.is_dir():
        fail("missing regular prepared source directory")
    seen = set()
    files = []
    for parent, directories, names in os.walk(source, followlinks=False):
        for leaf in [*directories, *names]:
            path = Path(parent) / leaf
            name = path.relative_to(source).as_posix()
            entry = expected.get(name)
            mode = path.lstat().st_mode
            if name in seen or entry is None or stat.S_ISLNK(mode):
                fail("altered prepared source roster")
            seen.add(name)
            if entry["type"] == "tree":
                if not stat.S_ISDIR(mode) or stat.S_IMODE(mode) != 0o755:
                    fail("altered prepared directory mode/type")
                continue
            required = 0o755 if entry["mode"] == "100755" else 0o644
            if not stat.S_ISREG(mode) or stat.S_IMODE(mode) != required or path.stat().st_size != entry["size"]:
                fail("altered prepared file mode/length")
            git_hash = hashlib.sha1(b"blob " + str(entry["size"]).encode() + b"\0")
            sha = hashlib.sha256()
            with path.open("rb") as stream:
                while chunk := stream.read(CHUNK):
                    git_hash.update(chunk)
                    sha.update(chunk)
            if git_hash.hexdigest() != entry["sha"]:
                fail("altered prepared file contents")
            files.append({"path": name, "mode": entry["mode"], "bytes": entry["size"], "sha256": sha.hexdigest(), "git_sha1": entry["sha"]})
    actual = {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "archive_sha256": pins["upstream"]["archive_sha256"], "tree": pins["upstream"]["tree"], "patch_series_sha256": pins["patch_series_sha256"], "files": sorted(files, key=lambda item: item["path"])}
    if seen != set(expected) or recorded != actual:
        fail("prepared source manifest mismatch")
    if file_hash(source / "Cargo.lock") != pins["upstream"]["cargo_lock_sha256"]:
        fail("upstream lockfile changed")
    return actual


def prepare(archive, root=ROOT):
    runtime = runtime_directory(root)
    destination = runtime / "source"
    if destination.exists() or destination.is_symlink():
        return verify_source(root)
    if (runtime / "source-manifest.json").exists():
        fail("orphan prepared manifest requires inspection")
    staging = Path(tempfile.mkdtemp(prefix="source-staging-", dir=runtime))
    try:
        manifest = inspect_archive(archive, root, staging)
        # The caller-created staging root remains private. Only its validated
        # children receive normalized Git executable-bit permissions.
        os.rename(staging, destination)
        write_json(runtime / "source-manifest.json", manifest)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return verify_source(root)


def expanded_archive(archive, maximum, output=None):
    result = hashlib.sha256()
    length = 0
    prefix = b""
    with gzip.open(archive, "rb") as stream:
        while chunk := stream.read(min(CHUNK, maximum - length + 1)):
            length += len(chunk)
            if length > maximum:
                fail("native archive expansion exceeds admission ceiling")
            prefix = (prefix + chunk)[:8]
            result.update(chunk)
            if output is not None:
                output.write(chunk)
    if prefix != b"!<arch>\n":
        fail("native asset is not a static archive")
    return {"bytes": length, "sha256": result.hexdigest()}


def prepare_assets(archive, binding, root=ROOT):
    pins = load_pins(root)
    v8 = pins["v8"]
    checked_file(archive, v8["archive"])
    checked_file(binding, v8["binding"])
    runtime = runtime_directory(root)
    destination = runtime / "artifacts"
    if destination.exists() or destination.is_symlink():
        return verify_assets(root)
    staging = Path(tempfile.mkdtemp(prefix="assets-staging-", dir=runtime))
    try:
        shutil.copyfile(archive, staging / v8["archive"]["name"])
        shutil.copyfile(binding, staging / v8["binding"]["name"])
        checked_file(staging / v8["archive"]["name"], v8["archive"])
        checked_file(staging / v8["binding"]["name"], v8["binding"])
        with (staging / "librusty_v8.a").open("xb") as output:
            expanded = expanded_archive(staging / v8["archive"]["name"], v8["expanded_maximum_bytes"], output)
        receipt = {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "expanded": expanded}
        write_json(staging / "assets.json", receipt)
        os.rename(staging, destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return verify_assets(root)


def verify_assets(root=ROOT):
    pins = load_pins(root)
    v8 = pins["v8"]
    directory = root / "runtime/artifacts"
    if directory.is_symlink() or not directory.is_dir():
        fail("missing regular assets directory")
    if {path.name for path in directory.iterdir()} != {v8["archive"]["name"], v8["binding"]["name"], "librusty_v8.a", "assets.json"}:
        fail("unexpected native asset roster")
    archive = checked_file(directory / v8["archive"]["name"], v8["archive"])
    checked_file(directory / v8["binding"]["name"], v8["binding"])
    expanded = expanded_archive(archive, v8["expanded_maximum_bytes"])
    checked_file(directory / "librusty_v8.a", expanded)
    expected = {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "expanded": expanded}
    if read_json(directory / "assets.json") != expected:
        fail("native asset receipt mismatch")
    return expected


def config_inputs(root=ROOT):
    paths = set()
    for parent in (root, *root.parents, Path.home()):
        for name in ("config", "config.toml"):
            path = parent / ".cargo" / name
            if path.exists() or path.is_symlink():
                paths.add(path)
    pins = load_pins(root)
    answer = {}
    for path in sorted(paths):
        if path.is_symlink() or not path.is_file():
            fail("non-regular Cargo configuration")
        checksum = file_hash(path)
        if checksum not in pins["cargo_config_sha256"]:
            fail("unreviewed effective Cargo configuration: " + str(path))
        answer[str(path)] = checksum
    return answer


def controlled_environment(root=ROOT, ambient=None):
    ambient = dict(os.environ if ambient is None else ambient)
    forbidden_prefixes = ("CARGO_", "RUST", "V8_", "GN", "CELLD_", "DYLD_", "LD_", "MACOSX_DEPLOYMENT_TARGET")
    forbidden = {"CC", "CXX", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "AR", "AS", "NM", "RANLIB", "SDKROOT", "CCACHE", "SCCACHE", "CLANG_BASE_PATH", "CXXSTDLIB", "LIBCLANG_PATH", "NINJA", "DOCS_RS", "DENO_TRYBUILD", "DISABLE_CLANG", "EXTRA_GN_ARGS", "PRINT_GN_ARGS", "PYTHON"}
    for name in ambient:
        if name in forbidden or name.startswith(forbidden_prefixes):
            fail("unreviewed build environment variable: " + name)
    pins = load_pins(root)
    toolchain = Path.home() / ".rustup/toolchains" / (pins["toolchain"]["channel"] + "-" + pins["toolchain"]["host"])
    for relative, expected in pins["toolchain"]["files"].items():
        path = toolchain / relative
        if path.is_symlink() or not path.is_file() or file_hash(path) != expected:
            fail("pinned toolchain file mismatch: " + str(path))
    environment = {name: ambient[name] for name in ("HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "TZ") if name in ambient}
    environment.update({
        "PATH": str(toolchain / "bin") + ":/usr/bin:/bin:/usr/sbin:/sbin",
        "TMPDIR": "/tmp", "CARGO_INCREMENTAL": "0", "CARGO_NET_OFFLINE": "true",
        "CARGO_TARGET_DIR": str(root / "runtime/target"),
        "RUSTY_V8_ARCHIVE": str(root / "runtime/artifacts/librusty_v8.a"),
        "RUSTY_V8_SRC_BINDING_PATH": str(root / "runtime/artifacts" / pins["v8"]["binding"]["name"]),
        "CC": pins["sdk"]["cc"], "CXX": pins["sdk"]["cxx"], "SDKROOT": pins["sdk"]["path"],
    })
    return toolchain, environment


def lock_inventory(path):
    value = tomllib.loads(Path(path).read_text())
    packages = {}
    for package in value.get("package", []):
        name, version = package.get("name"), package.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            fail("malformed lock package")
        identity = (name, version, package.get("source"))
        if identity in packages:
            fail("duplicate lock identity")
        if package.get("source") not in (None, CRATES_IO):
            fail("unreviewed registry or Git source")
        if package.get("source") and not re.fullmatch(r"[0-9a-f]{64}", package.get("checksum", "")):
            fail("missing registry checksum")
        packages[identity] = package
    return packages


def compare_locks(root=ROOT):
    pins = load_pins(root)
    before = lock_inventory(root / "runtime/source/Cargo.lock")
    after = lock_inventory(root / "Cargo.lock")
    before_registry = {key: value for key, value in before.items() if key[2] is not None}
    after_registry = {key: value for key, value in after.items() if key[2] is not None}
    for key in before_registry.keys() & after_registry.keys():
        if before_registry[key]["checksum"] != after_registry[key]["checksum"]:
            fail("retained upstream registry checksum changed")
    old_names = {key[0] for key in before_registry}
    added = after_registry.keys() - before_registry.keys()
    approved = pins["foks_additional_versions"]
    baseline_path = root.parents[1] / relative_name(approved["baseline_lock"])
    if baseline_path.is_symlink() or file_hash(baseline_path) != approved["baseline_lock_sha256"]:
        fail("reviewed FOKS baseline lock changed")
    baseline = lock_inventory(baseline_path)
    permitted = {}
    for item in approved["allowed"]:
        key = (item["name"], item["version"], item["source"])
        if key in permitted or key in before_registry or key not in baseline or baseline[key].get("checksum") != item["checksum"] or key[0] in pins["registry_engines"]:
            fail("invalid additional-version allowlist")
        permitted[key] = item["checksum"]
    for key in added:
        if key[0] in old_names and permitted.get(key) != after_registry[key]["checksum"]:
            fail("unreviewed additional upstream-name version")
    for name, expected in pins["registry_engines"].items():
        matches = [value for key, value in after_registry.items() if key[0] == name]
        if len(matches) != 1 or any(matches[0].get(key) != value for key, value in expected.items()):
            fail("engine lock identity mismatch: " + name)
    encode = lambda keys: [{"name": key[0], "version": key[1], "source": key[2]} for key in sorted(keys)]
    return {"retained": encode(before_registry.keys() & after_registry.keys()), "removed_unreachable_candidates": encode(before_registry.keys() - after_registry.keys()), "added": encode(added)}


def upstream_edges(metadata, reached, root=ROOT):
    """An allowed FOKS duplicate never replaces a selected upstream dependency."""
    before = lock_inventory(root / "runtime/source/Cargo.lock")
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    checked = []
    for identity in sorted(reached):
        package = packages[identity]
        key = (package["name"], package["version"], package["source"])
        if key not in before:
            continue
        expected = set()
        for description in before[key].get("dependencies", []):
            parts = description.split()
            if not 1 <= len(parts) <= 3:
                fail("unclassified upstream lock edge")
            candidates = [candidate for candidate in before if candidate[0] == parts[0] and (len(parts) < 2 or candidate[1] == parts[1]) and (len(parts) < 3 or candidate[2] == parts[2].strip("()"))]
            if len(candidates) != 1:
                fail("ambiguous upstream lock edge")
            expected.add(candidates[0])
        for dependency in nodes[identity]["deps"]:
            if not any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                continue
            target = packages[dependency["pkg"]]
            actual = (target["name"], target["version"], target["source"])
            if actual not in expected:
                fail("selected upstream dependency replaced or extended: " + package["name"] + " -> " + target["name"])
            checked.append({"from": identity, "to": dependency["pkg"]})
    return checked


def registry_inputs(metadata, reached, root=ROOT):
    """Check locked archives plus actual manifests/build sources before Cargo."""
    pins = load_pins(root)
    locked = lock_inventory(root / "Cargo.lock")
    registry = Path.home() / ".cargo/registry"
    records = []
    for package in metadata["packages"]:
        if package["id"] not in reached or package["source"] is None:
            continue
        key = (package["name"], package["version"], package["source"])
        entry = locked.get(key)
        if entry is None or key[2] != CRATES_IO:
            fail("resolved registry package missing from lock")
        manifest = Path(package["manifest_path"]).resolve()
        directory = manifest.parent
        if not directory.is_relative_to(registry / "src") or directory.name != key[0] + "-" + key[1]:
            fail("registry manifest outside canonical cache")
        cache = registry / "cache" / directory.parent.name / (directory.name + ".crate")
        if file_hash(cache) != entry["checksum"]:
            fail("cached registry archive checksum mismatch")
        actual = tomllib.loads(manifest.read_text())
        declared = actual.get("package", {}).get("rust-version")
        if declared is not None:
            if not re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", declared):
                fail("unclassified dependency MSRV")
            if tuple(map(int, declared.split("."))) > (1, 95, 0):
                fail("dependency requires a newer compiler: " + key[0])
        build = actual.get("package", {}).get("build", "build.rs")
        requested = {"Cargo.toml", "Cargo.toml.orig"}
        if build is not False:
            relative_name(build)
            if (directory / build).exists():
                requested.add(build)
        verified = {}
        roster = {}
        with tarfile.open(cache, "r|gz") as archive:
            for member in archive:
                prefix = directory.name + "/"
                if not member.name.startswith(prefix):
                    fail("registry archive root mismatch")
                relative = relative_name(member.name[len(prefix):].rstrip("/"))
                if member.isdir():
                    continue
                if not member.isfile() or member.size > 256 * 1024 * 1024 or relative in roster:
                    fail("invalid registry source entry")
                path = directory / relative
                if path.is_symlink() or not path.is_file() or path.stat().st_size != member.size:
                    fail("extracted registry source type/length mismatch")
                archived = archive.extractfile(member)
                checksum = hashlib.sha256()
                remaining = member.size
                with path.open("rb") as actual_source:
                    while remaining:
                        chunk = archived.read(min(CHUNK, remaining))
                        if not chunk or actual_source.read(len(chunk)) != chunk:
                            fail("extracted registry source differs from archive")
                        remaining -= len(chunk)
                        checksum.update(chunk)
                    if actual_source.read(1):
                        fail("registry source changed during verification")
                roster[relative] = {"bytes": member.size, "sha256": checksum.hexdigest()}
                if relative in requested:
                    verified[relative] = checksum.hexdigest()
        actual_roster = set()
        for parent, directories, names in os.walk(directory, followlinks=False):
            for name in [*directories, *names]:
                path = Path(parent) / name
                if path.is_symlink():
                    fail("symlink in extracted registry source")
                if path.is_file():
                    actual_roster.add(path.relative_to(directory).as_posix())
        if actual_roster - {".cargo-ok", ".cargo-checksum.json"} != set(roster):
            fail("extracted registry source roster differs from archive")
        if "Cargo.toml" not in verified or any(name not in verified for name in requested if (directory / name).exists()):
            fail("registry build input missing from archive")
        if key[0] == "v8" and (verified.get("build.rs") != pins["v8"]["build_rs_sha256"] or verified.get("Cargo.toml.orig") != pins["v8"]["cargo_toml_orig_sha256"]):
            fail("V8 build source differs from reviewed immutable source")
        records.append({"id": package["id"], "archive": str(cache), "archive_bytes": cache.stat().st_size, "extracted_bytes": sum(item["bytes"] for item in roster.values()), "checksum": entry["checksum"], "source_roster_sha256": digest(json.dumps(roster, sort_keys=True).encode()), "rust_version": declared, "manifest_and_build_sources": verified})
    return sorted(records, key=lambda item: item["id"])


def directory_bytes(path):
    path = Path(path)
    try:
        root_status = path.lstat()
    except FileNotFoundError:
        return 0
    if stat.S_ISLNK(root_status.st_mode):
        fail("symlink at measured build target")

    def walk_error(error):
        # Cargo can remove an enumerated directory while this live scan runs.
        # An inaccessible or otherwise unobservable tree is not an empty tree.
        if not isinstance(error, FileNotFoundError):
            raise error

    total = 0
    for parent, directories, names in os.walk(path, followlinks=False, onerror=walk_error):
        for name in [*directories, *names]:
            item = Path(parent) / name
            try:
                status = item.lstat()
            except FileNotFoundError:
                continue
            if stat.S_ISLNK(status.st_mode):
                fail("symlink in measured build target")
            if stat.S_ISREG(status.st_mode):
                total += status.st_size
    return total


def disk_status(root=ROOT):
    return {"free_bytes": shutil.disk_usage(root).free, "target_bytes": directory_bytes(root / "runtime/target")}


def resource_refusal(root, initial=False):
    pins = load_pins(root)["build"]
    measured = disk_status(root)
    floor = pins["minimum_free_bytes"] if initial else pins["free_floor_bytes"]
    if measured["free_bytes"] < floor:
        return "free-space admission refusal", measured
    if measured["target_bytes"] > pins["maximum_target_bytes"]:
        return "target working allowance exceeded", measured
    return None, measured


class BuildSlot:
    """A stale or uncertain slot is inspected by root; never automatically stolen."""
    def __init__(self, root=ROOT):
        self.path = runtime_directory(root) / "build-slot.json"
        self.clean = True
        self.state = {"format": 1, "wrapper_pid": os.getpid(), "status": "held", "groups": []}

    def __enter__(self):
        with self.path.open("x") as stream:
            json.dump(self.state, stream)
        return self

    def started(self, pid):
        self.clean = False
        self.state["groups"].append(pid)
        write_json(self.path, self.state)

    def cleaned(self):
        self.clean = True
        self.state["status"] = "held; last group cleaned"
        write_json(self.path, self.state)

    def __exit__(self, *_):
        if self.clean:
            self.path.unlink()
        else:
            self.state["status"] = "cleanup uncertain; slot quarantined"
            write_json(self.path, self.state)


def process_table():
    result = subprocess.run(["/bin/ps", "-axo", "pid=,ppid=,pgid="], capture_output=True, timeout=3, check=False)
    if result.returncode:
        fail("cannot observe build child cleanup")
    entries = {}
    for line in result.stdout.decode().splitlines():
        fields = line.split()
        if len(fields) != 3 or any(not value.isdecimal() for value in fields):
            fail("unclassified process-table row")
        pid, parent, group = map(int, fields)
        entries[pid] = (parent, group)
    return entries


def group_alive(group):
    try:
        os.killpg(group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        # Darwin can transiently deny a probe while a signalled child exits.
        # Unknown presence is still charged; only ESRCH establishes absence.
        return True


def observe_children(group, known):
    table = process_table()
    changed = True
    while changed:
        changed = False
        for pid, (parent, candidate_group) in table.items():
            if (parent in known or candidate_group == group) and pid not in known:
                known.add(pid)
                changed = True
    return sorted(pid for pid in known if pid in table and table[pid][1] != group)


def signal_group(group, selected, events):
    try:
        os.killpg(group, selected)
        events.append({"signal": selected.name, "group": group})
    except ProcessLookupError:
        events.append({"signal": selected.name, "group": group, "already_gone": True})
    except PermissionError:
        # Retain the bounded cleanup wait and eventual quarantine if access
        # remains denied. A failed signal never establishes disappearance.
        events.append({"signal": selected.name, "group": group, "permission_denied": True})


def finish_group(process, known, events, term_grace, kill_grace):
    """Parent wait plus absent process group, never parent exit alone."""
    group = process.pid
    for selected, grace in ((signal.SIGTERM, term_grace), (signal.SIGKILL, kill_grace)):
        if group_alive(group):
            signal_group(group, selected, events)
        until = time.monotonic() + grace
        while True:
            process.poll()  # reap the owned parent if it has exited
            escaped = observe_children(group, known)
            if not group_alive(group):
                return process.poll() is not None and not escaped, escaped
            if time.monotonic() >= until:
                break
            time.sleep(0.05)
    return False, observe_children(group, known)


def run_owned(argv, cwd, environment, directory, name, slot, timeout, monitor=None, term_grace=3, kill_grace=3):
    """One process group, direct argv, bounded wait and durable failure evidence."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    record = {"argv": [str(arg) for arg in argv], "cwd": str(cwd), "timeout_seconds": timeout, "events": [], "cleanup_established": False}
    process = None
    known = set()
    cancellation = []
    saved_handlers = {}
    started = time.monotonic()
    result_path = directory / (name + ".command.json")
    try:
        for selected in (signal.SIGINT, signal.SIGTERM):
            saved_handlers[selected] = signal.signal(selected, lambda received, _frame: cancellation.append(received))
        with (directory / (name + ".stdout")).open("xb") as stdout, (directory / (name + ".stderr")).open("xb") as stderr:
            process = subprocess.Popen(record["argv"], cwd=cwd, env=environment, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, start_new_session=True)
            known.add(process.pid)
            slot.started(process.pid)
            record["process_group"] = process.pid
            write_json(result_path, record)
            deadline = started + timeout
            parent_done_at = None
            while True:
                escaped = observe_children(process.pid, known)
                if escaped:
                    record["reason"] = "observed escaped children"
                    record["escaped_children"] = escaped
                    break
                if cancellation:
                    record["reason"] = "wrapper cancelled"
                    break
                if time.monotonic() >= deadline:
                    record["reason"] = "command timeout"
                    break
                if monitor:
                    refusal, measured = monitor()
                    record["last_resources"] = measured
                    if refusal:
                        record["reason"] = refusal
                        break
                if stdout.tell() + stderr.tell() > 128 * 1024 * 1024:
                    record["reason"] = "command log allowance exceeded"
                    break
                status = process.poll()
                if status is not None:
                    if not group_alive(process.pid):
                        record["reason"] = "completed" if status == 0 else "command failed"
                        break
                    if parent_done_at is None:
                        parent_done_at = time.monotonic()
                    if time.monotonic() - parent_done_at >= term_grace:
                        record["reason"] = "parent exited with outstanding children"
                        break
                time.sleep(0.1)
    except BaseException as error:
        record["reason"] = "wrapper exception: " + type(error).__name__
        record["error"] = str(error)
    finally:
        try:
            if process is not None:
                try:
                    clean, escaped = finish_group(process, known, record["events"], term_grace, kill_grace)
                    record["cleanup_established"] = clean and not record.get("escaped_children")
                    record["remaining_escaped_children"] = escaped
                    record["exit_code"] = process.poll()
                    if record["cleanup_established"]:
                        slot.cleaned()
                except BaseException as error:
                    record["cleanup_error"] = str(error)
            else:
                record["cleanup_established"] = True
            record["elapsed_seconds"] = time.monotonic() - started
            for stream in ("stdout", "stderr"):
                path = directory / (name + "." + stream)
                if path.exists():
                    record[stream + "_sha256"] = file_hash(path)
            write_json(result_path, record)
        finally:
            # Evidence failure must not leave cancellation handlers installed
            # after the owned group has been cleaned or quarantined.
            for selected, handler in saved_handlers.items():
                signal.signal(selected, handler)
    if not record["cleanup_established"]:
        fail("cleanup uncertain; sole Cargo slot remains quarantined")
    if record.get("reason") != "completed":
        fail(record.get("reason", "command did not complete"))
    return record


def network_command(argv):
    sandbox = Path("/usr/bin/sandbox-exec")
    if not sandbox.is_file():
        fail("reviewed network-denial launcher unavailable")
    return [str(sandbox), "-p", "(version 1) (allow default) (deny network*)", *map(str, argv)]


def identity_commands(root, toolchain):
    pins = load_pins(root)
    return {
        "rustc-version": [toolchain / "bin/rustc", "-Vv"],
        "cargo-version": [toolchain / "bin/cargo", "-Vv"],
        "sdk-version": ["/usr/bin/xcrun", "--show-sdk-version"],
        "clang-version": [pins["sdk"]["cc"], "--version"],
        "foks-revision": ["/usr/bin/git", "-C", str(root.parents[1]), "rev-parse", "HEAD"],
        "foks-status": ["/usr/bin/git", "-C", str(root.parents[1]), "status", "--porcelain=v1", "--untracked-files=all"],
    }


def native_tool_paths(root):
    pins = load_pins(root)
    return (
        pins["sdk"]["cc"], pins["sdk"]["cxx"],
        "/Library/Developer/CommandLineTools/usr/bin/ld",
        pins["sdk"]["path"] + "/SDKSettings.json",
        "/usr/bin/xcrun", "/usr/bin/sandbox-exec",
    )


def tool_identity(root, directory, slot, environment, toolchain):
    pins = load_pins(root)
    result = {}
    for name, argv in identity_commands(root, toolchain).items():
        result[name] = run_owned(network_command(argv), root, environment, directory, name, slot, 15)
    compiler = (directory / "rustc-version.stdout").read_text()
    for expected in ("release: " + pins["toolchain"]["channel"], "commit-hash: " + pins["toolchain"]["rustc_commit"], "host: " + pins["toolchain"]["host"], "LLVM version: " + pins["toolchain"]["llvm"]):
        if expected not in compiler.splitlines():
            fail("actual compiler differs from pin")
    if (directory / "sdk-version.stdout").read_text().strip() != pins["sdk"]["version"]:
        fail("actual SDK differs from pin")
    result["native_tools"] = {path: file_hash(path) for path in native_tool_paths(root)}
    result["environment"] = environment
    result["cargo_configs"] = config_inputs(root)
    return result


def build(graph_directory, root=ROOT):
    import check_graph
    pins = load_pins(root)
    source = verify_source(root)
    assets = verify_assets(root)
    graph = check_graph.verify(graph_directory, root)
    refusal, measured = resource_refusal(root, initial=True)
    if refusal:
        fail(refusal + ": " + json.dumps(measured))
    toolchain, environment = controlled_environment(root)
    evidence_root = runtime_directory(root) / "evidence"
    evidence_root.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="build-", dir=evidence_root))
    receipt = {"format": 1, "claim": "a1-library-identity-only", "status": "incomplete", "source": source, "assets": assets, "graph_directory": str(Path(graph_directory).resolve()), "graph_sha256": digest(json.dumps(graph, sort_keys=True).encode()), "initial_resources": measured}
    write_json(directory / "receipt.json", receipt)
    with BuildSlot(root) as slot:
        try:
            receipt["tools"] = tool_identity(root, directory, slot, environment, toolchain)
            cargo = toolchain / "bin/cargo"
            shared = ["--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", "-j", "2", "--target", TARGET, "--bin", "foks-celld-guarded"]
            for name, arguments in (("build", ["build", *shared]), ("clippy", ["clippy", *shared, "--", "-D", "warnings"])):
                receipt[name] = run_owned(network_command([cargo, *arguments]), root, environment, directory, name, slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
            binary = root / "runtime/target" / TARGET / "debug/foks-celld-guarded"
            receipt["binary_sha256"] = file_hash(binary)
            receipt["identity"] = run_owned(network_command([binary, "identity"]), root, environment, directory, "identity", slot, pins["build"]["identity_timeout_seconds"], lambda: resource_refusal(root))
            if file_hash(binary) != receipt["binary_sha256"]:
                fail("binary changed during identity probe")
            identity = read_json(directory / "identity.stdout", 64 * 1024)
            if identity.get("claim") != "a1-library-identity-only" or identity.get("input_sha256") != file_hash(root / "runtime.lock.json") or identity.get("actual_shared_request_exercised") is not True:
                fail("identity output is not bound to the actual probe")
            if not identity.get("v8_engine") or not identity.get("sqlite_source_id") or not identity.get("sqlite_version"):
                fail("missing actual engine identity")
            # Bind the source/config/registry inputs both before and after the
            # commands; a successful compiler exit cannot bless changed inputs.
            if verify_source(root) != source or verify_assets(root) != assets or check_graph.verify(graph_directory, root) != graph:
                fail("build inputs changed during qualification")
            if config_inputs(root) != receipt["tools"]["cargo_configs"]:
                fail("Cargo configuration changed during build")
            if any(file_hash(path) != expected for path, expected in receipt["tools"]["native_tools"].items()):
                fail("native compiler/linker/SDK inputs changed during build")
            controlled_environment(root)
            receipt["identity_output"] = identity
            receipt["status"] = "passed"
        except BaseException as error:
            receipt["error"] = str(error)
            receipt["status"] = "failed" if slot.clean else "cleanup uncertain"
            raise
        finally:
            receipt["final_resources"] = disk_status(root)
            write_json(directory / "receipt.json", receipt)
    return {"directory": str(directory), "receipt_sha256": file_hash(directory / "receipt.json"), "status": receipt["status"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    for name in ("verify", "prepare"):
        sub.add_parser(name).add_argument("archive", type=Path)
    assets = sub.add_parser("assets")
    assets.add_argument("archive", type=Path)
    assets.add_argument("binding", type=Path)
    sub.add_parser("verify-source")
    sub.add_parser("verify-assets")
    sub.add_parser("build").add_argument("graph_directory", type=Path)
    args = parser.parse_args()
    try:
        if args.operation == "verify":
            report = inspect_archive(args.archive)
        elif args.operation == "prepare":
            report = prepare(args.archive)
        elif args.operation == "assets":
            report = prepare_assets(args.archive, args.binding)
        elif args.operation == "verify-source":
            report = verify_source()
        elif args.operation == "verify-assets":
            report = verify_assets()
        else:
            report = build(args.graph_directory)
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, TypeError, KeyError, EOFError, tarfile.TarError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

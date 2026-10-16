#!/usr/bin/env python3
"""Prepare and qualify pinned A1/A2 inputs; no command downloads dependencies.

verify checks only the original upstream archive. prepare also applies the closed
A2 patch in private staging. assets accepts the two reviewed local V8 assets.
Graph collection and build are explicit later actions after source/lock review,
and share one fail-closed process-group build slot.
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
A1 = "a1-library-identity"
A2 = "a2-physical-lifetimes"
A2_BASE = "8e31944080e03651caaa3512169fe468f8375342"
PATCH_PATH = "patches/0001-guarded-physical-lifetimes.patch"
PATCH_PATHS = (
    "crates/celld/Cargo.toml", "crates/celld/lib.rs", "crates/celld/pool.rs",
    "crates/celld/runtime.rs", "crates/celld/js.rs", "crates/celld/js/guarded.rs",
    "crates/celld/guarded_execution.rs", "crates/celld/guarded_execution/qualification.rs",
)
PATCH_ADDITIONS = frozenset(PATCH_PATHS[-3:])
MAXIMUM_PATCH = 1024 * 1024
MAXIMUM_POSTIMAGE = 512 * 1024
MAXIMUM_POSTIMAGES = 2 * 1024 * 1024


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
    def unique_fields(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                fail("duplicate JSON field: " + key)
            value[key] = item
        return value
    return json.loads(path.read_text(), object_pairs_hook=unique_fields)


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
    if not isinstance(pins, dict) or (pins.get("format"), pins.get("profile")) not in ((1, A1), (2, A2)) or type(pins.get("format")) is not int or pins.get("target") != TARGET:
        fail("unsupported input profile")
    expected = {"format", "profile", "target", "foks_baseline", "upstream", "patch_series_sha256", "toolchain", "v8", "registry_engines", "build", "cargo_config_sha256", "sdk", "foks_additional_versions"}
    if pins["profile"] == A2:
        expected.add("qualification")
    if set(pins) != expected:
        fail("unrecognized input-record field")
    if pins["profile"] == A2:
        qualification = pins["qualification"]
        if not isinstance(qualification, dict) or set(qualification) != {"base_commit", "program", "execution_slots", "wasm_bytes_per_slot"} or qualification["base_commit"] != A2_BASE:
            fail("invalid A2 qualification identity")
        if type(qualification["execution_slots"]) is not int or qualification["execution_slots"] != 2 or type(qualification["wasm_bytes_per_slot"]) is not int or qualification["wasm_bytes_per_slot"] != 64 * 1024 * 1024:
            fail("invalid A2 physical capacity")
        program = qualification["program"]
        if not isinstance(program, dict) or set(program) != {"js", "wasm"}:
            fail("invalid A2 program inventory")
        for value in program.values():
            validate_image(value, git=False, positive=True)
        if pins["build"].get("admission") != "remaining-target-allowance":
            fail("invalid A2 build admission policy")
        for name, value in (("minimum_free_bytes", 6 * 1024**3), ("free_floor_bytes", 2 * 1024**3), ("maximum_target_bytes", 4 * 1024**3)):
            if type(pins["build"].get(name)) is not int or pins["build"][name] != value:
                fail("A2 build allowance changed")
    elif "admission" in pins["build"]:
        fail("A1 build admission policy changed")
    patch_series(root, pins)
    return pins


def validate_image(value, git=True, positive=False):
    keys = {"bytes", "sha256", "git_sha1"} if git else {"bytes", "sha256"}
    if not isinstance(value, dict) or set(value) != keys or type(value.get("bytes")) is not int or not (int(positive) <= value["bytes"] <= MAXIMUM_POSTIMAGE):
        fail("invalid image size/fields")
    hashes = (("sha256", 64), ("git_sha1", 40)) if git else (("sha256", 64),)
    for name, length in hashes:
        if not isinstance(value[name], str) or not re.fullmatch(r"[0-9a-f]{" + str(length) + "}", value[name]):
            fail("invalid image hash")


def patch_series(root, pins):
    path = root / "patches/series.json"
    series = read_json(path)
    if file_hash(path) != pins["patch_series_sha256"]:
        fail("patch series hash mismatch")
    if pins["profile"] == A1:
        if series != {"format": 1, "patches": []} or type(series.get("format")) is not int:
            fail("A1 requires the exact empty patch series")
        return series
    if not isinstance(series, dict) or set(series) != {"format", "profile", "base_commit", "patches"} or type(series["format"]) is not int or series["format"] != 2 or series["profile"] != A2 or series["base_commit"] != A2_BASE:
        fail("invalid A2 patch series identity")
    patches = series["patches"]
    if not isinstance(patches, list) or len(patches) != 1:
        fail("A2 requires exactly one patch")
    selected = patches[0]
    if not isinstance(selected, dict) or set(selected) != {"path", "bytes", "sha256", "files"} or selected["path"] != PATCH_PATH or type(selected["bytes"]) is not int or not 0 < selected["bytes"] <= MAXIMUM_PATCH or not isinstance(selected["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", selected["sha256"]):
        fail("invalid A2 patch envelope")
    files = selected["files"]
    if not isinstance(files, list) or len(files) != len(PATCH_PATHS):
        fail("invalid A2 patch file roster")
    seen = set()
    total = 0
    for entry in files:
        if not isinstance(entry, dict) or set(entry) != {"path", "mode", "before", "after"} or entry.get("path") not in PATCH_PATHS or entry["path"] in seen or entry["mode"] != "100644":
            fail("invalid A2 patch path/mode")
        seen.add(entry["path"])
        if entry["path"] in PATCH_ADDITIONS:
            if entry["before"] is not None:
                fail("addition must have no preimage")
        else:
            validate_image(entry["before"])
        validate_image(entry["after"])
        total += entry["after"]["bytes"]
    if total > MAXIMUM_POSTIMAGES:
        fail("A2 patch postimages exceed allowance")
    checked_file(root / PATCH_PATH, selected)
    return series


def selected_mode(pins, mode):
    if mode not in ("identity", "lifecycle") or (mode == "lifecycle" and pins["profile"] != A2):
        fail("unsupported graph/build mode for input profile")
    return [] if mode == "identity" else ["--features", "lifecycle"]


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


def image_identity(data):
    return {"bytes": len(data), "sha256": digest(data), "git_sha1": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()}


def parse_patch(root, pins, upstream):
    """A closed diff grammar; no context search, offset guessing or external tool."""
    if pins["profile"] == A1:
        return []
    selected = patch_series(root, pins)["patches"][0]
    data = checked_file(root / PATCH_PATH, selected).read_bytes()
    if len(data) != selected["bytes"] or digest(data) != selected["sha256"]:
        fail("patch changed while reading")
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        fail("patch is not UTF-8")
    if not data.endswith(b"\n") or b"\r" in data:
        fail("patch requires LF and terminal newline")
    # Split on LF alone; Unicode line separators are ordinary source bytes.
    lines = [line + b"\n" for line in data[:-1].split(b"\n")]
    cursor = 0
    parsed = []
    for entry in selected["files"]:
        path = entry["path"]
        original = upstream.get(path)
        if entry["before"] is None:
            if original is not None:
                fail("patch addition already exists upstream")
            before_header = b"--- /dev/null\n"
        else:
            if original is None or original["type"] != "blob" or original["mode"] != entry["mode"] or original["size"] != entry["before"]["bytes"] or original["sha"] != entry["before"]["git_sha1"]:
                fail("patch preimage does not identify the immutable upstream blob")
            before_header = ("--- a/" + path + "\n").encode()
        if lines[cursor:cursor + 2] != [before_header, ("+++ b/" + path + "\n").encode()]:
            fail("patch file headers/order mismatch")
        cursor += 2
        hunks = []
        while cursor < len(lines) and lines[cursor].startswith(b"@@"):
            match = re.fullmatch(rb"@@ -([0-9]+),([0-9]+) \+([0-9]+),([0-9]+) @@\n", lines[cursor])
            if match is None:
                fail("invalid explicit patch hunk header")
            old_start, old_count, new_start, new_count = map(int, match.groups())
            if (old_count and old_start == 0) or (new_count and new_start == 0):
                fail("invalid nonempty patch hunk origin")
            old_index = old_start - bool(old_count)
            new_index = new_start - bool(new_count)
            cursor += 1
            before, after = [], []
            while len(before) < old_count or len(after) < new_count:
                if cursor >= len(lines) or lines[cursor][:1] not in (b" ", b"-", b"+"):
                    fail("truncated or unsupported patch hunk body")
                line = lines[cursor]
                cursor += 1
                if line[:1] in (b" ", b"-"):
                    before.append(line[1:])
                if line[:1] in (b" ", b"+"):
                    after.append(line[1:])
                if len(before) > old_count or len(after) > new_count:
                    fail("patch hunk line counts mismatch")
            if not old_count and not new_count:
                fail("empty patch hunk")
            hunks.append((old_index, new_index, before, after))
        if not hunks:
            fail("patch file has no hunks")
        parsed.append((entry, hunks))
    if cursor != len(lines):
        fail("unused or unsupported trailing patch content")
    return parsed


def transform_image(data, hunks, reverse=False):
    """Apply only the stated line positions, preserving every gap and tail."""
    if data and not data.endswith(b"\n"):
        fail("patched source requires a terminal newline")
    lines = [] if not data else [line + b"\n" for line in data[:-1].split(b"\n")]
    result = []
    cursor = 0
    for old_index, new_index, before, after in hunks:
        if reverse:
            old_index, new_index, before, after = new_index, old_index, after, before
        if old_index < cursor or old_index > len(lines):
            fail("patch hunk overlaps or exceeds source")
        gap = old_index - cursor
        if new_index != len(result) + gap:
            fail("patch hunk output offset mismatch")
        result.extend(lines[cursor:old_index])
        if lines[old_index:old_index + len(before)] != before:
            fail("patch context/removal mismatch")
        result.extend(after)
        cursor = old_index + len(before)
    result.extend(lines[cursor:])
    return b"".join(result)


def postimage_inventory(upstream, patches):
    expected = {name: dict(entry) for name, entry in upstream.items()}
    for entry, _ in patches:
        name = entry["path"]
        expected[name] = {"path": name, "mode": entry["mode"], "type": "blob", "size": entry["after"]["bytes"], "sha": entry["after"]["git_sha1"]}
        parent = name.rpartition("/")[0]
        while parent:
            if parent in expected:
                if expected[parent]["type"] != "tree":
                    fail("patch parent is an upstream file")
            elif name in PATCH_ADDITIONS:
                expected[parent] = {"path": parent, "mode": "040000", "type": "tree"}
            else:
                fail("patch changes an absent parent")
            parent = parent.rpartition("/")[0]
    return expected


def source_manifest(root, pins, files, prepared):
    value = {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "archive_sha256": pins["upstream"]["archive_sha256"], "tree": pins["upstream"]["tree"], "patch_series_sha256": pins["patch_series_sha256"], "files": sorted(files, key=lambda item: item["path"])}
    if pins["profile"] == A2:
        value.update(format=2, profile=A2, kind="patched-source" if prepared else "upstream-archive-only")
    return value


def apply_patch_set(staging, patches):
    # Validate every complete preimage/postimage before writing any changed file.
    # A failed write still remains in the disposable private staging directory.
    outputs = []
    for entry, hunks in patches:
        path = staging / entry["path"]
        before = b"" if entry["before"] is None else path.read_bytes()
        if entry["before"] is not None and image_identity(before) != entry["before"]:
            fail("patch full preimage mismatch")
        after = transform_image(before, hunks)
        if image_identity(after) != entry["after"]:
            fail("patch full postimage mismatch")
        outputs.append((path, after, entry["before"] is None))
    for path, after, addition in outputs:
        missing = []
        parent = path.parent
        while not parent.exists():
            missing.append(parent)
            parent = parent.parent
        path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        for parent in missing:
            parent.chmod(0o755)
        with path.open("xb" if addition else "wb") as stream:
            stream.write(after)
        path.chmod(0o644)


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
    return source_manifest(root, pins, manifest, prepared=False)


def runtime_directory(root=ROOT):
    path = root / "runtime"
    if path.is_symlink():
        fail("runtime directory is a symlink")
    path.mkdir(mode=0o700, exist_ok=True)
    if not path.is_dir():
        fail("runtime path is not a directory")
    return path


def inspect_source_tree(source, root, pins, expected, patches):
    if source.is_symlink() or not source.is_dir():
        fail("missing regular prepared source directory")
    seen = set()
    files = []
    changed = {entry["path"]: (entry, hunks) for entry, hunks in patches}
    def walk_error(error):
        raise error
    for parent, directories, names in os.walk(source, followlinks=False, onerror=walk_error):
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
            identity = {"bytes": entry["size"], "sha256": sha.hexdigest(), "git_sha1": entry["sha"]}
            if name in changed:
                change, hunks = changed[name]
                if identity != change["after"]:
                    fail("prepared patch postimage mismatch")
                after = path.read_bytes()
                if image_identity(after) != identity:
                    fail("prepared patch changed while reading")
                before = transform_image(after, hunks, reverse=True)
                if (change["before"] is None and before) or (change["before"] is not None and image_identity(before) != change["before"]):
                    fail("prepared patch does not reconstruct its upstream preimage")
                if transform_image(before, hunks) != after:
                    fail("prepared patch roundtrip mismatch")
            files.append({"path": name, "mode": entry["mode"], **identity})
    if seen != set(expected):
        fail("prepared source roster mismatch")
    if file_hash(source / "Cargo.lock") != pins["upstream"]["cargo_lock_sha256"]:
        fail("upstream lockfile changed")
    return source_manifest(root, pins, files, prepared=True)


def verify_source(root=ROOT):
    pins = load_pins(root)
    upstream = inventory(root, pins)
    patches = parse_patch(root, pins, upstream)
    expected = postimage_inventory(upstream, patches)
    runtime = runtime_directory(root)
    recorded = read_json(runtime / "source-manifest.json")
    actual = inspect_source_tree(runtime / "source", root, pins, expected, patches)
    if recorded != actual:
        fail("prepared source manifest mismatch")
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
        original = inspect_archive(archive, root, staging)
        pins = load_pins(root)
        if file_hash(root / "runtime.lock.json") != original["input_sha256"]:
            fail("input changed during archive preparation")
        upstream = inventory(root, pins)
        patches = parse_patch(root, pins, upstream)
        expected = postimage_inventory(upstream, patches)
        apply_patch_set(staging, patches)
        manifest = inspect_source_tree(staging, root, pins, expected, patches)
        if manifest["input_sha256"] != original["input_sha256"]:
            fail("input changed during patch preparation")
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
    if not stat.S_ISDIR(root_status.st_mode):
        fail("measured build target is not a directory")

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
            elif not stat.S_ISDIR(status.st_mode):
                fail("special file in measured build target")
    return total


def disk_status(root=ROOT):
    return {"free_bytes": shutil.disk_usage(root).free, "target_bytes": directory_bytes(root / "runtime/target")}


def resource_refusal(root, initial=False):
    inputs = load_pins(root)
    pins = inputs["build"]
    measured = disk_status(root)
    if not isinstance(measured, dict) or set(measured) != {"free_bytes", "target_bytes"} or any(type(value) is not int or value < 0 for value in measured.values()):
        fail("unknown build resource observation")
    if inputs["profile"] == A2 and measured["target_bytes"] > pins["maximum_target_bytes"]:
        return "target working allowance exceeded", measured
    floor = pins["minimum_free_bytes"] if initial else pins["free_floor_bytes"]
    if initial and inputs["profile"] == A2:
        floor = pins["free_floor_bytes"] + max(0, pins["maximum_target_bytes"] - measured["target_bytes"])
        measured = {**measured, "required_free_bytes": floor}
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


def build_commands(root, toolchain, pins, mode="identity"):
    features = selected_mode(pins, mode)
    shared = ["--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", *features, "-j", "2", "--target", TARGET]
    binary = ["--bin", "foks-celld-guarded"]
    cargo = toolchain / "bin/cargo"
    commands = {
        "build": network_command([cargo, "build", *shared, *binary]),
        "clippy": network_command([cargo, "clippy", *shared, *(["--all-targets"] if mode == "lifecycle" else binary), "--", "-D", "warnings"]),
    }
    if mode == "lifecycle":
        commands["lifecycle"] = network_command([cargo, "test", *shared, "--test", "lifecycle", "--", "--test-threads=1", "--nocapture"])
    return commands


def build(graph_directory, root=ROOT, mode="identity"):
    import check_graph
    pins = load_pins(root)
    selected_mode(pins, mode)
    source = verify_source(root)
    assets = verify_assets(root)
    graph = check_graph.verify(graph_directory, root, mode)
    refusal, measured = resource_refusal(root, initial=True)
    if refusal:
        fail(refusal + ": " + json.dumps(measured))
    toolchain, environment = controlled_environment(root)
    evidence_root = runtime_directory(root) / "evidence"
    evidence_root.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="build-", dir=evidence_root))
    identity_claim = "a1-library-identity-only" if pins["profile"] == A1 else "a2-library-identity-only"
    receipt = {"format": 1, "profile": pins["profile"], "mode": mode, "claim": identity_claim if mode == "identity" else "a2-physical-lifetimes-local-fixture", "status": "incomplete", "source": source, "assets": assets, "graph_directory": str(Path(graph_directory).resolve()), "graph_sha256": digest(json.dumps(graph, sort_keys=True).encode()), "initial_resources": measured}
    if pins["profile"] == A2:
        receipt["qualification"] = pins["qualification"]
    write_json(directory / "receipt.json", receipt)
    with BuildSlot(root) as slot:
        try:
            receipt["tools"] = tool_identity(root, directory, slot, environment, toolchain)
            commands = build_commands(root, toolchain, pins, mode)
            for name in ("build", "clippy"):
                receipt[name] = run_owned(commands[name], root, environment, directory, name, slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
            binary = root / "runtime/target" / TARGET / "debug/foks-celld-guarded"
            receipt["binary_sha256"] = file_hash(binary)
            receipt["identity"] = run_owned(network_command([binary, "identity"]), root, environment, directory, "identity", slot, pins["build"]["identity_timeout_seconds"], lambda: resource_refusal(root))
            if file_hash(binary) != receipt["binary_sha256"]:
                fail("binary changed during identity probe")
            identity = read_json(directory / "identity.stdout", 64 * 1024)
            if identity.get("claim") != identity_claim or identity.get("input_sha256") != file_hash(root / "runtime.lock.json") or identity.get("actual_shared_request_exercised") is not True:
                fail("identity output is not bound to the actual probe")
            if not identity.get("v8_engine") or not identity.get("sqlite_source_id") or not identity.get("sqlite_version"):
                fail("missing actual engine identity")
            if mode == "lifecycle":
                # The reviewed integration target owns/drains closed child
                # scenarios and validates their bounded, input-bound reports.
                # Children inherit this group; a parent exit is never cleanup.
                receipt["lifecycle"] = run_owned(commands["lifecycle"], root, environment, directory, "lifecycle", slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
                if file_hash(binary) != receipt["binary_sha256"]:
                    fail("binary changed during lifecycle qualification")
            # Bind the source/config/registry inputs both before and after the
            # commands; a successful compiler exit cannot bless changed inputs.
            if verify_source(root) != source or verify_assets(root) != assets or check_graph.verify(graph_directory, root, mode) != graph:
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
        help_text = "validate only the immutable upstream archive" if name == "verify" else "validate archive and publish the reviewed prepared postimage"
        sub.add_parser(name, help=help_text).add_argument("archive", type=Path)
    assets = sub.add_parser("assets")
    assets.add_argument("archive", type=Path)
    assets.add_argument("binding", type=Path)
    sub.add_parser("verify-source")
    sub.add_parser("verify-assets")
    build_parser = sub.add_parser("build")
    build_parser.add_argument("graph_directory", type=Path)
    build_parser.add_argument("--mode", choices=("identity", "lifecycle"), default="identity")
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
            report = build(args.graph_directory, mode=args.mode)
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, TypeError, KeyError, EOFError, tarfile.TarError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Preparation/resource regressions. No Cargo or V8 build; only owned local network-policy helpers."""
import copy
from contextlib import contextmanager
import errno
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("preparer_under_test", ROOT / "prepare-runtime.py")
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


def blob(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def tree(children):
    body = b"".join(mode.encode() + b" " + name.encode() + b"\0" + bytes.fromhex(checksum) for name, mode, checksum in sorted(children, key=lambda item: item[0] + ("/" if item[1] == "40000" else "")))
    return hashlib.sha1(b"tree " + str(len(body)).encode() + b"\0" + body).hexdigest()


def save_pins(root, pins):
    (root / "runtime.lock.json").write_text(json.dumps(pins))


def historical_pins():
    # The live reviewed input advances to A2. A1 regression fixtures remain an
    # explicit historical profile, not whichever profile happens to be current.
    pins = json.loads((ROOT / "runtime.lock.json").read_text())
    pins.update(format=1, profile=p.A1)
    pins.pop("qualification", None)
    pins["build"].pop("admission", None)
    pins["patch_series_sha256"] = p.digest(b'{"format":1,"patches":[]}\n')
    return pins


def fixture(root, contents=None):
    root.mkdir(exist_ok=True)
    (root / "patches").mkdir()
    (root / "patches/series.json").write_bytes(b'{"format":1,"patches":[]}\n')
    contents = contents if contents is not None else {"Cargo.lock": b'version = 3\n', "src/lib.rs": b"pub fn real_source() {}\n"}
    directories = {str(parent) for name in contents for parent in Path(name).parents if str(parent) != "."}
    entries = [{"path": name, "mode": "100644", "type": "blob", "sha": blob(data), "size": len(data)} for name, data in contents.items()]
    root_tree = None
    for directory in sorted(["", *directories], key=lambda name: (name.count("/"), len(name)), reverse=True):
        children = [(entry["path"].rpartition("/")[2], entry["mode"].lstrip("0"), entry["sha"]) for entry in entries if entry["path"].rpartition("/")[0] == directory]
        checksum = tree(children)
        if directory:
            entries.append({"path": directory, "mode": "040000", "type": "tree", "sha": checksum})
        else:
            root_tree = checksum
    pins = historical_pins()
    pins["upstream"].update(commit="1" * 40, tree=root_tree, archive_root="source", files=len(contents), directories=len(directories), payload_bytes=sum(map(len, contents.values())), cargo_lock_sha256=p.digest(contents["Cargo.lock"]))
    (root / "upstream-tree.json").write_text(json.dumps({"sha": "1" * 40, "truncated": False, "tree": entries}))
    pins["upstream"]["inventory_sha256"] = p.file_hash(root / "upstream-tree.json")
    members = [("source", None), *(("source/" + name, None) for name in sorted(directories)), *(("source/" + name, data) for name, data in contents.items())]
    archive = root / "source.tgz"
    write_archive(archive, members)
    repin_archive(root, pins, archive)
    return pins, archive, members


def write_archive(path, members):
    with tarfile.open(path, "w:gz", format=tarfile.PAX_FORMAT, pax_headers={"comment": "1" * 40}) as archive:
        for name, data in members:
            if isinstance(name, tarfile.TarInfo):
                member = name
            else:
                member = tarfile.TarInfo(name)
                member.type = tarfile.DIRTYPE if data is None else tarfile.REGTYPE
                member.mode = 0o775 if data is None else 0o664
                member.size = 0 if data is None else len(data)
            archive.addfile(member, None if data is None else io.BytesIO(data))


def repin_archive(root, pins, archive):
    pins["upstream"]["archive_bytes"] = archive.stat().st_size
    pins["upstream"]["archive_sha256"] = p.file_hash(archive)
    save_pins(root, pins)


def repin_patch(root, pins, series, data):
    path = root / p.PATCH_PATH
    path.write_bytes(data)
    series["patches"][0].update(bytes=len(data), sha256=p.digest(data))
    (root / "patches/series.json").write_text(json.dumps(series))
    pins["patch_series_sha256"] = p.file_hash(root / "patches/series.json")
    save_pins(root, pins)


def a2_fixture(root, additional_contents=None):
    contents = {
        "Cargo.toml": b'[workspace]\nmembers = ["crates/celld"]\n',
        "Cargo.lock": b"version = 3\n", "unchanged.txt": b"unchanged upstream\n",
    }
    contents.update({name: ("before " + name + "\nshared\nlast\n").encode() for name in p.PATCH_PATHS if name not in p.PATCH_ADDITIONS})
    contents.update(additional_contents or {})
    pins, archive, members = fixture(root, contents)
    pins.update(format=2, profile=p.A2, qualification={
        "base_commit": p.A2_BASE,
        "program": {name: {"bytes": len(name), "sha256": p.digest(name.encode())} for name in ("js", "wasm")},
        "execution_slots": 2, "wasm_bytes_per_slot": 64 * 1024 * 1024,
    })
    pins["build"]["admission"] = "remaining-target-allowance"
    files, patches = [], []
    for name in p.PATCH_PATHS:
        before = contents.get(name)
        after = ("after " + name + "\n").encode() + (b"shared\nlast\n" if before else b"")
        files.append({"path": name, "mode": "100644", "before": p.image_identity(before) if before else None, "after": p.image_identity(after)})
        if before:
            patches.append(("--- a/" + name + "\n+++ b/" + name + "\n@@ -1,3 +1,3 @@\n-before " + name + "\n+after " + name + "\n shared\n last\n").encode())
        else:
            patches.append(("--- /dev/null\n+++ b/" + name + "\n@@ -0,0 +1,1 @@\n+after " + name + "\n").encode())
    series = {"format": 2, "profile": p.A2, "base_commit": p.A2_BASE, "patches": [{"path": p.PATCH_PATH, "bytes": 0, "sha256": "", "files": files}]}
    data = b"".join(patches)
    repin_patch(root, pins, series, data)
    return pins, archive, series, data, contents


def foundation_file(root, name):
    path = root / name
    return {"path": name, "bytes": path.stat().st_size, "sha256": p.file_hash(path)}


def foundation_celld_manifest():
    return b'''[package]
name = "celld"
version = "0.6.2"
[features]
foks-guarded-qualification = []
foks-guarded-node-qualification = ["foks-guarded-qualification", "dep:foks-hosted-transport", "dep:foks-metadata-core"]
[dependencies]
foks-hosted-transport = { path = "../../../../../../crates/foks-hosted-transport", optional = true, default-features = false, features = ["qualification"] }
foks-metadata-core = { path = "../../../../../../crates/foks-metadata-core", optional = true, default-features = false, features = ["owned-list"] }
'''


@contextmanager
def foundation_fixture(root, additional_contents=None):
    """Synthetic two-patch source; never reads or replaces actual runtime/source."""
    extra = {name: ("upstream " + name + "\n").encode() for name in p.FOUNDATION_PATHS if name not in p.PATCH_PATHS and name not in p.FOUNDATION_ADDITIONS}
    extra.update(additional_contents or {})
    pins, archive, base_series, _, _ = a2_fixture(root, extra)
    native, binding = root / "v8.gz", root / "binding.rs"
    native.write_bytes(gzip.compress(b"!<arch>\n" + b"native" * 100))
    binding.write_bytes(b"// reviewed binding\n")
    for key, path in (("archive", native), ("binding", binding)):
        pins["v8"][key].update(bytes=path.stat().st_size, sha256=p.file_hash(path))
    save_pins(root, pins)
    accepted = p.prepare(archive, root)
    p.prepare_assets(native, binding, root)
    preserved = root / Path(p.FOUNDATION_MANIFEST).parent
    preserved.mkdir(parents=True)
    (root / "runtime/source").rename(preserved / "source")
    (root / "runtime/source-manifest.json").rename(preserved / "source-manifest.json")
    (root / "node.lock.json").write_bytes(b'{"synthetic_accepted_node":true}\n')
    (root / "Cargo.toml").write_bytes(b'[package]\nname = "foks-celld-guarded"\nversion = "0.0.0"\n[features]\nnode-qualification = ["celld/foks-guarded-node-qualification"]\n')
    before = b'version = 3\n[[package]]\nname = "celld"\nversion = "0.6.2"\ndependencies = []\n'
    expected = before.replace(b'dependencies = []', b'dependencies = ["foks-hosted-transport", "foks-metadata-core"]')
    (root / "Cargo.lock").write_bytes(before)
    files, chunks = [], []
    final = {row["path"]: dict(row) for row in accepted["files"]}
    for name in p.FOUNDATION_PATHS:
        path = preserved / "source" / name
        original = path.read_bytes() if path.exists() else None
        changed = foundation_celld_manifest() if name == "crates/celld/Cargo.toml" else ("foundation " + name + "\n").encode()
        files.append({"path": name, "mode": "100644", "before": None if original is None else p.image_identity(original), "after": p.image_identity(changed)})
        old_lines = [] if original is None else original.splitlines(keepends=True)
        new_lines = changed.splitlines(keepends=True)
        chunks.append(("--- " + ("/dev/null" if original is None else "a/" + name) + "\n+++ b/" + name + "\n@@ -" + str(1 if old_lines else 0) + "," + str(len(old_lines)) + " +1," + str(len(new_lines)) + " @@\n").encode() + b"".join(b"-" + line for line in old_lines) + b"".join(b"+" + line for line in new_lines))
        final[name] = {"path": name, "mode": "100644", **p.image_identity(changed)}
    (root / p.FOUNDATION_PATCH).write_bytes(b"".join(chunks))
    base = {name: foundation_file(root, path) for name, (path, _) in p.FOUNDATION_BASE_IMAGES.items()}
    series = {"format": 1, "profile": p.FOUNDATION, "accepted_a2_commit": p.NODE_BASE, "accepted_a3a_commit": p.FOUNDATION_BASE, "base_series": base["patch_series"], "patches": [copy.deepcopy(base_series["patches"][0]), {**foundation_file(root, p.FOUNDATION_PATCH), "files": files}]}
    (root / p.FOUNDATION_SERIES).write_text(json.dumps(series))
    rows = sorted(final.values(), key=lambda item: item["path"])
    program = {name: {"bytes": len(name), "sha256": p.digest(name.encode())} for name in ("primary_js", "fixture_js", "primary_metadata", "fixture_metadata")}
    program["wasm"] = copy.deepcopy(pins["qualification"]["program"]["wasm"])
    profile = {"format": 1, "profile": p.FOUNDATION, "target": p.TARGET, "accepted_a2_commit": p.NODE_BASE, "accepted_a3a_commit": p.FOUNDATION_BASE, "base": base, "pins": pins, "node_series": foundation_file(root, p.FOUNDATION_SERIES), "cargo_lock": {"before": foundation_file(root, "Cargo.lock"), "expected": {"path": "Cargo.lock", "bytes": len(expected), "sha256": p.digest(expected)}, "added_edges": [{"from": "celld", "to": name} for name in p.FOUNDATION_EDGES]}, "qualification": {"claim": "b1-foundation-only", "source_roster": list(p.FOUNDATION_PATHS), "tree": {"files": len(rows), "payload_bytes": sum(row["bytes"] for row in rows), "inventory_sha256": p.digest(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode())}, "program": program, "deployments": {name: {"version": "1" * 16, "prefix": "deploy/foks-guarded-node-" + name + "/" + "1" * 16} for name in ("primary", "fixture")}, "tests": {"prefix": p.FOUNDATION_TEST_PREFIX, "names": p.FOUNDATION_TESTS.copy(), "threads": 1}, "modes": copy.deepcopy(p.FOUNDATION_MODES), "not_qualified": p.FOUNDATION_EXCLUSIONS.copy()}}
    (root / p.FOUNDATION_PROFILE).write_text(json.dumps(profile))
    base_hashes = {name: (entry["path"], entry["sha256"]) for name, entry in base.items()}
    with patch.object(p, "FOUNDATION_BASE_IMAGES", base_hashes), patch.object(p, "FOUNDATION_LOCK_BEFORE", p.digest(before)):
        yield {"pins": pins, "archive": archive, "native": native, "binding": binding, "profile": profile, "series": series, "expected_lock": expected, "rows": rows, "accepted": accepted}


def repin_foundation(root, data, patch_bytes=None):
    if patch_bytes is not None:
        (root / p.FOUNDATION_PATCH).write_bytes(patch_bytes)
        data["series"]["patches"][1].update(foundation_file(root, p.FOUNDATION_PATCH))
    (root / p.FOUNDATION_SERIES).write_text(json.dumps(data["series"]))
    data["profile"]["node_series"] = foundation_file(root, p.FOUNDATION_SERIES)
    (root / p.FOUNDATION_PROFILE).write_text(json.dumps(data["profile"]))


@contextmanager
def lifetimes_fixture(root):
    """A full synthetic immutable B1 prefix followed by twenty-one B2 edits."""
    extra = {name: ("upstream " + name + "\n").encode() for name in p.LIFETIMES_PATHS if name not in p.FOUNDATION_PATHS and name not in p.PATCH_PATHS}
    with foundation_fixture(root, extra) as previous:
        (root / "Cargo.lock").write_bytes(previous["expected_lock"])
        accepted = p.prepare(previous["archive"], root, p.FOUNDATION)
        assets = p.prepare_assets(previous["native"], previous["binding"], root, p.FOUNDATION)
        preserved = root / Path(p.LIFETIMES_MANIFEST).parent
        preserved.mkdir(parents=True)
        (root / "runtime/source").rename(preserved / "source")
        (root / "runtime/source-manifest.json").rename(preserved / "source-manifest.json")
        for original, target in ((p.FOUNDATION_PROFILE, p.LIFETIMES_PROFILE), (p.FOUNDATION_SERIES, p.LIFETIMES_SERIES), ("runtime/assets-node-foundation.json", p.LIFETIMES_ASSETS)):
            destination = root / target
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((root / original).read_bytes())
        accepted_images = {name: foundation_file(root, path) for name, (path, _) in p.LIFETIMES_BASE_IMAGES.items()}
        files, chunks = [], []
        final = {row["path"]: dict(row) for row in accepted["files"]}
        for name in p.LIFETIMES_PATHS:
            before = (preserved / "source" / name).read_bytes()
            after = ("lifetimes " + name + "\n").encode()
            files.append({"path": name, "mode": "100644", "before": p.image_identity(before), "after": p.image_identity(after)})
            chunks.append(("--- a/" + name + "\n+++ b/" + name + "\n@@ -1," + str(len(before.splitlines())) + " +1,1 @@\n").encode() + b"".join(b"-" + line for line in before.splitlines(keepends=True)) + b"+" + after)
            final[name] = {"path": name, "mode": "100644", **p.image_identity(after)}
        (root / p.LIFETIMES_PATCH).write_bytes(b"".join(chunks))
        rows = sorted(final.values(), key=lambda row: row["path"])
        series = copy.deepcopy(previous["series"])
        series.update(profile=p.LIFETIMES, accepted_b1_commit=p.LIFETIMES_BASE)
        series["patches"].append({**foundation_file(root, p.LIFETIMES_PATCH), "files": files})
        (root / p.FOUNDATION_SERIES).write_text(json.dumps(series))
        profile = copy.deepcopy(previous["profile"])
        profile.update(profile=p.LIFETIMES, accepted_b1_commit=p.LIFETIMES_BASE, node_series=foundation_file(root, p.FOUNDATION_SERIES))
        profile["base"].update(accepted_images)
        profile["qualification"].update(claim="b2-lifetimes-components-only", source_roster=list(p.LIFETIMES_PATHS), tree={"files": len(rows), "payload_bytes": sum(row["bytes"] for row in rows), "inventory_sha256": p.digest(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode())}, tests={"prefix": p.LIFETIMES_TEST_PREFIX, "names": p.LIFETIMES_TESTS.copy(), "threads": 1}, modes=copy.deepcopy(p.LIFETIMES_MODES), layout_evidence=p.lifetimes_layout_contract())
        (root / p.FOUNDATION_PROFILE).write_text(json.dumps(profile))
        base_hashes = {name: (entry["path"], entry["sha256"]) for name, entry in accepted_images.items()}
        with patch.object(p, "LIFETIMES_BASE_IMAGES", base_hashes), patch.object(p, "LIFETIMES_LOCK_SHA256", p.digest(previous["expected_lock"])), patch.object(p, "LIFETIMES_ASSET_SHA256", p.file_hash(root / p.LIFETIMES_ASSETS)):
            yield {**previous, "profile": profile, "series": series, "rows": rows, "accepted_b1": accepted, "accepted_b1_profile": previous["profile"], "accepted_b1_assets": assets, "accepted_images": accepted_images}


def repin_lifetimes(root, data, patch_bytes=None):
    if patch_bytes is not None:
        (root / p.LIFETIMES_PATCH).write_bytes(patch_bytes)
        data["series"]["patches"][2].update(foundation_file(root, p.LIFETIMES_PATCH))
    (root / p.FOUNDATION_SERIES).write_text(json.dumps(data["series"]))
    data["profile"]["node_series"] = foundation_file(root, p.FOUNDATION_SERIES)
    (root / p.FOUNDATION_PROFILE).write_text(json.dumps(data["profile"]))


class ArchiveTests(unittest.TestCase):
    def test_exact_tree_preparation_reuses_only_identical_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, archive, _ = fixture(root)
            first = p.prepare(archive, root)
            self.assertEqual(p.prepare(archive, root), first)
            self.assertEqual(p.verify_source(root), first)
            self.assertEqual((root / "runtime/source/src/lib.rs").read_bytes(), b"pub fn real_source() {}\n")
            self.assertFalse(list((root / "runtime").glob("source-staging-*")))

    def test_archive_length_hash_and_inventory_are_checked_before_extracting(self):
        for mutation in ("archive", "inventory", "series"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                _, archive, _ = fixture(root)
                path = {"archive": archive, "inventory": root / "upstream-tree.json", "series": root / "patches/series.json"}[mutation]
                path.write_bytes(path.read_bytes() + b" ")
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertFalse((root / "runtime/source").exists())
                self.assertFalse(list((root / "runtime").glob("source-staging-*")))

    def test_nonempty_patch_series_is_refused_even_with_matching_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, archive, _ = fixture(root)
            (root / "patches/series.json").write_text('{"format":1,"patches":["manual.patch"]}')
            pins["patch_series_sha256"] = p.file_hash(root / "patches/series.json")
            save_pins(root, pins)
            with self.assertRaisesRegex(ValueError, "empty patch series"):
                p.prepare(archive, root)

    def test_traversal_links_wrong_modes_duplicate_extra_missing_and_payload(self):
        mutations = ("traversal", "absolute", "symlink", "hardlink", "mode", "duplicate", "extra", "missing", "payload")
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, members = fixture(root)
                if mutation == "traversal":
                    members[-1] = ("source/../escape", members[-1][1])
                elif mutation == "absolute":
                    members[-1] = ("/escape", members[-1][1])
                elif mutation in ("symlink", "hardlink", "mode"):
                    member = tarfile.TarInfo("source/src/lib.rs")
                    member.type = {"symlink": tarfile.SYMTYPE, "hardlink": tarfile.LNKTYPE, "mode": tarfile.REGTYPE}[mutation]
                    member.linkname = "../../escape"
                    member.mode = 0o755 if mutation == "mode" else 0o664
                    data = members[-1][1] if mutation == "mode" else None
                    member.size = len(data) if data is not None else 0
                    members[-1] = (member, data)
                elif mutation == "duplicate":
                    members.append(members[-1])
                elif mutation == "extra":
                    members.append(("source/extra", b"extra"))
                elif mutation == "missing":
                    members.pop()
                else:
                    members[-1] = (members[-1][0], b"x" * len(members[-1][1]))
                write_archive(archive, members)
                repin_archive(root, pins, archive)
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertFalse((root / "runtime/source").exists())
                self.assertFalse((root.parent / "escape").exists())
                self.assertFalse(list((root / "runtime").glob("source-staging-*")))

    def test_altered_source_content_mode_roster_and_symlink_refuse_reuse(self):
        for mutation in ("content", "mode", "extra", "symlink", "missing"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                _, archive, _ = fixture(root)
                p.prepare(archive, root)
                path = root / "runtime/source/src/lib.rs"
                if mutation == "content":
                    path.write_bytes(b"x" * path.stat().st_size)
                elif mutation == "mode":
                    path.chmod(0o755)
                elif mutation == "extra":
                    path.with_name("extra").write_text("unreviewed")
                elif mutation == "symlink":
                    path.unlink()
                    path.symlink_to(archive)
                else:
                    path.unlink()
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertTrue((root / "runtime/source").is_dir())

    def test_source_publication_without_receipt_stays_unusable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, archive, _ = fixture(root)
            with patch.object(p, "write_json", side_effect=OSError("receipt publication failed")):
                with self.assertRaisesRegex(OSError, "receipt publication failed"):
                    p.prepare(archive, root)
            self.assertTrue((root / "runtime/source").is_dir())
            self.assertFalse((root / "runtime/source-manifest.json").exists())
            with self.assertRaises(ValueError):
                p.prepare(archive, root)
            self.assertFalse(list((root / "runtime").glob("source-staging-*")))


class PatchTests(unittest.TestCase):
    def test_duplicate_json_fields_cannot_override_closed_records(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            for data in ('{"format":1,"format":2}', '{"entry":{"bytes":1,"bytes":2}}'):
                path.write_text(data)
                with self.assertRaisesRegex(ValueError, "duplicate JSON field"):
                    p.read_json(path)

    def test_a2_archive_is_distinct_from_exact_patched_tree_and_reuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, archive, series, _, contents = a2_fixture(root)
            original = p.inspect_archive(archive, root)
            self.assertEqual(original["kind"], "upstream-archive-only")
            self.assertEqual({entry["path"] for entry in original["files"]}, set(contents))
            prepared = p.prepare(archive, root)
            self.assertEqual(prepared["kind"], "patched-source")
            self.assertEqual(prepared["format"], 2)
            self.assertEqual({entry["path"] for entry in prepared["files"]}, set(contents) | p.PATCH_ADDITIONS)
            for entry in series["patches"][0]["files"]:
                path = root / "runtime/source" / entry["path"]
                self.assertEqual(p.image_identity(path.read_bytes()), entry["after"])
                self.assertEqual(path.stat().st_mode & 0o777, 0o644)
                self.assertEqual(path.parent.stat().st_mode & 0o777, 0o755)
            self.assertEqual((root / "runtime/source/Cargo.lock").read_bytes(), contents["Cargo.lock"])
            self.assertEqual(p.prepare(archive, root), prepared)
            # Verification derives/reconstructs the pinned postimage; it does
            # not depend on an original archive path or trust a success marker.
            archive.unlink()
            self.assertEqual(p.verify_source(root), prepared)
            self.assertEqual(prepared["input_sha256"], p.file_hash(root / "runtime.lock.json"))
            self.assertEqual(prepared["patch_series_sha256"], pins["patch_series_sha256"])

    def test_a2_qualification_and_resource_policy_are_closed(self):
        mutations = (
            lambda pins: pins.update(format=True),
            lambda pins: pins.update(extra="unknown"),
            lambda pins: pins["qualification"].update(extra=0),
            lambda pins: pins["qualification"].update(base_commit="0" * 40),
            lambda pins: pins["qualification"].update(execution_slots=True),
            lambda pins: pins["qualification"].update(execution_slots=3),
            lambda pins: pins["qualification"].update(wasm_bytes_per_slot=1),
            lambda pins: pins["qualification"]["program"].update(other={}),
            lambda pins: pins["qualification"]["program"]["js"].update(bytes=True),
            lambda pins: pins["qualification"]["program"]["wasm"].update(bytes="4"),
            lambda pins: pins["qualification"]["program"]["js"].update(bytes=0),
            lambda pins: pins["qualification"]["program"]["wasm"].update(bytes=524289),
            lambda pins: pins["qualification"]["program"]["js"].update(sha256="A" * 64),
            lambda pins: pins["qualification"]["program"]["wasm"].update(extra=0),
            lambda pins: pins["build"].update(admission="fixed-free"),
            lambda pins: pins["build"].update(free_floor_bytes=1),
            lambda pins: pins["build"].update(maximum_target_bytes=True),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, _, _, _, _ = a2_fixture(root)
                mutate(pins)
                save_pins(root, pins)
                with self.assertRaises(ValueError):
                    p.load_pins(root)

    def test_patch_envelope_paths_modes_bounds_and_images_are_closed(self):
        mutations = (
            lambda series: series.update(extra=0),
            lambda series: series.update(format=True),
            lambda series: series.update(profile=p.A1),
            lambda series: series.update(base_commit="0" * 40),
            lambda series: series["patches"].append(copy.deepcopy(series["patches"][0])),
            lambda series: series["patches"][0].update(path="patches/other.patch"),
            lambda series: series["patches"][0].update(bytes=True),
            lambda series: series["patches"][0].update(bytes=p.MAXIMUM_PATCH + 1),
            lambda series: series["patches"][0].update(sha256="0" * 64),
            lambda series: series["patches"][0]["files"].pop(),
            lambda series: series["patches"][0]["files"].__setitem__(1, copy.deepcopy(series["patches"][0]["files"][0])),
            lambda series: series["patches"][0]["files"][0].update(path="crates/celld/../outside"),
            lambda series: series["patches"][0]["files"][0].update(mode="100755"),
            lambda series: series["patches"][0]["files"][0].update(before=None),
            lambda series: series["patches"][0]["files"][-1].update(before=copy.deepcopy(series["patches"][0]["files"][0]["before"])),
            lambda series: series["patches"][0]["files"][0]["after"].update(bytes=-1),
            lambda series: series["patches"][0]["files"][0]["after"].update(bytes=True),
            lambda series: series["patches"][0]["files"][0]["after"].update(bytes=p.MAXIMUM_POSTIMAGE + 1),
            lambda series: series["patches"][0]["files"][0]["after"].update(git_sha1="A" * 40),
            lambda series: series["patches"][0]["files"][0]["after"].update(extra=0),
            lambda series: [entry["after"].update(bytes=p.MAXIMUM_POSTIMAGE) for entry in series["patches"][0]["files"]],
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, series, _, _ = a2_fixture(root)
                mutate(series)
                (root / "patches/series.json").write_text(json.dumps(series))
                pins["patch_series_sha256"] = p.file_hash(root / "patches/series.json")
                save_pins(root, pins)
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertFalse((root / "runtime/source").exists())

    def test_patch_syntax_excludes_generic_patch_operations(self):
        mutations = (
            lambda data: b"diff --git a/path b/path\n" + data,
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1 +1 @@", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1,3 +1,3 @@ annotation", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -0,3 +1,3 @@", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1,3 +0,3 @@", 1),
            lambda data: data.replace(b"--- a/crates/celld/Cargo.toml\n", b"--- a/crates/celld/Cargo.toml\t2026-01-01\n", 1),
            lambda data: data.replace(b"+++ b/crates/celld/Cargo.toml", b"+++ b/../escape", 1),
            lambda data: data.replace(b"--- /dev/null", b"--- a/crates/celld/new.rs", 1),
            lambda data: data.replace(b"+++ b/crates/celld/js/guarded.rs", b"+++ /dev/null", 1),
            lambda data: data.replace(b"--- a/crates/celld/Cargo.toml", b"--- a/crates/celld/lib.rs", 1),
            lambda data: data.replace(b" shared\n", b"?shared\n", 1),
            lambda data: data.replace(b" shared\n", b"\xffshared\n", 1),
            lambda data: data.replace(b" shared\n", b" shared\r\n", 1),
            lambda data: data[:-1],
            lambda data: data + b"old mode 100644\nnew mode 100755\n",
            lambda data: data + b"rename from a\nrename to b\n",
            lambda data: data + b"copy from a\ncopy to b\n",
            lambda data: data + b"GIT binary patch\n",
            lambda data: data + b"\\ No newline at end of file\n",
            lambda data: data + data,
            lambda data: data + b"\n",
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, series, data, _ = a2_fixture(root)
                repin_patch(root, pins, series, mutate(data))
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertFalse((root / "runtime/source").exists())
                self.assertFalse(list((root / "runtime").glob("source-staging-*")))

    def test_hunks_cannot_search_shift_overlap_or_lie_about_counts(self):
        mutations = (
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -2,3 +1,3 @@", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1,3 +2,3 @@", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1,2 +1,3 @@", 1),
            lambda data: data.replace(b"@@ -1,3 +1,3 @@", b"@@ -1,3 +1,4 @@", 1),
            lambda data: data.replace(b" shared\n", b" wrong!\n", 1),
            lambda data: data.replace(b"-before crates/celld/Cargo.toml", b"-changed upstream", 1),
            lambda data: data.replace(b" last\n", b" last\n@@ -1,1 +1,1 @@\n last\n", 1),
            lambda data: data.replace(b" last\n", b" last\n@@ -2,1 +2,1 @@\n shared\n", 1),
            lambda data: data.replace(b"@@ -0,0 +1,1 @@", b"@@ -1,0 +1,1 @@", 1),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, series, data, _ = a2_fixture(root)
                repin_patch(root, pins, series, mutate(data))
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                self.assertFalse((root / "runtime/source").exists())

    def test_disjoint_hunks_preserve_gaps_tail_and_reverse_to_upstream(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, archive, series, data, contents = a2_fixture(root)
            name = p.PATCH_PATHS[0]
            original = ("@@ -1,3 +1,3 @@\n-before " + name + "\n+after " + name + "\n shared\n last\n").encode()
            # Insertion and deletion have exact zero-length origins. The middle
            # untouched line survives both forward and reverse reconstruction.
            replacement = ("@@ -1,1 +1,2 @@\n-before " + name + "\n+first\n+second\n@@ -3,1 +3,0 @@\n-last\n").encode()
            expected = b"first\nsecond\nshared\n"
            series["patches"][0]["files"][0]["after"] = p.image_identity(expected)
            repin_patch(root, pins, series, data.replace(original, replacement, 1))
            p.prepare(archive, root)
            self.assertEqual((root / "runtime/source" / name).read_bytes(), expected)
            patches = p.parse_patch(root, pins, p.inventory(root, pins))
            self.assertEqual(p.transform_image(expected, patches[0][1], reverse=True), contents[name])
            p.verify_source(root)

    def test_full_preimage_and_postimage_hashes_bind_the_patch(self):
        for side in ("before", "after"):
            for field in ("bytes", "sha256", "git_sha1"):
                with self.subTest(side=side, field=field), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    pins, archive, series, data, _ = a2_fixture(root)
                    image = series["patches"][0]["files"][0][side]
                    image[field] = image[field] + 1 if field == "bytes" else "0" * len(image[field])
                    repin_patch(root, pins, series, data)
                    with self.assertRaises(ValueError):
                        p.prepare(archive, root)
                    self.assertFalse((root / "runtime/source").exists())

    def test_changed_patch_or_tampered_source_cannot_relabel_existing_tree(self):
        for mutation in ("postimage", "unchanged", "manifest", "patch", "preimage", "forged-postimage"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, series, data, _ = a2_fixture(root)
                manifest = p.prepare(archive, root)
                before = {str(path.relative_to(root / "runtime/source")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}
                if mutation in ("postimage", "unchanged", "forged-postimage"):
                    selected = "unchanged.txt" if mutation == "unchanged" else p.PATCH_PATHS[0]
                    path = root / "runtime/source" / selected
                    path.write_bytes(b"x" * path.stat().st_size)
                    before[selected] = path.read_bytes()
                    # Even a caller-edited success marker cannot bless it.
                    next(entry for entry in manifest["files"] if entry["path"] == selected).update(p.image_identity(path.read_bytes()))
                    if mutation == "forged-postimage":
                        # Also rewrite the declared after hash and marker's
                        # input binding. Reverse reconstruction must still
                        # reject the file against the real upstream preimage.
                        series["patches"][0]["files"][0]["after"] = p.image_identity(path.read_bytes())
                        repin_patch(root, pins, series, data)
                        manifest.update(input_sha256=p.file_hash(root / "runtime.lock.json"), patch_series_sha256=pins["patch_series_sha256"])
                    p.write_json(root / "runtime/source-manifest.json", manifest)
                elif mutation == "manifest":
                    manifest["kind"] = "upstream-archive-only"
                    p.write_json(root / "runtime/source-manifest.json", manifest)
                elif mutation == "patch":
                    repin_patch(root, pins, series, data.replace(b" shared\n", b" changed\n", 1))
                else:
                    series["patches"][0]["files"][0]["before"]["sha256"] = "0" * 64
                    repin_patch(root, pins, series, data)
                with self.assertRaises(ValueError):
                    p.prepare(archive, root)
                after = {str(path.relative_to(root / "runtime/source")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}
                self.assertEqual(before, after)

    def test_a1_prepared_tree_is_never_automatically_upgraded_to_a2(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, archive, series, data, _ = a2_fixture(root)
            a1 = copy.deepcopy(pins)
            a1.update(format=1, profile=p.A1)
            a1.pop("qualification")
            a1["build"].pop("admission")
            (root / "patches/series.json").write_bytes(b'{"format":1,"patches":[]}\n')
            a1["patch_series_sha256"] = p.file_hash(root / "patches/series.json")
            save_pins(root, a1)
            p.prepare(archive, root)
            manifest_before = (root / "runtime/source-manifest.json").read_bytes()
            contents_before = {str(path.relative_to(root / "runtime/source")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}
            repin_patch(root, pins, series, data)
            with self.assertRaises(ValueError):
                p.prepare(archive, root)
            self.assertEqual((root / "runtime/source-manifest.json").read_bytes(), manifest_before)
            self.assertEqual({str(path.relative_to(root / "runtime/source")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}, contents_before)

    def test_partial_patch_or_publication_failure_never_qualifies(self):
        for mutation in ("write", "receipt", "late-postimage"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pins, archive, series, data, _ = a2_fixture(root)
                original_open = Path.open
                def refuse_second_write(path, *args, **kwargs):
                    if "source-staging-" in str(path) and path.name == "lib.rs" and args and args[0] == "wb":
                        raise OSError("controlled patch write failure")
                    return original_open(path, *args, **kwargs)
                if mutation == "late-postimage":
                    series["patches"][0]["files"][-1]["after"]["sha256"] = "0" * 64
                    repin_patch(root, pins, series, data)
                selected = patch.object(Path, "open", refuse_second_write) if mutation == "write" else patch.object(p, "write_json", side_effect=OSError("controlled receipt failure")) if mutation == "receipt" else patch.object(p, "write_json", wraps=p.write_json)
                with selected, self.assertRaises((ValueError, OSError)):
                    p.prepare(archive, root)
                self.assertEqual((root / "runtime/source").exists(), mutation == "receipt")
                self.assertFalse((root / "runtime/source-manifest.json").exists())
                self.assertFalse(list((root / "runtime").glob("source-staging-*")))
                if mutation == "receipt":
                    with self.assertRaises(ValueError):
                        p.verify_source(root)


class FoundationTests(unittest.TestCase):
    def test_chain_roundtrip_and_full_inventory_are_stage_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                pins = p.load_pins(root, p.FOUNDATION)
                original = p.inspect_archive(data["archive"], root, stage=p.FOUNDATION)
                self.assertEqual((original["format"], original["kind"]), (3, "upstream-archive-only"))
                self.assertNotEqual(original["files"], data["rows"])
                source = p.prepare(data["archive"], root, p.FOUNDATION)
                self.assertEqual(source["files"], data["rows"])
                self.assertEqual(source["profile"], p.FOUNDATION)
                self.assertEqual(source["kind"], "patched-source")
                self.assertEqual(source["base_input_sha256"], p.file_hash(root / "runtime.lock.json"))
                self.assertEqual(source["patch_series_sha256"], p.file_hash(root / p.FOUNDATION_SERIES))
                self.assertEqual(p.prepare(data["archive"], root, p.FOUNDATION), source)
                changes = p.parse_patch(root, pins, p.inventory(root, pins))
                self.assertEqual(sum(entry["path"] == "crates/celld/guarded_execution.rs" for entry, _ in changes), 2)
                data["archive"].unlink()
                self.assertEqual(p.verify_source(root, p.FOUNDATION), source)
                self.assertEqual(p.read_json(root / p.FOUNDATION_MANIFEST), data["accepted"])
                with self.assertRaises(ValueError):
                    p.verify_source(root)

    def test_prepared_historical_tree_is_not_implicitly_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                preserved = root / Path(p.FOUNDATION_MANIFEST).parent
                import shutil
                shutil.copytree(preserved / "source", root / "runtime/source")
                shutil.copyfile(preserved / "source-manifest.json", root / "runtime/source-manifest.json")
                before = (root / "runtime/source-manifest.json").read_bytes()
                with self.assertRaises(ValueError):
                    p.prepare(data["archive"], root, p.FOUNDATION)
                self.assertEqual((root / "runtime/source-manifest.json").read_bytes(), before)
                self.assertEqual(p.verify_source(root), data["accepted"])

    def test_profile_closed_fields_bounds_and_accepted_inputs(self):
        mutations = (
            lambda x: x.update(format=True), lambda x: x.update(profile=p.A2),
            lambda x: x.update(extra=0), lambda x: x.update(accepted_a3a_commit="0" * 40),
            lambda x: x["base"]["source_manifest"].update(path="runtime/source-manifest.json"),
            lambda x: x["base"]["node_profile"].update(sha256="0" * 64),
            lambda x: x["pins"]["build"].update(maximum_target_bytes=1),
            lambda x: x["cargo_lock"]["before"].update(sha256="0" * 64),
            lambda x: x["cargo_lock"]["expected"].update(bytes=True),
            lambda x: x["cargo_lock"]["added_edges"].append({"from": "celld", "to": "foks-server"}),
            lambda x: x["qualification"]["tree"].update(files=True),
            lambda x: x["qualification"]["tree"].update(payload_bytes=p.MAXIMUM_FOUNDATION_SOURCE + 1),
            lambda x: x["qualification"]["tree"].update(inventory_sha256="A" * 64),
            lambda x: x["qualification"]["source_roster"].append("crates/celld/main.rs"),
            lambda x: x["qualification"]["program"]["wasm"].update(sha256="0" * 64),
            lambda x: x["qualification"]["deployments"]["fixture"].update(prefix="deploy/other/" + "1" * 16),
            lambda x: x["qualification"]["tests"].update(threads=True),
            lambda x: x["qualification"]["tests"]["names"].pop(),
            lambda x: x["qualification"]["modes"]["node-foundation-default"].update(direct_root_tokio=0),
            lambda x: x["qualification"]["not_qualified"].pop(),
        )
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with foundation_fixture(root) as data:
                    mutation(data["profile"])
                    (root / p.FOUNDATION_PROFILE).write_text(json.dumps(data["profile"]))
                    with self.assertRaises(ValueError):
                        p.load_pins(root, p.FOUNDATION)

    def test_series_exact_order_roster_and_limits(self):
        mutations = (
            lambda x: x.update(format=True), lambda x: x.update(extra=0),
            lambda x: x["patches"].reverse(), lambda x: x["patches"].append(copy.deepcopy(x["patches"][1])),
            lambda x: x["patches"][0]["files"][0]["after"].update(sha256="0" * 64),
            lambda x: x["patches"][1].update(bytes=p.MAXIMUM_FOUNDATION_PATCH + 1),
            lambda x: x["patches"][1].update(path="patches/0003.patch"),
            lambda x: x["patches"][1]["files"].reverse(),
            lambda x: x["patches"][1]["files"][0].update(mode="100755"),
            lambda x: x["patches"][1]["files"][0].update(path="crates/celld/main.rs"),
            lambda x: next(row for row in x["patches"][1]["files"] if row["path"] in p.FOUNDATION_ADDITIONS).update(before=p.image_identity(b"old\n")),
        )
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with foundation_fixture(root) as data:
                    mutation(data["series"])
                    repin_foundation(root, data)
                    with self.assertRaises(ValueError):
                        p.prepare(data["archive"], root, p.FOUNDATION)
                    self.assertFalse((root / "runtime/source").exists())

    def test_second_preimage_must_match_first_patch_postimage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                first = data["series"]["patches"][0]["files"][0]
                second = next(row for row in data["series"]["patches"][1]["files"] if row["path"] == first["path"])
                second["before"] = copy.deepcopy(first["before"])
                repin_foundation(root, data)
                with self.assertRaisesRegex(ValueError, "preceding blob"):
                    p.prepare(data["archive"], root, p.FOUNDATION)
                self.assertFalse((root / "runtime/source").exists())

    def test_full_intermediate_hash_and_inventory_cannot_be_forged(self):
        for mutation in ("intermediate", "inventory", "payload", "source"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with foundation_fixture(root) as data:
                    p.prepare(data["archive"], root, p.FOUNDATION)
                    if mutation == "intermediate":
                        # Git identity still matches; reverse reconstruction
                        # must verify SHA256 of the actual intermediate bytes.
                        next(row for row in data["series"]["patches"][1]["files"] if row["path"] == "crates/celld/guarded_execution.rs")["before"]["sha256"] = "0" * 64
                    elif mutation == "inventory":
                        data["profile"]["qualification"]["tree"]["inventory_sha256"] = "0" * 64
                    elif mutation == "payload":
                        data["profile"]["qualification"]["tree"]["payload_bytes"] += 1
                    else:
                        (root / "runtime/source/crates/celld/guarded_execution.rs").write_bytes(b"changed\n")
                    repin_foundation(root, data)
                    with self.assertRaises(ValueError):
                        p.verify_source(root, p.FOUNDATION)

    def test_bad_late_postimage_never_publishes_partial_chain(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                data["series"]["patches"][1]["files"][-1]["after"]["sha256"] = "0" * 64
                repin_foundation(root, data)
                with self.assertRaises(ValueError):
                    p.prepare(data["archive"], root, p.FOUNDATION)
                self.assertFalse((root / "runtime/source").exists())
                self.assertFalse(list((root / "runtime").glob("source-staging-*")))
                self.assertEqual(p.read_json(root / p.FOUNDATION_MANIFEST), data["accepted"])

    def test_preparation_lock_preimage_never_authorizes_graph_or_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                pins = p.load_pins(root, p.FOUNDATION)
                with self.assertRaisesRegex(ValueError, "size/hash"):
                    p.foundation_lock(root, pins)
                p.prepare(data["archive"], root, p.FOUNDATION)
                (root / "Cargo.lock").write_bytes(data["expected_lock"])
                p.foundation_lock(root, p.load_pins(root, p.FOUNDATION))
                (root / "Cargo.lock").write_bytes(data["expected_lock"] + b"\n")
                with self.assertRaisesRegex(ValueError, "neither reviewed"):
                    p.load_pins(root, p.FOUNDATION)

    def test_assets_reuse_bytes_and_preserve_historical_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with foundation_fixture(root) as data:
                path = root / "runtime/artifacts/assets.json"
                before = path.read_bytes()
                identities = {item.name: (item.stat().st_ino, p.file_hash(item)) for item in path.parent.iterdir()}
                with self.assertRaises(ValueError):
                    p.verify_assets(root, p.FOUNDATION)
                receipt = p.prepare_assets(data["native"], data["binding"], root, p.FOUNDATION)
                self.assertEqual(receipt["profile"], p.FOUNDATION)
                self.assertEqual(receipt["base_receipt_sha256"], p.digest(before))
                self.assertEqual(p.verify_assets(root, p.FOUNDATION), receipt)
                self.assertEqual(path.read_bytes(), before)
                self.assertEqual({item.name: (item.stat().st_ino, p.file_hash(item)) for item in path.parent.iterdir()}, identities)
                p.verify_assets(root)
                for altered in ({**receipt, "profile": p.A2}, {**receipt, "format": True}):
                    (root / "runtime/assets-node-foundation.json").write_text(json.dumps(altered))
                    with self.subTest(receipt=altered), self.assertRaisesRegex(ValueError, "foundation native asset"):
                        p.prepare_assets(data["native"], data["binding"], root, p.FOUNDATION)
                    self.assertEqual(path.read_bytes(), before)

    def test_three_modes_bind_features_targets_and_exact_test_filter(self):
        pins = {**historical_pins(), "profile": p.FOUNDATION}
        with patch.object(p, "network_command", side_effect=lambda argv: list(map(str, argv))):
            for mode, contract in p.FOUNDATION_MODES.items():
                commands = p.build_commands(Path("/fixture"), Path("/toolchain"), pins, mode)
                build = commands["build"]
                self.assertEqual("--features" in build, bool(contract["root_features"]))
                if contract["root_features"]:
                    self.assertEqual(build[build.index("--features") + 1], "foks-celld-guarded/" + contract["root_features"][0])
                self.assertEqual(build[build.index("--bin") + 1], "foks-celld-guarded" if mode.endswith("lifecycle") else "celld")
                self.assertEqual("lifecycle" in commands, mode.endswith("lifecycle"))
                self.assertEqual(commands["clippy"][-3:], ["--", "-D", "warnings"])
            commands = p.foundation_test_commands(Path("/fixture"), Path("/toolchain"), pins)
            for name, argv in commands.items():
                self.assertIn("--locked", argv)
                self.assertIn("--offline", argv)
                self.assertIn("--lib", argv)
                self.assertEqual(argv.count("--package"), 2)
                self.assertEqual(argv[-3:], [p.FOUNDATION_TEST_PREFIX, "--", "--list" if name == "list" else "--test-threads=1"])
        for mode in ("identity", "lifecycle", "node-baseline", "node-foundation-future"):
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                p.selected_mode(pins, mode)
        for mode in p.FOUNDATION_MODES:
            with self.assertRaises(ValueError):
                p.selected_mode(historical_pins(), mode)

    def test_component_output_requires_all_twenty_one_passed_without_extras(self):
        listing = "\n".join(name + ": test" for name in p.FOUNDATION_TESTS) + "\n\n21 tests, 0 benchmarks\n"
        run = "running 21 tests\n" + "\n".join("test " + name + " ... ok" for name in p.FOUNDATION_TESTS) + "\n\ntest result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.12s\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "output"
            for is_list, text in ((True, listing), (False, run)):
                path.write_text(text)
                self.assertEqual(p.foundation_test_output(path, is_list), p.FOUNDATION_TESTS)
                for altered in ("", text.replace(p.FOUNDATION_TESTS[0], "unknown"), text.replace(p.FOUNDATION_TESTS[0], p.FOUNDATION_TESTS[1]), text.replace("21 tests", "20 tests"), text + "unexpected output\n", text.replace(": test", ": benchmark", 1) if is_list else text.replace(" ... ok", " ... ignored", 1)):
                    path.write_text(altered)
                    with self.subTest(listing=is_list, text=altered[:30]), self.assertRaises(ValueError):
                        p.foundation_test_output(path, is_list)

    @staticmethod
    def layout_record():
        # Synthetic parser input, not a substitute for the compiled record.
        layouts = {
            "request_arc": (256, 8), "control_arc": (128, 8),
            "registry_arc": (1024, 8), "owner_factory": (128, 8),
            "original_owner": (48, 8), "header_cell": (16, 8),
            "cleanup_credits": (2, 1), "control_observation": (104, 8),
            "node_arc": (256, 8),
        }
        return {"format": 1, "claim": "b1-native-layouts-only", "arc_convention": "repr-c-two-atomic-usize", "layouts": {name: {"bytes": size, "alignment": alignment} for name, (size, alignment) in layouts.items()}, "role_caps": [1048576, 8388608, 65536], "role_counts": [1, 2, 1], "initial_process_debt": 67 * 256 + 128 + 1024, "cleanup_reserve": 204800, "weak_limit": 66}

    def test_compiled_layout_record_schema_and_raw_evidence(self):
        record = self.layout_record()
        prefix = b"FOKS_B1_LAYOUT "

        def encode(value):
            return prefix + json.dumps(value, separators=(",", ":")).encode() + b"\n"

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "tests-run.stderr"

            def observe(raw):
                path.write_bytes(raw)
                result = p.foundation_layout_output(path, hashlib.sha256(raw).hexdigest())
                self.assertEqual(path.read_bytes(), raw)
                return result

            tagged = encode(record)
            for raw in (tagged, b"ordinary Cargo output\n\xffnative warning\n" + tagged + b"other stderr\n"):
                self.assertEqual(observe(raw), {"record": record, "raw_record_bytes": len(tagged), "raw_record_sha256": hashlib.sha256(tagged).hexdigest()})
            padded = tagged[:-1] + b" " * (4096 - len(tagged)) + b"\n"
            self.assertEqual(len(padded), 4096)
            result = observe(padded)
            self.assertEqual(result["record"], record)
            self.assertEqual(result["raw_record_bytes"], 4096)
            self.assertEqual(result["raw_record_sha256"], hashlib.sha256(padded).hexdigest())
            self.assertEqual(observe(b"x" * (1024 * 1024 - len(tagged) - 1) + b"\n" + tagged)["record"], record)
            maxima = copy.deepcopy(record)
            for name, value in maxima["layouts"].items():
                value.update(bytes=4096 if name == "request_arc" else 65536, alignment=16)
            maxima["initial_process_debt"] = 67 * 4096 + 2 * 65536
            self.assertEqual(observe(encode(maxima))["record"], maxima)

            # Stage comes from the validated build, never from the record.
            # The default remains the exact historical B1 reserve.
            for stage, reserve, other in ((p.FOUNDATION, 204800, p.LIFETIMES), (p.LIFETIMES, 532480, p.FOUNDATION)):
                staged = {**record, "cleanup_reserve": reserve}
                raw = encode(staged)
                path.write_bytes(raw)
                checksum = hashlib.sha256(raw).hexdigest()
                self.assertEqual(p.foundation_layout_output(path, checksum, stage)["record"], staged)
                for rejected_stage in (other, None, "", "node-lifetimes-default", "unreviewed", True, 1, [], {}):
                    with self.subTest(stage=stage, rejected_stage=rejected_stage), self.assertRaises(ValueError):
                        p.foundation_layout_output(path, checksum, rejected_stage)
                if stage == p.LIFETIMES:
                    with self.assertRaises(ValueError):
                        p.foundation_layout_output(path, checksum)
                for wrong in (reserve + 1, float(reserve), True):
                    bad = encode({**staged, "cleanup_reserve": wrong})
                    path.write_bytes(bad)
                    with self.subTest(stage=stage, reserve=wrong), self.assertRaises(ValueError):
                        p.foundation_layout_output(path, hashlib.sha256(bad).hexdigest(), stage)

            malformed = []
            for key in record:
                changed = copy.deepcopy(record)
                del changed[key]
                malformed.append(("missing " + key, changed))
            malformed.append(("extra field", {**record, "future": 1}))
            for key, values in {
                "format": (True, 1.0, 2, "1"),
                "claim": ("production-ready", None),
                "arc_convention": ("guessed-pointer-header", None),
                "role_caps": ([1048576, 8388608, 65537], [1048576.0, 8388608, 65536]),
                "role_counts": ([True, 2, 1], [1, 1, 1], [1, 2]),
                "initial_process_debt": (True, record["initial_process_debt"] + 1, float(record["initial_process_debt"]), None),
                "cleanup_reserve": (204801, 204800.0, True),
                "weak_limit": (65, 66.0, True),
                "layouts": ([], None),
            }.items():
                for value in values:
                    malformed.append((key + " wrong value/type " + repr(value), {**record, key: value}))
            for name in record["layouts"]:
                changed = copy.deepcopy(record)
                del changed["layouts"][name]
                malformed.append(("missing layout " + name, changed))
            changed = copy.deepcopy(record)
            changed["layouts"]["future"] = {"bytes": 8, "alignment": 8}
            malformed.append(("extra layout", changed))
            for value in ([], None, {"bytes": 8}, {"alignment": 8}, {"bytes": 8, "alignment": 8, "extra": 1}):
                changed = copy.deepcopy(record)
                changed["layouts"]["node_arc"] = value
                malformed.append(("layout shape " + repr(value), changed))
            for key, values in {
                "bytes": (True, 0, -1, 65537, 257, 256.0, "256", None, float("nan"), float("inf"), -float("inf")),
                "alignment": (True, 0, -1, 3, 32, 8.0, "8", None),
            }.items():
                for value in values:
                    changed = copy.deepcopy(record)
                    changed["layouts"]["node_arc"][key] = value
                    malformed.append(("layout " + key + " " + repr(value), changed))
            changed = copy.deepcopy(record)
            changed["layouts"]["request_arc"]["bytes"] = 4104
            changed["initial_process_debt"] = 67 * 4104 + 128 + 1024
            malformed.append(("oversized request with matching debt", changed))
            for label, value in malformed:
                with self.subTest(case=label), self.assertRaises(ValueError):
                    observe(encode(value))

            duplicates = (
                tagged.replace(b'"format":1', b'"format":1,"format":1', 1),
                tagged.replace(b'"bytes":256', b'"bytes":256,"bytes":256', 1),
                tagged.replace(b'"request_arc":', b'"request_arc":{"bytes":256,"alignment":8},"request_arc":', 1),
            )
            malformed_raw = [
                b"ordinary stderr only\n", tagged + tagged,
                b" " + tagged, tagged.replace(prefix, b"FOKS_B1_LAYOUT\t", 1),
                tagged.replace(prefix, b"FOKS_B1_LAYOUT", 1),
                tagged.replace(b"\n", b"\r\n"), tagged[:-1],
                prefix + b"\xff\n", tagged[:-1] + b"{}\n",
                tagged.replace(b'"bytes":256', b'"bytes":1e999', 1),
                prefix + b"[]\n", prefix + b"null\n",
                padded[:-1] + b" \n", b"x" * (1024 * 1024 - len(tagged)) + b"\n" + tagged,
                *duplicates,
            ]
            for index, raw in enumerate(malformed_raw):
                with self.subTest(raw_case=index), self.assertRaises(ValueError):
                    observe(raw)

    def test_compiled_layout_stderr_binds_owned_hash_and_stable_regular_file(self):
        raw = b"Cargo diagnostic\nFOKS_B1_LAYOUT " + json.dumps(self.layout_record()).encode() + b"\n"
        checksum = hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "tests-run.stderr"
            path.write_bytes(raw)
            for expected in (None, True, "", "A" * 64, "0" * 63, "0" * 64):
                with self.subTest(expected=expected), self.assertRaises(ValueError):
                    p.foundation_layout_output(path, expected)
            path.write_bytes(raw.replace(b"Cargo diagnostic", b"other diagnostic"))
            with self.assertRaisesRegex(ValueError, "differs from owned command"):
                p.foundation_layout_output(path, checksum)
            path.write_bytes(raw)
            link = root / "link"
            link.symlink_to(path)
            fifo = root / "fifo"
            os.mkfifo(fifo)
            for invalid in (link, fifo, root, root / "missing"):
                with self.subTest(path=str(invalid)), self.assertRaises(ValueError):
                    p.foundation_layout_output(invalid, checksum)

            real_open = os.open

            def replace_before_open(candidate, flags):
                replacement = root / "replacement-before"
                replacement.write_bytes(raw)
                replacement.replace(path)
                return real_open(candidate, flags)

            with patch.object(p.os, "open", side_effect=replace_before_open), self.assertRaisesRegex(ValueError, "replaced before read"):
                p.foundation_layout_output(path, checksum)
            path.write_bytes(raw)
            real_fstat = os.fstat
            calls = 0

            def replace_after_read(descriptor):
                nonlocal calls
                calls += 1
                observed = real_fstat(descriptor)
                if calls == 2:
                    replacement = root / "replacement-after"
                    replacement.write_bytes(raw)
                    replacement.replace(path)
                return observed

            with patch.object(p.os, "fstat", side_effect=replace_after_read), self.assertRaisesRegex(ValueError, "replaced or resized"):
                p.foundation_layout_output(path, checksum)
            self.assertEqual(calls, 2)

    def test_actual_binary_probe_output_has_closed_positive_and_refusal_shapes(self):
        # Render the unchanged pinned source literally, rather than inventing a
        # short prefix fixture that could hide a changed/truncated CLI surface.
        cli = (ROOT / "runtime/source/crates/celld/main/cli.rs").read_text()
        help_literal = cli.split("pub(crate) fn print_help() -> anyhow::Result<()> {", 1)[1].split('r#"', 1)[1].split('"#,', 1)[0]
        self.assertEqual(help_literal.count("{shutdown_total_ms}"), 1)
        self.assertEqual(help_literal.count("{"), 1)
        self.assertIn("pub const DEFAULT_SHUTDOWN_TOTAL_MS: u64 = 40_000;", (ROOT / "runtime/source/crates/celld/env_vars.rs").read_text())
        help_output = (help_literal.replace("{shutdown_total_ms}", "40000") + "\n").encode()
        self.assertEqual((len(help_output), hashlib.sha256(help_output).hexdigest()), (7752, "421243bdfe4c6f4e89834852eadd8becb82a1b3931f4d29fd80b5f9da4f6c4f9"))
        allocator = b"<jemalloc>: option background_thread currently supports pthread only\n"
        memory = b"2026-10-16T21:55:19.715603Z  WARN celld::memory: the allocator will not run a background thread, so freed pages return only when a thread allocates again error=`name` or `mib` specifies an unknown/invalid value.\n"
        refusal = b"Error: unknown command or option: --foks-guarded-node-qualification; run `celld --help` for usage\n"
        outputs = {
            "version": (b"celld 0.6.2 (debug)\n", b""),
            "help": (help_output, b""),
            "invalid-option": (b"", refusal),
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def observe(name, stdout, stderr):
                for stream, data in (("stdout", stdout), ("stderr", stderr)):
                    (root / (name + "." + stream)).write_bytes(data)
                result = p.foundation_probe_output(root, name)
                for stream, data in (("stdout", stdout), ("stderr", stderr)):
                    self.assertEqual((root / (name + "." + stream)).read_bytes(), data)
                    self.assertEqual(result[stream + "_bytes"], len(data))
                    self.assertEqual(result[stream + "_sha256"], hashlib.sha256(data).hexdigest())
                return result

            for name, (stdout, stderr) in outputs.items():
                variants = [(stdout, [])]
                if name != "invalid-option":
                    for before, after in ((b"", stdout), (stdout, b"")):
                        variants.append((before + memory + after, [{"id": "celld-allocator-background-thread-unavailable", "stream": "stdout", "line": before.count(b"\n") + 1, "text": memory.decode().removesuffix("\n")}]))
                    if name == "help":
                        first, rest = stdout.split(b"\n", 1)
                        variants.append((first + b"\n" + memory + rest, [{"id": "celld-allocator-background-thread-unavailable", "stream": "stdout", "line": 2, "text": memory.decode().removesuffix("\n")}]))
                for actual_stdout, expected_warnings in variants:
                    for prefix in (b"", allocator):
                        warnings = ([{"id": "jemalloc-background-thread-unsupported", "stream": "stderr", "line": 1, "text": allocator.decode().removesuffix("\n")}] if prefix else []) + expected_warnings
                        with self.subTest(name=name, warnings=len(warnings), stdout=actual_stdout[:40]):
                            self.assertEqual(observe(name, actual_stdout, prefix + stderr)["recognized_warnings"], warnings)
                malformed = [
                    (stdout + b"--foks-guarded-node-qualification\n", stderr),
                    (stdout + b"unrelated trailing log\n", stderr),
                    (b"unexpected leading log\n" + stdout, stderr),
                    (stdout, b"unknown warning\n" + stderr),
                    (stdout, allocator + allocator + stderr),
                    (stdout, allocator.replace(b"pthread", b"other") + stderr),
                    (allocator + stdout, stderr),
                    (stdout, memory + stderr),
                    (stdout + b"\x00", stderr),
                    (stdout + b"\xff", stderr),
                ]
                if name == "invalid-option":
                    malformed += [(memory, stderr), (stdout, stderr + allocator), (stdout, b" " + stderr), (stdout, stderr.removesuffix(b"\n")), (stdout, stderr.replace(b"\n", b"\r\n"))]
                else:
                    malformed += [(memory + memory + stdout, stderr), (stdout.removesuffix(b"\n"), stderr), (stdout.replace(b"\n", b"\r\n"), stderr)]
                    for changed in (memory.replace(b"WARN", b"INFO"), memory.replace(b"celld::memory", b"celld::other"), memory.replace(b"unknown/invalid", b"different"), memory.replace(b".715603Z", b".715603123Z"), memory.replace(b"2026-", b"xxxx-", 1), memory.replace(b"\n", b"\r\n"), memory.removesuffix(b"\n"), b"\x1b[33m" + memory):
                        malformed.append((changed + stdout, stderr))
                for actual_stdout, actual_stderr in malformed:
                    with self.subTest(name=name, rejected=(actual_stdout[:40], actual_stderr[:40])), self.assertRaises(ValueError):
                        observe(name, actual_stdout, actual_stderr)
            for altered in (help_output.split(b"OPTIONS:", 1)[0], help_output.replace(b"40000", b"40001"), help_output + b"another option\n"):
                with self.assertRaises(ValueError):
                    observe("help", altered, b"")
            with self.assertRaises(ValueError):
                observe("version", b"celld 0.6.3 (debug)\n", b"")
            with self.assertRaises(ValueError):
                observe("future", b"", b"")

    def test_nonzero_refusal_requires_the_exact_expected_exit_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with p.BuildSlot(root) as slot:
                argv = [sys.executable, "-I", "-S", "-B", "-c", "raise SystemExit(1)"]
                record = p.run_owned(argv, root, {}, root, "expected", slot, 5, expected_exit_code=1)
                self.assertEqual(record["exit_code"], 1)
                self.assertEqual(record["reason"], "completed")
                self.assertIs(record["cleanup_established"], True)
                with self.assertRaisesRegex(ValueError, "command failed"):
                    p.run_owned(argv, root, {}, root, "unexpected", slot, 5)
                self.assertTrue(slot.clean)


class LifetimesTests(unittest.TestCase):
    def test_three_patch_roundtrip_preserves_the_accepted_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with lifetimes_fixture(root) as data:
                preserved = {name: (root / name).read_bytes() for name in (p.PATCH_PATH, p.FOUNDATION_PATCH, p.LIFETIMES_PROFILE, p.LIFETIMES_SERIES, p.LIFETIMES_MANIFEST, p.LIFETIMES_ASSETS)}
                original = p.inspect_archive(data["archive"], root, stage=p.LIFETIMES)
                self.assertEqual((original["profile"], original["kind"]), (p.LIFETIMES, "upstream-archive-only"))
                source = p.prepare(data["archive"], root, p.LIFETIMES)
                self.assertEqual(source["files"], data["rows"])
                self.assertEqual(source["format"], 3)
                self.assertEqual(source["kind"], "patched-source")
                self.assertEqual(source["input_sha256"], p.file_hash(root / p.FOUNDATION_PROFILE))
                self.assertEqual(source["patch_series_sha256"], p.file_hash(root / p.FOUNDATION_SERIES))
                self.assertEqual(p.prepare(data["archive"], root, p.LIFETIMES), source)
                pins = p.load_pins(root, p.LIFETIMES)
                changes = p.parse_patch(root, pins, p.inventory(root, pins))
                self.assertEqual(sum(entry["path"] == "crates/celld/guarded_execution.rs" for entry, _ in changes), 3)
                self.assertEqual(len(p.LIFETIMES_PATHS), 21)
                self.assertIn("crates/celld/lib.rs", p.LIFETIMES_PATHS)
                self.assertEqual(sum(entry["path"] == "crates/celld/lib.rs" for entry, _ in changes), 3)
                self.assertEqual(sum(entry["path"] == "crates/celld/asyncrt.rs" for entry, _ in changes), 1)
                self.assertEqual(sum(row["path"] == "crates/celld/asyncrt.rs" for row in source["files"]), 1)
                data["archive"].unlink()
                self.assertEqual(p.verify_source(root, p.LIFETIMES), source)
                self.assertEqual({name: (root / name).read_bytes() for name in preserved}, preserved)
                for stage in (None, p.FOUNDATION):
                    with self.subTest(stage=stage), self.assertRaises(ValueError):
                        p.verify_source(root, stage)

    def test_prepared_foundation_is_not_implicitly_upgraded(self):
        import shutil
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with lifetimes_fixture(root) as data:
                preserved = root / Path(p.LIFETIMES_MANIFEST).parent
                shutil.copytree(preserved / "source", root / "runtime/source")
                shutil.copyfile(preserved / "source-manifest.json", root / "runtime/source-manifest.json")
                before = {str(path.relative_to(root / "runtime")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}
                manifest = (root / "runtime/source-manifest.json").read_bytes()
                with self.assertRaises(ValueError):
                    p.prepare(data["archive"], root, p.LIFETIMES)
                self.assertEqual((root / "runtime/source-manifest.json").read_bytes(), manifest)
                self.assertEqual({str(path.relative_to(root / "runtime")): path.read_bytes() for path in (root / "runtime/source").rglob("*") if path.is_file()}, before)

    def test_profile_is_closed_and_cannot_relabel_old_inputs_or_policy(self):
        mutations = (
            lambda x: x.update(format=True), lambda x: x.update(profile=p.FOUNDATION),
            lambda x: x.update(extra=0), lambda x: x.pop("accepted_b1_commit"),
            lambda x: x.update(accepted_b1_commit="0" * 40),
            lambda x: x["base"].pop("foundation_series"),
            lambda x: x["base"]["foundation_profile"].update(path=p.FOUNDATION_PROFILE),
            lambda x: x["base"]["foundation_source_manifest"].update(sha256="0" * 64),
            lambda x: x["pins"]["build"].update(timeout_seconds=99999),
            lambda x: x["cargo_lock"]["expected"].update(sha256="0" * 64),
            lambda x: x["qualification"].update(claim="guarded-readiness"),
            lambda x: x["qualification"]["source_roster"].append("crates/celld/main.rs"),
            lambda x: x["qualification"]["source_roster"].remove("crates/celld/asyncrt.rs"),
            lambda x: x["qualification"]["tree"].update(files=True),
            lambda x: x["qualification"]["tree"].update(payload_bytes=p.MAXIMUM_FOUNDATION_SOURCE + 1),
            lambda x: x["qualification"]["tree"].update(inventory_sha256="A" * 64),
            lambda x: x["qualification"]["program"]["fixture_js"].update(bytes=1),
            lambda x: x["qualification"]["deployments"]["primary"].update(version="0" * 16),
            lambda x: x["qualification"]["tests"].update(prefix=p.FOUNDATION_TEST_PREFIX),
            lambda x: x["qualification"]["tests"].update(threads=True),
            lambda x: x["qualification"]["tests"]["names"].pop(),
            lambda x: x["qualification"]["modes"]["node-lifetimes-default"].update(direct_root_tokio=0),
            lambda x: x["qualification"]["not_qualified"].pop(),
            lambda x: x["qualification"]["layout_evidence"]["limits"].update(cleanup_child_creation_limit=17),
            lambda x: x["qualification"]["layout_evidence"]["limits"].update(cleanup_reserve=204800),
            lambda x: x["qualification"]["layout_evidence"]["limits"].update(cleanup_root_bytes=24576),
            lambda x: x["qualification"]["layout_evidence"]["limits"].update(typed_root_payload_bytes=8192),
            lambda x: x["qualification"]["layout_evidence"]["fixed_policy"]["a2_cancellation"].update(arc=1),
            lambda x: x["qualification"]["layout_evidence"]["constructor_policy"]["future/storage_sync"].update(rule="I"),
            lambda x: x["qualification"]["layout_evidence"]["constructor_policy"]["root/driver"].update(ceiling_bytes=16384),
            lambda x: x["qualification"]["layout_evidence"]["constructor_policy"]["future/gate_dispatch"].update(ceiling_bytes=57344),
            lambda x: x["qualification"]["layout_evidence"]["constructor_policy"]["native/sync_capture"].update(ceiling_bytes=57344),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with lifetimes_fixture(root) as data:
                    mutate(data["profile"])
                    repin_lifetimes(root, data)
                    with self.assertRaises(ValueError):
                        p.load_pins(root, p.LIFETIMES)

    def test_immutable_prefix_and_actual_lock_are_required_for_every_stage_action(self):
        cases = [path for path, _ in p.LIFETIMES_BASE_IMAGES.values()] + [p.PATCH_PATH, p.FOUNDATION_PATCH, "Cargo.lock"]
        for name in cases:
            with self.subTest(path=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with lifetimes_fixture(root) as data:
                    path = root / name
                    if name == "Cargo.lock":
                        # The exact formerly admissible B1 before-image is
                        # historical only; this refusal precedes preparation.
                        path.write_bytes(b'version = 3\n[[package]]\nname = "celld"\nversion = "0.6.2"\ndependencies = []\n')
                        self.assertEqual(p.file_hash(path), data["profile"]["cargo_lock"]["before"]["sha256"])
                    else:
                        path.write_bytes(path.read_bytes() + b"\n")
                    for operation in (
                        lambda: p.load_pins(root, p.LIFETIMES),
                        lambda: p.inspect_archive(data["archive"], root, stage=p.LIFETIMES),
                        lambda: p.prepare(data["archive"], root, p.LIFETIMES),
                        lambda: p.prepare_assets(data["native"], data["binding"], root, p.LIFETIMES),
                    ):
                        with self.assertRaises(ValueError):
                            operation()
                    self.assertFalse((root / "runtime/source").exists())
                    self.assertFalse((root / "runtime/assets-node-lifetimes.json").exists())

    def test_third_patch_cannot_change_prefix_add_files_or_exceed_caps(self):
        mutations = (
            lambda x: x.update(format=True), lambda x: x.pop("accepted_b1_commit"),
            lambda x: x.update(accepted_b1_commit="0" * 40), lambda x: x["patches"].reverse(),
            lambda x: x["patches"].pop(), lambda x: x["patches"].append(copy.deepcopy(x["patches"][2])),
            lambda x: x["patches"][0]["files"][0]["after"].update(sha256="0" * 64),
            lambda x: x["patches"][1]["files"][0]["after"].update(sha256="0" * 64),
            lambda x: x["patches"][2].update(path="patches/other.patch"),
            lambda x: x["patches"][2].update(bytes=p.MAXIMUM_LIFETIMES_PATCH + 1),
            lambda x: x["patches"][2]["files"].reverse(),
            lambda x: x["patches"][2].update(files=[row for row in x["patches"][2]["files"] if row["path"] != "crates/celld/asyncrt.rs"]),
            lambda x: x["patches"][2]["files"][0].update(path="crates/celld/main.rs"),
            lambda x: x["patches"][2]["files"][0].update(mode="100755"),
            lambda x: x["patches"][2]["files"][0].update(before=None),
            lambda x: x["patches"][2]["files"][0]["after"].update(bytes=p.MAXIMUM_POSTIMAGE + 1),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with lifetimes_fixture(root) as data:
                    mutate(data["series"])
                    repin_lifetimes(root, data)
                    with self.assertRaises(ValueError):
                        p.prepare(data["archive"], root, p.LIFETIMES)
                    self.assertFalse((root / "runtime/source").exists())

        # Build actual valid diffs at each boundary. Padding is replacement
        # file content across two existing paths, never ignored patch garbage.
        for size in (p.MAXIMUM_FOUNDATION_PATCH + 1, p.MAXIMUM_LIFETIMES_PATCH, p.MAXIMUM_LIFETIMES_PATCH + 1):
            with self.subTest(patch_bytes=size), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with lifetimes_fixture(root) as data:
                    initial = (root / p.LIFETIMES_PATCH).read_bytes()
                    padding = size - len(initial)
                    self.assertGreater(padding, 0)
                    extras = dict(zip(p.LIFETIMES_PATHS[:2], (padding // 2, padding - padding // 2), strict=True))
                    predecessor = root / Path(p.LIFETIMES_MANIFEST).parent / "source"
                    final = {row["path"]: dict(row) for row in data["rows"]}
                    chunks = []
                    for row in data["series"]["patches"][2]["files"]:
                        name = row["path"]
                        before = (predecessor / name).read_bytes()
                        after = ("lifetimes " + name).encode() + b"x" * extras.get(name, 0) + b"\n"
                        self.assertLessEqual(len(after), p.MAXIMUM_POSTIMAGE)
                        row["after"] = p.image_identity(after)
                        chunks.append(("--- a/" + name + "\n+++ b/" + name + "\n@@ -1," + str(len(before.splitlines())) + " +1,1 @@\n").encode() + b"".join(b"-" + line for line in before.splitlines(keepends=True)) + b"+" + after)
                        final[name] = {"path": name, "mode": "100644", **p.image_identity(after)}
                    patch_bytes = b"".join(chunks)
                    self.assertEqual(len(patch_bytes), size)
                    rows = sorted(final.values(), key=lambda row: row["path"])
                    data["profile"]["qualification"]["tree"] = {"files": len(rows), "payload_bytes": sum(row["bytes"] for row in rows), "inventory_sha256": p.digest(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode())}
                    repin_lifetimes(root, data, patch_bytes)
                    if size <= p.MAXIMUM_LIFETIMES_PATCH:
                        prepared = p.prepare(data["archive"], root, p.LIFETIMES)
                        self.assertEqual(prepared["files"], rows)
                        self.assertEqual(p.verify_source(root, p.LIFETIMES), prepared)
                    else:
                        with self.assertRaises(ValueError):
                            p.prepare(data["archive"], root, p.LIFETIMES)
                        self.assertFalse((root / "runtime/source").exists())
                        self.assertFalse((root / "runtime/source-manifest.json").exists())

    def test_late_prefix_hash_postimage_and_final_inventory_must_agree(self):
        for mutation in ("wrong-predecessor", "sha-only", "inventory", "payload", "source", "enqueue-source", "late-patch"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with lifetimes_fixture(root) as data:
                    if mutation != "wrong-predecessor":
                        p.prepare(data["archive"], root, p.LIFETIMES)
                    row = next(entry for entry in data["series"]["patches"][2]["files"] if entry["path"] == "crates/celld/guarded_execution.rs")
                    if mutation == "wrong-predecessor":
                        row["before"] = copy.deepcopy(next(entry for entry in data["series"]["patches"][0]["files"] if entry["path"] == row["path"])["after"])
                    elif mutation == "sha-only":
                        row["before"]["sha256"] = "0" * 64
                    elif mutation == "inventory":
                        data["profile"]["qualification"]["tree"]["inventory_sha256"] = "0" * 64
                    elif mutation == "payload":
                        data["profile"]["qualification"]["tree"]["payload_bytes"] += 1
                    elif mutation == "source":
                        (root / "runtime/source" / row["path"]).write_bytes(b"changed\n")
                    elif mutation == "enqueue-source":
                        (root / "runtime/source/crates/celld/asyncrt.rs").write_bytes(b"changed\n")
                    else:
                        last = data["series"]["patches"][2]["files"][-1]
                        last["after"]["sha256"] = "0" * 64
                    repin_lifetimes(root, data)
                    with self.assertRaises(ValueError):
                        p.prepare(data["archive"], root, p.LIFETIMES)

    def test_native_assets_are_reused_with_a_new_receipt_and_immutable_old_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with lifetimes_fixture(root) as data:
                paths = [*sorted((root / "runtime/artifacts").iterdir()), root / p.LIFETIMES_ASSETS, root / "runtime/assets-node-foundation.json"]
                before = {str(path): (path.read_bytes(), path.stat().st_ino) for path in paths}
                result = p.prepare_assets(data["native"], data["binding"], root, p.LIFETIMES)
                self.assertEqual(result["profile"], p.LIFETIMES)
                self.assertEqual(result["assets"], p.verify_assets(root))
                self.assertEqual(result["foundation_receipt_sha256"], p.file_hash(root / p.LIFETIMES_ASSETS))
                self.assertEqual(result, p.verify_assets(root, p.LIFETIMES))
                self.assertEqual({str(path): (path.read_bytes(), path.stat().st_ino) for path in paths}, before)
                receipt = root / "runtime/assets-node-lifetimes.json"
                changed = copy.deepcopy(result)
                changed["format"] = True
                receipt.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    p.verify_assets(root, p.LIFETIMES)
                receipt.write_text(json.dumps(result))
                for path in (root / p.LIFETIMES_ASSETS, root / "runtime/assets-node-foundation.json", root / "runtime/artifacts/librusty_v8.a"):
                    old = path.read_bytes()
                    path.write_bytes(old + b"x")
                    with self.subTest(path=str(path)), self.assertRaises(ValueError):
                        p.verify_assets(root, p.LIFETIMES)
                    path.write_bytes(old)

    def test_modes_commands_and_exact_fifty_three_test_inventory(self):
        pins = {**historical_pins(), "profile": p.LIFETIMES}
        self.assertEqual(len(p.LIFETIMES_TESTS), 53)
        self.assertEqual(len(set(p.LIFETIMES_TESTS)), 53)
        self.assertEqual(p.LIFETIMES_TESTS[:21], p.FOUNDATION_TESTS)
        with patch.object(p, "network_command", side_effect=lambda argv: list(map(str, argv))):
            for mode, contract in p.LIFETIMES_MODES.items():
                self.assertEqual(p.stage_for_mode(mode), p.LIFETIMES)
                commands = p.build_commands(Path("/fixture"), Path("/toolchain"), pins, mode)
                for name in ("build", "clippy"):
                    argv = commands[name]
                    self.assertEqual(argv[argv.index("--target") + 1], p.TARGET)
                    self.assertIn("--locked", argv)
                    self.assertIn("--offline", argv)
                    self.assertEqual("--features" in argv, bool(contract["root_features"]))
                    if contract["root_features"]:
                        self.assertEqual(argv[argv.index("--features") + 1], "foks-celld-guarded/" + contract["root_features"][0])
                if mode.endswith("lifecycle"):
                    self.assertIn("--all-targets", commands["clippy"])
                    self.assertIn("lifecycle", commands["lifecycle"])
                    self.assertIn("--test-threads=1", commands["lifecycle"])
                else:
                    self.assertEqual(commands["build"].count("--package"), 2)
                    self.assertEqual(commands["build"][commands["build"].index("--bin") + 1], "celld")
                    self.assertNotIn("lifecycle", commands)
            tests = p.foundation_test_commands(Path("/fixture"), Path("/toolchain"), pins)
            self.assertIn(p.LIFETIMES_TEST_PREFIX, tests["list"])
            self.assertNotIn(p.FOUNDATION_TEST_PREFIX, tests["list"])
            self.assertIn("--test-threads=1", tests["run"])
            self.assertIn("--lib", tests["run"])
            self.assertIn("--list", tests["list"])
            for old in ("identity", "lifecycle", "node-baseline", *p.FOUNDATION_MODES):
                with self.subTest(old=old), self.assertRaises(ValueError):
                    p.selected_mode(pins, old)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "output"
            outputs = {
                True: "".join(name + ": test\n" for name in p.LIFETIMES_TESTS) + "\n53 tests, 0 benchmarks\n",
                False: "running 53 tests\n" + "".join("test " + name + " ... ok\n" for name in p.LIFETIMES_TESTS) + "\ntest result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.01s\n",
            }
            for listing, raw in outputs.items():
                path.write_text(raw)
                self.assertEqual(p.foundation_test_output(path, listing, p.LIFETIMES), p.LIFETIMES_TESTS)
                for changed in (raw.replace(p.LIFETIMES_TESTS[-1], "unreviewed"), raw + raw, raw.replace("53", "52"), raw.replace(p.LIFETIMES_TESTS[-1], p.LIFETIMES_TESTS[0]), raw + "ignored\n", raw.replace("0 ignored", "1 ignored")):
                    if changed == raw:
                        continue
                    path.write_text(changed)
                    with self.subTest(listing=listing), self.assertRaises(ValueError):
                        p.foundation_test_output(path, listing, p.LIFETIMES)
                path.write_text(raw)
                with self.assertRaises(ValueError):
                    p.foundation_test_output(path, listing)

    @staticmethod
    def layout_record():
        fixed = {name: {"bytes": 32, "alignment": 8, "arc_bytes": 48 if arc else 0, "bucket": bucket} for name, (bucket, arc, _) in p.LIFETIMES_FIXED_POLICY.items()}
        fixed["a2_cancellation"].update(bytes=1, alignment=1, arc_bytes=24)
        fixed["a2_events"].update(bytes=4096)
        constructors = {}
        for name, (rule, lanes) in p.LIFETIMES_CONSTRUCTOR_POLICY.items():
            constructors[name] = {
                "capture_bytes": 32, "capture_alignment": 8, "future_bytes": 16 if rule == "R" else 0, "future_alignment": 8 if rule == "R" else 1,
                "result_bytes": 16, "result_alignment": 8, "channel_payload_bytes": 0 if rule == "I" else 24, "channel_payload_alignment": 1 if rule == "I" else 8,
                "channel_bound_bytes": 0 if rule == "I" else 112, "owned_capacity_bound_bytes": 8080 if rule == "R" else 0,
                "admitted_overlap_bytes": {"R": 8256, "N": 160, "I": 0}[rule], "ceiling_bytes": {"R": 57344, "N": 512, "I": 8192}[rule],
                "creation_limit": {lane: "inline" if rule == "I" else "cumulative" if lane.startswith("F") else 16 if rule == "N" else 1 for lane in lanes},
                "lanes": list(lanes), "bucket": "inline-observation" if rule == "I" else "invocation",
            }
        return {"format": 1, "claim": "b2-constructor-layouts-only", "arc_convention": "repr-c-two-atomic-usize", "fixed": fixed, "constructors": constructors, "controls": {name: {"bytes": 32, "alignment": 8, "ceiling_bytes": 2048} for name in p.LIFETIMES_CONTROL_NAMES}, "limits": dict(p.LIFETIMES_LIMITS)}

    def test_compiled_lifetimes_layout_records_bind_additive_and_inline_rules(self):
        record = self.layout_record()
        self.assertEqual((len(record["fixed"]), len(record["constructors"])), (16, 36))
        self.assertEqual(record["constructors"]["root/wake"]["creation_limit"], {"F7": "cumulative", "C5": 1})
        self.assertEqual(record["constructors"]["native/sync_capture"]["creation_limit"], {"F1": "cumulative", "C0": 16, "C1": 16, "C6": 16})
        self.assertEqual(record["constructors"]["future/gate_dispatch"]["lanes"], [])
        self.assertEqual((record["limits"]["typed_root_payload_bytes"], record["limits"]["cleanup_root_bytes"], record["limits"]["cleanup_reserve"]), (49152, 65536, 532480))
        b1_record = {**FoundationTests.layout_record(), "cleanup_reserve": 532480}
        b1 = b"FOKS_B1_LAYOUT " + json.dumps(b1_record).encode() + b"\n"
        prefix = b"FOKS_B2_LAYOUT "
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "tests-run.stderr"

            def observe(value):
                tagged = prefix + json.dumps(value, separators=(",", ":")).encode() + b"\n"
                raw = b"Cargo output\n" + b1 + tagged + b"native diagnostic\n"
                path.write_bytes(raw)
                result = p.lifetimes_layout_output(path, hashlib.sha256(raw).hexdigest())
                self.assertEqual(result, {"record": value, "raw_record_bytes": len(tagged), "raw_record_sha256": hashlib.sha256(tagged).hexdigest()})
                self.assertEqual(p.foundation_layout_output(path, hashlib.sha256(raw).hexdigest(), p.LIFETIMES)["record"], b1_record)
                self.assertEqual(path.read_bytes(), raw)
                return result

            observe(record)
            maxima = copy.deepcopy(record)
            for name, (_, arc, ceiling) in p.LIFETIMES_FIXED_POLICY.items():
                maxima["fixed"][name].update(bytes=ceiling - 16 if arc else ceiling, alignment=16, arc_bytes=ceiling if arc else 0)
            for name, (rule, _) in p.LIFETIMES_CONSTRUCTOR_POLICY.items():
                entry = maxima["constructors"][name]
                entry.update(capture_bytes={"N": 400, "I": 8192, "R": 49152}[rule], capture_alignment=16, future_bytes=0, future_alignment=1, result_bytes=0, result_alignment=1)
                if rule == "R":
                    entry.update(channel_payload_bytes=8112, channel_payload_alignment=16, channel_bound_bytes=8192, owned_capacity_bound_bytes=0, admitted_overlap_bytes=57344)
                elif rule == "N":
                    entry.update(channel_payload_bytes=32, channel_payload_alignment=16, admitted_overlap_bytes=512)
            for entry in maxima["controls"].values():
                entry.update(bytes=2048, alignment=16)
            observe(maxima)

            # Every R row admits the same actual full-future bound, including
            # cleanup lanes and the separately charged storage-sync future.
            for name, (rule, _) in p.LIFETIMES_CONSTRUCTOR_POLICY.items():
                if rule != "R":
                    continue
                for field in ("capture", "future", "result"):
                    boundary = copy.deepcopy(record)
                    entry = boundary["constructors"][name]
                    entry.update(capture_bytes=0, capture_alignment=1, future_bytes=0, future_alignment=1, result_bytes=0, result_alignment=1, admitted_overlap_bytes=57344)
                    entry.update({field + "_bytes": 49152, field + "_alignment": 16})
                    observe(boundary)
                    entry.update({field + "_bytes": 49153, field + "_alignment": 1, "admitted_overlap_bytes": 57345})
                    with self.subTest(name=name, field=field), self.assertRaises(ValueError):
                        observe(boundary)

            malformed = []
            for field in record:
                altered = copy.deepcopy(record)
                del altered[field]
                malformed.append(("missing " + field, altered))
            for field, value in (("format", True), ("claim", "ready"), ("arc_convention", "other"), ("extra", 0)):
                malformed.append(("top " + field, {**record, field: value}))
            for section in ("fixed", "constructors", "controls"):
                for name in record[section]:
                    altered = copy.deepcopy(record)
                    del altered[section][name]
                    malformed.append(("missing " + name, altered))
                altered = copy.deepcopy(record)
                altered[section]["unreviewed"] = next(iter(altered[section].values()))
                malformed.append(("extra " + section, altered))
            for key in record["limits"]:
                for value in (True, record["limits"][key] + 1):
                    altered = copy.deepcopy(record)
                    altered["limits"][key] = value
                    malformed.append(("limit " + key, altered))
            for name, original in record["fixed"].items():
                for field, value in (("bytes", 0), ("bytes", -1), ("bytes", True), ("bytes", 1048577), ("alignment", 0), ("alignment", True), ("alignment", 3), ("alignment", 32), ("arc_bytes", original["arc_bytes"] + 8), ("arc_bytes", False), ("bucket", "unreviewed"), ("extra", 0)):
                    altered = copy.deepcopy(record)
                    altered["fixed"][name][field] = value
                    malformed.append((name + " " + field, altered))
            altered = copy.deepcopy(record)
            altered["fixed"]["a2_events"]["bytes"] = 4104
            malformed.append(("event backing cardinality", altered))
            for name, original in record["constructors"].items():
                changes = [(field, original[field] + 1) for field in ("channel_bound_bytes", "owned_capacity_bound_bytes", "admitted_overlap_bytes", "ceiling_bytes")]
                changes += [("capture_bytes", True), ("capture_bytes", -1), ("capture_bytes", 33), ("capture_bytes", 1 << 64), ("future_bytes", 49153), ("channel_payload_bytes", 1 << 64), ("result_alignment", 3), ("channel_payload_alignment", 32), ("lanes", [] if original["lanes"] else ["F5"]), ("creation_limit", {} if original["creation_limit"] else {"F5": 1}), ("bucket", "process"), ("extra", 0)]
                for field, value in changes:
                    altered = copy.deepcopy(record)
                    altered["constructors"][name][field] = value
                    malformed.append((name + " " + field, altered))
            for section, name in (("fixed", "invocation_record"), ("constructors", "root/driver"), ("controls", "retirement")):
                for field in record[section][name]:
                    altered = copy.deepcopy(record)
                    del altered[section][name][field]
                    malformed.append(("missing row field " + section + " " + field, altered))
            for name in record["controls"]:
                for field, value in (("bytes", 2056), ("bytes", False), ("alignment", 32), ("ceiling_bytes", 4096), ("extra", 0)):
                    altered = copy.deepcopy(record)
                    altered["controls"][name][field] = value
                    malformed.append((name + " " + field, altered))
            for label, value in malformed:
                with self.subTest(case=label), self.assertRaises(ValueError):
                    observe(value)

    def test_compiled_lifetimes_records_reject_forged_overlap_even_with_consistent_policy_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stderr"
            mutations = (
                ("root/driver", {"capture_bytes": 49128, "admitted_overlap_bytes": 57352}),
                ("root/driver", {"capture_bytes": 49121, "capture_alignment": 1, "admitted_overlap_bytes": 57345}),
                ("root/driver", {"channel_payload_bytes": 8120, "channel_bound_bytes": 8208, "owned_capacity_bound_bytes": -16}),
                ("native/sync_capture", {"future_bytes": 8, "future_alignment": 8}),
                ("native/sync_capture", {"capture_bytes": 392, "result_bytes": 16, "admitted_overlap_bytes": 520}),
                ("inline/publish", {"channel_payload_bytes": 8, "channel_payload_alignment": 8}),
                ("future/gate_dispatch", {"capture_bytes": 8192, "result_bytes": 16}),
                ("future/storage_sync", {"channel_payload_bytes": 0, "channel_payload_alignment": 1, "channel_bound_bytes": 0, "owned_capacity_bound_bytes": 8192}),
                ("root/wake", {"creation_limit": {"F7": "cumulative", "C5": True}}),
                ("native/sync_capture", {"lanes": ["F1", "C1", "C0", "C6"]}),
            )
            for name, changed in mutations:
                record = self.layout_record()
                record["constructors"][name].update(changed)
                raw = b"FOKS_B2_LAYOUT " + json.dumps(record).encode() + b"\n"
                path.write_bytes(raw)
                with self.subTest(name=name, changed=changed), self.assertRaises(ValueError):
                    p.lifetimes_layout_output(path, p.digest(raw))

    def test_compiled_lifetimes_stderr_is_bounded_hash_bound_and_strictly_framed(self):
        prefix = b"FOKS_B2_LAYOUT "
        tagged = prefix + json.dumps(self.layout_record(), separators=(",", ":")).encode() + b"\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stderr"
            padded = tagged[:-1] + b" " * (65536 - len(tagged)) + b"\n"
            self.assertEqual(len(padded), 65536)
            path.write_bytes(padded)
            self.assertEqual(p.lifetimes_layout_output(path, p.digest(padded))["raw_record_bytes"], 65536)
            malformed = (
                b"ordinary output\n", tagged + tagged, b" " + tagged, tagged[:-1], tagged.replace(b"\n", b"\r\n"),
                tagged.replace(prefix, b"FOKS_B2_LAYOUT\t", 1), prefix + b"\xff\n", prefix + b"[]\n",
                tagged.replace(b'"format":1', b'"format":1,"format":1', 1),
                tagged.replace(b'"bytes":32', b'"bytes":32,"bytes":32', 1),
                tagged.replace(b'"bytes":32', b'"bytes":1e999', 1),
                tagged.replace(b'"bytes":32', b'"bytes":32.0', 1),
                tagged.replace(b'"bytes":32', b'"bytes":NaN', 1),
                padded[:-1] + b" \n", b"x" * (1024 * 1024) + b"\n" + tagged,
            )
            for index, raw in enumerate(malformed):
                path.write_bytes(raw)
                with self.subTest(index=index), self.assertRaises(ValueError):
                    p.lifetimes_layout_output(path, p.digest(raw))
            raw = b"Cargo diagnostic\n" + tagged
            path.write_bytes(raw)
            for expected in (None, True, "A" * 64, "0" * 64):
                with self.subTest(expected=expected), self.assertRaises(ValueError):
                    p.lifetimes_layout_output(path, expected)
            path.write_bytes(b"other diagnostic\n" + tagged)
            with self.assertRaisesRegex(ValueError, "differs from owned command"):
                p.lifetimes_layout_output(path, p.digest(raw))
            link = Path(directory) / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                p.lifetimes_layout_output(link, p.file_hash(path))


class AssetTests(unittest.TestCase):
    def test_exact_local_assets_and_expanded_archive_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _ = fixture(root)
            archive, binding = root / "v8.gz", root / "binding.rs"
            archive.write_bytes(gzip.compress(b"!<arch>\n" + b"native" * 100))
            binding.write_bytes(b"// reviewed binding\n")
            for key, path in (("archive", archive), ("binding", binding)):
                pins["v8"][key].update(bytes=path.stat().st_size, sha256=p.file_hash(path))
            save_pins(root, pins)
            actual = p.prepare_assets(archive, binding, root)
            self.assertEqual(actual, p.verify_assets(root))
            self.assertEqual(actual["expanded"]["bytes"], 608)
            (root / "runtime/artifacts/librusty_v8.a").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "size/hash"):
                p.verify_assets(root)

    def test_expansion_ceiling_header_and_gzip_integrity(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "native.gz"
            archive.write_bytes(gzip.compress(b"!<arch>\n" + b"x" * 1000))
            with self.assertRaisesRegex(ValueError, "ceiling"):
                p.expanded_archive(archive, 10)
            archive.write_bytes(gzip.compress(b"not an archive"))
            with self.assertRaisesRegex(ValueError, "static archive"):
                p.expanded_archive(archive, 100)
            archive.write_bytes(gzip.compress(b"!<arch>\n")[:-4])
            with self.assertRaises((EOFError, OSError)):
                p.expanded_archive(archive, 100)

    def test_failed_asset_expansion_removes_only_new_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _ = fixture(root)
            archive, binding = root / "v8.gz", root / "binding.rs"
            archive.write_bytes(gzip.compress(b"!<arch>\n" + b"x" * 20))
            binding.write_bytes(b"binding")
            for key, path in (("archive", archive), ("binding", binding)):
                pins["v8"][key].update(bytes=path.stat().st_size, sha256=p.file_hash(path))
            pins["v8"]["expanded_maximum_bytes"] = 10
            save_pins(root, pins)
            with self.assertRaisesRegex(ValueError, "ceiling"):
                p.prepare_assets(archive, binding, root)
            self.assertFalse((root / "runtime/artifacts").exists())
            self.assertFalse(list((root / "runtime").glob("assets-staging-*")))


class ControlTests(unittest.TestCase):
    def test_ambient_build_bypasses_are_rejected_before_tool_use(self):
        for name in ("V8_FROM_SOURCE", "V8_FORCE_DEBUG", "RUSTY_V8_MIRROR", "DOCS_RS", "DENO_TRYBUILD", "CARGO_TARGET_DIR", "CARGO_HOME", "RUSTFLAGS", "RUSTC_WRAPPER", "GN_ARGS", "CC", "DYLD_INSERT_LIBRARIES"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "environment variable"):
                p.controlled_environment(ambient={name: "value"})

    def test_disk_admission_and_running_floor_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _ = fixture(root)
            build = pins["build"]
            measured = {"free_bytes": build["minimum_free_bytes"] - 1, "target_bytes": 0}
            with patch.object(p, "disk_status", return_value=measured):
                self.assertIsNotNone(p.resource_refusal(root, initial=True)[0])
                self.assertIsNone(p.resource_refusal(root)[0])
            measured["free_bytes"] = build["free_floor_bytes"] - 1
            with patch.object(p, "disk_status", return_value=measured):
                self.assertIsNotNone(p.resource_refusal(root)[0])
            measured.update(free_bytes=build["minimum_free_bytes"], target_bytes=build["maximum_target_bytes"] + 1)
            with patch.object(p, "disk_status", return_value=measured):
                self.assertIsNotNone(p.resource_refusal(root)[0])

    def test_a2_initial_admission_reuses_only_observed_target_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _, _, _ = a2_fixture(root)
            maximum = pins["build"]["maximum_target_bytes"]
            floor = pins["build"]["free_floor_bytes"]
            for target in (0, 1, maximum // 2, maximum - 1, maximum):
                required = floor + maximum - target
                for extra in (-1, 0, 1):
                    measured = {"free_bytes": required + extra, "target_bytes": target}
                    with self.subTest(target=target, extra=extra), patch.object(p, "disk_status", return_value=measured) as sample:
                        refusal, actual = p.resource_refusal(root, initial=True)
                        sample.assert_called_once_with(root)
                        self.assertEqual(refusal is None, extra >= 0)
                        self.assertEqual(actual["required_free_bytes"], required)
                        self.assertEqual(measured, {"free_bytes": required + extra, "target_bytes": target})
            # Source/assets/cache bytes never enter the measured target term.
            with patch.object(p, "disk_status", return_value={"free_bytes": floor, "target_bytes": 0}):
                self.assertIsNotNone(p.resource_refusal(root, initial=True)[0])

    def test_a2_oversize_target_refuses_before_remaining_allowance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _, _, _ = a2_fixture(root)
            for free in (0, 100 * 1024**3):
                measured = {"free_bytes": free, "target_bytes": pins["build"]["maximum_target_bytes"] + 1}
                with self.subTest(free=free), patch.object(p, "disk_status", return_value=measured):
                    self.assertEqual(p.resource_refusal(root, initial=True), ("target working allowance exceeded", measured))

    def test_a2_running_floor_and_target_ceiling_are_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pins, _, _, _, _ = a2_fixture(root)
            floor, maximum = pins["build"]["free_floor_bytes"], pins["build"]["maximum_target_bytes"]
            for free, target, allowed in ((floor - 1, 0, False), (floor, 0, True), (floor, maximum, True), (floor, maximum + 1, False)):
                measured = {"free_bytes": free, "target_bytes": target}
                with self.subTest(free=free, target=target), patch.object(p, "disk_status", return_value=measured):
                    refusal, actual = p.resource_refusal(root)
                    self.assertEqual(refusal is None, allowed)
                    self.assertEqual(actual, measured)

    def test_resource_observation_unknown_and_errors_never_supply_capacity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            a2_fixture(root)
            for measured in (None, {}, {"free_bytes": 10**12}, {"free_bytes": 10**12, "target_bytes": None}, {"free_bytes": 10**12, "target_bytes": -1}, {"free_bytes": True, "target_bytes": 0}, {"free_bytes": 10**12, "target_bytes": 0, "reclaimable_bytes": 10**12}):
                with self.subTest(measured=measured), patch.object(p, "disk_status", return_value=measured), self.assertRaisesRegex(ValueError, "observation"):
                    p.resource_refusal(root, initial=True)
            for error in (PermissionError("unreadable"), OSError("failed scan")):
                with patch.object(p, "disk_status", side_effect=error), self.assertRaises(type(error)) as actual:
                    p.resource_refusal(root, initial=True)
                self.assertIs(actual.exception, error)

    def test_size_scan_rejects_non_directory_root_and_special_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / "file"
            file.write_bytes(b"not a target directory")
            with self.assertRaisesRegex(ValueError, "not a directory"):
                p.directory_bytes(file)
            os.mkfifo(root / "fifo")
            with self.assertRaisesRegex(ValueError, "special file"):
                p.directory_bytes(root)

    def test_mode_selects_exact_owned_build_lint_and_serial_test_commands(self):
        root, toolchain = Path("/fixture"), Path("/toolchain")
        for profile, mode in ((p.A1, "identity"), (p.A2, "identity"), (p.A2, "lifecycle")):
            pins = historical_pins()
            pins["profile"] = profile
            with self.subTest(profile=profile, mode=mode), patch.object(p, "network_command", side_effect=lambda argv: list(map(str, argv))):
                commands = p.build_commands(root, toolchain, pins, mode)
                self.assertEqual(set(commands), {"build", "clippy", *(["lifecycle"] if mode == "lifecycle" else [])})
                for argv in commands.values():
                    self.assertEqual(argv[0], str(toolchain / "bin/cargo"))
                    self.assertIn("--locked", argv)
                    self.assertIn("--offline", argv)
                    self.assertIn("--no-default-features", argv)
                    self.assertEqual(argv[argv.index("--target") + 1], p.TARGET)
                    self.assertEqual(argv[argv.index("-j") + 1], "2")
                    self.assertEqual("--features" in argv, mode == "lifecycle")
                    if mode == "lifecycle":
                        self.assertEqual(argv[argv.index("--features") + 1], "lifecycle")
                self.assertEqual(commands["clippy"][-3:], ["--", "-D", "warnings"])
                if mode == "lifecycle":
                    self.assertIn("--all-targets", commands["clippy"])
                    self.assertEqual(commands["lifecycle"][-5:], ["--test", "lifecycle", "--", "--test-threads=1", "--nocapture"])
                else:
                    self.assertEqual(commands["build"][-2:], ["--bin", "foks-celld-guarded"])
        with self.assertRaisesRegex(ValueError, "mode"):
            p.build_commands(root, toolchain, historical_pins(), "lifecycle")
        with self.assertRaisesRegex(ValueError, "mode"):
            p.build_commands(root, toolchain, historical_pins(), "unknown")

    def test_existing_slot_and_symlink_target_are_never_stolen(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with p.BuildSlot(root):
                with self.assertRaises(FileExistsError):
                    with p.BuildSlot(root):
                        self.fail("stolen slot")
            (root / "target").symlink_to(root / "runtime")
            with self.assertRaisesRegex(ValueError, "symlink"):
                p.directory_bytes(root / "target")

    def test_live_size_scan_uses_one_stat_and_tolerates_vanished_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, after = root / "removed-before-stat", root / "removed-after-stat"
            before.write_bytes(b"unobserved")
            after.write_bytes(b"observed")
            self.assertEqual(p.directory_bytes(root / "absent"), 0)
            original_lstat = Path.lstat
            calls = {}
            def disappearing(item):
                calls[item] = calls.get(item, 0) + 1
                if item == before:
                    item.unlink()
                result = original_lstat(item)
                if item == after:
                    item.unlink()
                return result
            with patch.object(Path, "lstat", disappearing):
                self.assertEqual(p.directory_bytes(root), len(b"observed"))
            self.assertEqual(calls, {root: 1, before: 1, after: 1})
            self.assertFalse(before.exists() or after.exists())

    def test_live_size_scan_tolerates_directory_removed_before_descent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            removed = root / "removed"
            removed.mkdir()
            (root / "retained").write_bytes(b"retained")
            original_scandir = os.scandir
            removals = []
            def disappearing(path):
                if Path(path) == removed:
                    removed.rmdir()
                    removals.append(removed)
                return original_scandir(path)
            with patch.object(p.os, "scandir", side_effect=disappearing):
                self.assertEqual(p.directory_bytes(root), len(b"retained"))
            self.assertEqual(removals, [removed])
            self.assertFalse(removed.exists())

    def test_live_size_scan_propagates_walk_and_stat_observation_errors(self):
        for kind in ("walk", "root-stat", "entry-stat"):
            for error in (PermissionError(errno.EACCES, "denied"), OSError(errno.EIO, "observation failed")):
                with self.subTest(kind=kind, error=type(error).__name__), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    item = root / "item"
                    item.write_bytes(b"present")
                    original_lstat = Path.lstat
                    def refuse_stat(path):
                        if path == (root if kind == "root-stat" else item):
                            raise error
                        return original_lstat(path)
                    selected = patch.object(p.os, "scandir", side_effect=error) if kind == "walk" else patch.object(Path, "lstat", refuse_stat)
                    with selected, self.assertRaises(type(error)) as raised:
                        p.directory_bytes(root)
                    self.assertIs(raised.exception, error)

    def test_live_size_scan_rejects_root_and_interior_symlinks(self):
        for kind in ("root", "directory", "file"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                outside = root / "outside"
                outside.mkdir()
                (outside / "file").write_bytes(b"outside")
                target = root / "target"
                if kind == "root":
                    target.symlink_to(outside, target_is_directory=True)
                else:
                    target.mkdir()
                    (target / "link").symlink_to(outside if kind == "directory" else outside / "file", target_is_directory=kind == "directory")
                with self.assertRaisesRegex(ValueError, "symlink"):
                    p.directory_bytes(target)

    def test_success_holds_slot_until_real_group_is_gone(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with p.BuildSlot(root) as slot:
                record = p.run_owned([sys.executable, "-c", "print('actual child')"], root, dict(os.environ), root / "evidence", "success", slot, 10)
                self.assertTrue(slot.path.exists())
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))
            self.assertFalse(slot.path.exists())

    def test_parent_exit_is_not_cleanup_and_sigterm_ignoring_child_is_killed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            child = "import os,signal; signal.signal(signal.SIGTERM,signal.SIG_IGN); os.write(1,b'ready\\n'); signal.pause()"
            parent = "import os,subprocess,sys; child=subprocess.Popen([sys.executable,'-c'," + repr(child) + "],stdout=subprocess.PIPE); assert child.stdout.readline()==b'ready\\n'; os._exit(0)"
            with p.BuildSlot(root) as slot:
                with self.assertRaisesRegex(ValueError, "outstanding children"):
                    p.run_owned([sys.executable, "-c", parent], root, dict(os.environ), root / "evidence", "orphan", slot, 10, term_grace=0.2, kill_grace=3)
                record = p.read_json(root / "evidence/orphan.command.json")
                self.assertEqual(record["exit_code"], 0)
                self.assertTrue(record["cleanup_established"])
                self.assertIn("SIGKILL", [event["signal"] for event in record["events"]])
                self.assertFalse(p.group_alive(record["process_group"]))
            self.assertFalse(slot.path.exists())

    def test_resource_refusal_after_child_barrier_cleans_group(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / "ready"
            child = "import pathlib,signal; pathlib.Path(" + repr(str(ready)) + ").write_text('ready'); signal.pause()"
            monitor = lambda: ("controlled resource refusal" if ready.exists() else None, {})
            with p.BuildSlot(root) as slot:
                with self.assertRaisesRegex(ValueError, "controlled resource refusal"):
                    p.run_owned([sys.executable, "-c", child], root, dict(os.environ), root / "evidence", "resource", slot, 10, monitor)
                record = p.read_json(root / "evidence/resource.command.json")
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))
            self.assertFalse(slot.path.exists())

    def test_permission_denied_probe_is_not_disappearance(self):
        with patch.object(p.os, "killpg", side_effect=PermissionError("denied")):
            self.assertTrue(p.group_alive(999999999))
            events = []
            p.signal_group(999999999, signal.SIGTERM, events)
            self.assertEqual(events, [{"signal": "SIGTERM", "group": 999999999, "permission_denied": True}])

    def test_transient_permission_denial_waits_for_real_group_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / "ready"
            child = "import pathlib,signal; pathlib.Path(" + repr(str(ready)) + ").write_text('ready'); signal.pause()"
            monitor = lambda: ("controlled resource refusal" if ready.exists() else None, {})
            original_killpg = os.killpg
            signalled = False
            denied_probe = False
            def transient(group, selected):
                nonlocal signalled, denied_probe
                if selected == signal.SIGTERM and not signalled:
                    original_killpg(group, selected)
                    signalled = True
                    raise PermissionError("injected dying-group signal race")
                if selected == 0 and signalled and not denied_probe:
                    denied_probe = True
                    raise PermissionError("injected dying-group probe race")
                return original_killpg(group, selected)
            with p.BuildSlot(root) as slot, patch.object(p.os, "killpg", side_effect=transient):
                with self.assertRaisesRegex(ValueError, "controlled resource refusal"):
                    p.run_owned([sys.executable, "-c", child], root, dict(os.environ), root / "evidence", "transient-permission", slot, 10, monitor)
                record = p.read_json(root / "evidence/transient-permission.command.json")
                self.assertTrue(signalled and denied_probe)
                self.assertTrue(record["cleanup_established"])
                self.assertIsNotNone(record["exit_code"])
                self.assertTrue(any(event.get("permission_denied") for event in record["events"]))
            self.assertFalse(p.group_alive(record["process_group"]))
            self.assertFalse(slot.path.exists())

    def test_persistent_permission_denial_cannot_release_cleanup_credit(self):
        process = unittest.mock.Mock(pid=999999999)
        process.poll.return_value = 0
        with patch.object(p.os, "killpg", side_effect=PermissionError("persistent denial")), patch.object(p, "observe_children", return_value=[]):
            events = []
            clean, escaped = p.finish_group(process, {process.pid}, events, 0, 0)
            self.assertFalse(clean)
            self.assertEqual(escaped, [])
            self.assertEqual([event["signal"] for event in events], ["SIGTERM", "SIGKILL"])
            self.assertTrue(all(event["permission_denied"] for event in events))

    def test_timeout_and_wrapper_exception_clean_before_slot_reuse(self):
        for kind in ("timeout", "exception"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                def monitor():
                    raise KeyboardInterrupt("controlled wrapper interruption")
                with p.BuildSlot(root) as slot:
                    with self.assertRaises(ValueError):
                        p.run_owned([sys.executable, "-c", "import signal; signal.pause()"], root, dict(os.environ), root / "evidence", kind, slot, 0.2 if kind == "timeout" else 10, None if kind == "timeout" else monitor)
                    record = p.read_json(root / "evidence" / (kind + ".command.json"))
                    self.assertTrue(record["cleanup_established"])
                    self.assertFalse(p.group_alive(record["process_group"]))
                with p.BuildSlot(root):
                    pass

    def test_real_wrapper_signal_after_child_barrier_cancels_entire_group(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / "ready"
            sent = False
            child = "import pathlib,signal; pathlib.Path(" + repr(str(ready)) + ").write_text('ready'); signal.pause()"
            def monitor():
                nonlocal sent
                if ready.exists() and not sent:
                    sent = True
                    os.kill(os.getpid(), signal.SIGTERM)
                return None, {}
            with p.BuildSlot(root) as slot:
                with self.assertRaisesRegex(ValueError, "wrapper cancelled"):
                    p.run_owned([sys.executable, "-c", child], root, dict(os.environ), root / "evidence", "signal", slot, 10, monitor)
                record = p.read_json(root / "evidence/signal.command.json")
                self.assertTrue(sent)
                self.assertEqual(record["reason"], "wrapper cancelled")
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))
            with p.BuildSlot(root):
                pass

    def test_final_evidence_failure_restores_handlers_after_physical_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result_path = root / "evidence/final-write.command.json"
            original_write = p.write_json
            handlers = {selected: signal.getsignal(selected) for selected in (signal.SIGINT, signal.SIGTERM)}
            final_record = {}
            def refuse_final_write(path, value):
                if Path(path) == result_path and value.get("cleanup_established") is True:
                    final_record.update(value)
                    raise OSError("injected final evidence failure")
                return original_write(path, value)
            with p.BuildSlot(root) as slot, patch.object(p, "write_json", side_effect=refuse_final_write):
                with self.assertRaisesRegex(OSError, "injected final evidence failure"):
                    p.run_owned([sys.executable, "-c", "print('done')"], root, dict(os.environ), root / "evidence", "final-write", slot, 10)
                self.assertEqual(final_record["reason"], "completed")
                self.assertEqual(final_record["exit_code"], 0)
                self.assertTrue(final_record["cleanup_established"])
                self.assertFalse(p.group_alive(final_record["process_group"]))
                for selected, handler in handlers.items():
                    self.assertEqual(signal.getsignal(selected), handler)
                self.assertTrue(slot.clean)
            self.assertFalse(slot.path.exists())
            with p.BuildSlot(root):
                pass

    def test_uncertain_cleanup_and_observed_escape_quarantine_the_slot(self):
        for kind in ("uncertain", "escape"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                original_finish = p.finish_group
                def uncertain(*args):
                    original_finish(*args)  # really clean the controlled group
                    return False, []
                selected = patch.object(p, "finish_group", side_effect=uncertain) if kind == "uncertain" else patch.object(p, "observe_children", return_value=[999999999])
                with selected, p.BuildSlot(root) as slot:
                    with self.assertRaisesRegex(ValueError, "cleanup uncertain"):
                        p.run_owned([sys.executable, "-c", "print('done')"], root, dict(os.environ), root / "evidence", kind, slot, 10)
                self.assertTrue(slot.path.exists())
                self.assertEqual(p.read_json(slot.path)["status"], "cleanup uncertain; slot quarantined")
                with self.assertRaises(FileExistsError):
                    with p.BuildSlot(root):
                        self.fail("reused quarantined slot")


class NodeInputTests(unittest.TestCase):
    def profile(self):
        return json.loads((ROOT / "node.lock.json").read_text())

    def test_profile_is_closed_typed_and_bound_to_actual_inputs(self):
        profile = self.profile()
        p.validate_node_profile(profile)
        self.assertEqual(p.file_hash(ROOT / "node.lock.json"), p.NODE_PROFILE_SHA256)
        # Source parsing does not import or execute the controller/fixture.
        self.assertEqual(p.node_program(ROOT), profile["program"])
        self.assertEqual(p.node_process_inspector(ROOT), profile["launch"]["process_inspection"]["source"])
        mutations = [
            lambda value: value.update(extra=True),
            lambda value: value.update(format=True),
            lambda value: value.update(accepted_a2_commit="0" * 40),
            lambda value: value["limits"].update(scenario_seconds=601),
            lambda value: value["limits"].update(maximum_nodes=True),
            lambda value: value["network"].update(template="(allow network*)"),
            lambda value: value["launch"].update(flags=["-I", "-B"]),
            lambda value: value["launch"]["process_inspection"].update(extra=True),
            lambda value: value["launch"]["process_inspection"].update(library="/tmp/libproc.dylib"),
            lambda value: value["launch"]["process_inspection"]["abi"].update(arg=0),
            lambda value: value["launch"]["process_inspection"]["abi"].update(arg=True),
            lambda value: value["launch"]["process_inspection"]["limits"].update(pid_capacity=8192),
            lambda value: value["launch"]["process_inspection"]["headers"][0].update(path="/tmp/libproc.h"),
            lambda value: value["fixture"]["inventory_limits"].update(bytes=2**31),
            lambda value: value["fixture"]["trees"][0].update(entries=True),
            lambda value: value["fixture"]["aliases"][0].update(extra="ignored"),
            lambda value: value["fixture"]["startup"][0].update(required="ignore"),
        ]
        for mutation in mutations:
            changed = copy.deepcopy(profile)
            mutation(changed)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                p.validate_node_profile(changed)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "node.lock.json").write_text(json.dumps(profile))
            with self.assertRaisesRegex(ValueError, "immutable bytes"):
                p.load_node_profile(root)
            (root / "node.lock.json").write_text('{"format":1,"format":1}')
            with self.assertRaisesRegex(ValueError, "duplicate"):
                p.load_node_profile(root)
            (root / "node.lock.json").write_bytes(b" " * (128 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "oversized"):
                p.load_node_profile(root)

    def test_program_extraction_allows_only_unique_literal_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "run-node.py"
            good = "WORKER_JS = b'actual program'\nWRANGLER_JSON = b'{}'\nraise RuntimeError('must never execute')\n"
            source.write_text(good)
            self.assertEqual(p.node_program(root), {"js": {"bytes": 14, "sha256": p.digest(b"actual program")}, "wrangler": {"bytes": 2, "sha256": p.digest(b"{}")}})
            for altered in (good.replace("b'actual program'", "bytes([1])"), good + "WORKER_JS = b'replaced'\n", good.replace("WORKER_JS =", "OTHER ="), good.replace("b'actual program'", "'not bytes'"), good.replace("WORKER_JS =", "WORKER_JS = alias =")):
                source.write_text(altered)
                with self.subTest(source=altered), self.assertRaises(ValueError):
                    p.node_program(root)

    def test_process_inspector_extraction_and_header_identity_are_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "run-node.py"
            good = "PROCESS_INSPECTOR = 'fixed native source'\nraise RuntimeError('must never execute')\n"
            source.write_text(good)
            image = {"bytes": 19, "sha256": p.digest(b"fixed native source")}
            self.assertEqual(p.node_process_inspector(root), image)
            for altered in (good.replace("'fixed native source'", "str(1)"),
                            good + "PROCESS_INSPECTOR = 'replaced'\n",
                            good.replace("PROCESS_INSPECTOR =", "OTHER ="),
                            good.replace("'fixed native source'", "b'not string'"),
                            good.replace("PROCESS_INSPECTOR =", "PROCESS_INSPECTOR = alias ="),
                            good.replace("'fixed native source'", "''"),
                            "PROCESS_INSPECTOR = " + repr("x" * 65537)):
                source.write_text(altered)
                with self.subTest(source=altered[:100]), self.assertRaises(ValueError):
                    p.node_process_inspector(root)
            source.write_text(good)
            header = root / "header.h"
            header.write_bytes(b"reviewed ABI")
            expected = {"path": str(header), "bytes": 12, "sha256": p.digest(b"reviewed ABI")}
            selected = {"launch": {"process_inspection": {"source": image, "headers": [expected],
                        "library": "/usr/lib/libproc.dylib", "library_policy": "operating-system-shared-cache"}}}
            self.assertEqual(p.node_process_inputs(root, selected)["headers"], [expected])
            header.write_bytes(b"modified ABI")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                p.node_process_inputs(root, selected)
            header.unlink()
            with self.assertRaises(FileNotFoundError):
                p.node_process_inputs(root, selected)
            target = root / "target.h"
            target.write_bytes(b"reviewed ABI")
            header.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "nonsymlink"):
                p.node_process_inputs(root, selected)
            source.write_text(good.replace("fixed native source", "other native source"))
            with self.assertRaisesRegex(ValueError, "literal differs"):
                p.node_process_inputs(root, selected)

    def tree(self, root):
        (root / "dir").mkdir()
        (root / "dir/module.py").write_bytes(b"source")
        (root / "dir/module.pyc").write_bytes(b"cached bytecode")
        (root / "alias").symlink_to("dir/module.py")
        (root / "outside-link").symlink_to("missing")
        return p.fixture_tree(root, p.FIXTURE_LIMITS)

    def test_tree_digest_matches_exact_canonical_rows_and_does_not_follow_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            summary, encoded, native, bytecode = self.tree(root)
            rows = json.loads(encoded)
            self.assertEqual(encoded, json.dumps(rows, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode())
            self.assertEqual(summary, {"entries": 5, "files": 2, "bytes": 21, "sha256": p.digest(encoded)})
            self.assertEqual([row["path"] for row in rows], sorted(row["path"] for row in rows))
            self.assertEqual(next(row for row in rows if row["path"] == "alias"), {"path": "alias", "kind": "symlink", "target": "dir/module.py", "resolved": str(root / "dir/module.py"), "exists": True})
            self.assertFalse(next(row for row in rows if row["path"] == "outside-link")["exists"])
            self.assertEqual(native, [])
            self.assertEqual(bytecode, 1)

    def test_tree_add_delete_bytecode_bootstrap_and_mode_changes_are_visible(self):
        for mutation in ("add", "delete", "bytecode", "bootstrap", "mode"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                original, *_ = self.tree(root)
                if mutation == "add":
                    (root / "new.py").write_bytes(b"added")
                elif mutation == "delete":
                    (root / "dir/module.py").unlink()
                elif mutation == "bytecode":
                    (root / "dir/module.pyc").write_bytes(b"changed bytecode")
                elif mutation == "bootstrap":
                    (root / "sitecustomize.py").write_bytes(b"unexpected bootstrap")
                else:
                    (root / "dir/module.py").chmod(0o700)
                current, *_ = p.fixture_tree(root, p.FIXTURE_LIMITS)
                self.assertNotEqual(current["sha256"], original["sha256"])

    def test_tree_caps_special_files_and_root_alias_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            root = parent / "input"
            root.mkdir()
            self.tree(root)
            for field, value in (("entries", 4), ("bytes", 19), ("canonical_json_bytes", 1)):
                with self.subTest(field=field), self.assertRaises(ValueError):
                    p.fixture_tree(root, {**p.FIXTURE_LIMITS, field: value})
            alias = parent / "root-alias"
            alias.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "root changed or is aliased"):
                p.fixture_tree(alias, p.FIXTURE_LIMITS)
            os.mkfifo(root / "special")
            with self.assertRaisesRegex(ValueError, "special file"):
                p.fixture_tree(root, p.FIXTURE_LIMITS)

    def test_alias_parent_retarget_cycles_and_hop_caps_are_observed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "real").mkdir()
            (root / "real/file").write_bytes(b"native")
            (root / "opt").symlink_to("real", target_is_directory=True)
            path = root / "opt/file"
            self.assertEqual(p.resolve_fixture_path(path), (str(root / "real/file"), [{"path": str(root / "opt"), "target": "real"}], True))
            (root / "other").mkdir()
            (root / "other/file").write_bytes(b"native")
            (root / "opt").unlink()
            (root / "opt").symlink_to("other", target_is_directory=True)
            self.assertEqual(p.resolve_fixture_path(path)[0], str(root / "other/file"))
            (root / "cycle-a").symlink_to("cycle-b")
            (root / "cycle-b").symlink_to("cycle-a")
            for name, hops in (("cycle-a", 32), ("opt/file", 0)):
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, "cycle or hop"):
                    p.resolve_fixture_path(root / name, hops)

    def test_stable_file_replacement_and_roster_change_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            item = root / "file"
            item.write_bytes(b"original")
            original_open = os.open
            def replace_before_open(path, flags, *args, **kwargs):
                replacement = root / "replacement"
                replacement.write_bytes(b"replaced")
                os.replace(replacement, item)
                return original_open(path, flags, *args, **kwargs)
            with patch.object(p.os, "open", side_effect=replace_before_open), self.assertRaisesRegex(ValueError, "replaced before"):
                p.stable_fixture_file(item)
            original_hash = p.stable_fixture_file
            def add_during_scan(path, *args, **kwargs):
                result = original_hash(path, *args, **kwargs)
                (root / "new").write_bytes(b"added")
                return result
            with patch.object(p, "stable_fixture_file", side_effect=add_during_scan), self.assertRaisesRegex(ValueError, "roster changed"):
                p.fixture_tree(root, p.FIXTURE_LIMITS)

    def test_file_size_is_admitted_before_open_and_errors_are_not_empty_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "input"
            path.write_bytes(b"oversized")
            for maximum, expected in ((2, None), (1024, {"bytes": 1, "sha256": "a" * 64})):
                with self.subTest(maximum=maximum), patch.object(p.os, "open") as opening, self.assertRaisesRegex(ValueError, "before read"):
                    p.stable_fixture_file(path, maximum, expected)
                opening.assert_not_called()
            with patch.object(p.os, "scandir", side_effect=PermissionError("denied")), self.assertRaises(PermissionError):
                p.fixture_tree(root, p.FIXTURE_LIMITS)

    def test_complete_fixture_verification_detects_native_alias_startup_and_tree_changes(self):
        for change in ("native", "alias", "startup", "tree", "platform", "roster"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                tree = root / "tree"
                tree.mkdir()
                (tree / "source.py").write_bytes(b"source")
                summary, _, _, _ = p.fixture_tree(tree, p.FIXTURE_LIMITS)
                native = root / "native"
                native.write_bytes(bytes.fromhex("cffaedfe") + b"pinned native input")
                image, _ = p.stable_fixture_file(native)
                alias = root / "alias"
                alias.symlink_to("native")
                platform = root / "platform"
                platform.write_bytes(b"pinned platform")
                platform_image, _ = p.stable_fixture_file(platform)
                absent = root / "python._pth"
                profile = {"fixture": {
                    "inventory_limits": dict(p.FIXTURE_LIMITS), "trees": [{**summary, "path": str(tree), "label": "stdlib"}], "bytecode": {"stdlib": 0},
                    "external_native": [{**image, "path": str(native)}], "platform": {**platform_image, "path": str(platform)}, "native_roster": [str(native)],
                    "aliases": [{"path": str(alias), "resolved": str(native), "symlink_chain": [{"path": str(alias), "target": "native"}], "target_sha256": image["sha256"]}],
                    "non_os_load_edges": [{"from": str(native), "name": str(alias), "resolved": str(native)}], "required_absence": [str(absent)], "startup": [],
                }}
                before = p.verify_node_fixture(profile)
                self.assertEqual(before["native_files"], 1)
                if change == "native":
                    native.write_bytes(bytes.fromhex("cffaedfe") + b"changed native data")
                elif change == "alias":
                    other = root / "other"
                    other.write_bytes(native.read_bytes())
                    alias.unlink()
                    alias.symlink_to("other")
                elif change == "startup":
                    absent.symlink_to("missing")
                elif change == "tree":
                    (tree / "cached.pyc").write_bytes(b"new bytecode")
                elif change == "platform":
                    platform.write_bytes(b"changed platform")
                else:
                    profile["fixture"]["native_roster"] = []
                with self.assertRaises(ValueError):
                    p.verify_node_fixture(profile)

    def test_absence_rejects_dangling_alias_and_native_config_rejects_provider(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "python._pth"
            p.fixture_absent(path)
            path.symlink_to("missing")
            with self.assertRaisesRegex(ValueError, "unexpected fixture"):
                p.fixture_absent(path)
            empty = root / "empty-native-config"
            empty.mkdir()
            p.verify_empty_native_directory(root)
            (empty / "provider.so").write_bytes(b"unexpected provider")
            with self.assertRaisesRegex(ValueError, "native configuration"):
                p.verify_empty_native_directory(root)

    def test_environment_is_whitelisted_and_launch_replaces_import_path_first(self):
        profile = self.profile()
        ports = dict(zip(p.NODE_ROLES, range(31001, 31006)))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            with patch.dict(os.environ, {"DYLD_INSERT_LIBRARIES": "evil", "PYTHONPATH": "evil", "HTTPS_PROXY": "evil", "AWS_PROFILE": "evil", "OPENSSL_CONF": "evil"}):
                environment = p.node_environment(ROOT, profile, root, ports)
            self.assertEqual(set(environment), set(profile["launch"]["environment"]) | {"TMPDIR", "OPENSSL_CONF_INCLUDE", "OPENSSL_MODULES", "SSL_CERT_DIR", "SSL_CERT_FILE"})
            self.assertNotIn("HOME", environment)
            self.assertEqual(environment["OPENSSL_CONF"], "/dev/null")
            self.assertEqual(environment["SSL_CERT_FILE"], profile["launch"]["certifi"])
            binary = {"path": "/actual/celld", "sha256": "a" * 64}
            deadline = p.time.monotonic_ns() + 600_000_000_000
            with patch.object(p, "node_network_command", side_effect=lambda args, _ports: args):
                argv = p.node_controller_command(ROOT, profile, binary, root, ports, deadline)
            self.assertEqual(argv[:5], [profile["launch"]["python"], "-I", "-S", "-B", "-c"])
            self.assertTrue(argv[5].startswith("import sys; sys.path[:] = "))
            self.assertLess(argv[5].index("sys.path[:]"), argv[5].index("import runpy"))
            self.assertEqual(argv[argv.index("--deadline-ns") + 1], str(deadline))
            self.assertEqual(argv[argv.index("--node-lock-sha256") + 1], p.NODE_PROFILE_SHA256)
            with self.assertRaisesRegex(ValueError, "deadline expired"):
                p.node_controller_command(ROOT, profile, binary, root, ports, 1)

    def test_node_build_commands_select_exact_primary_bin_and_keep_old_modes(self):
        pins = self.profile()
        pins["profile"] = p.A2
        root, toolchain = Path("/fixture"), Path("/toolchain")
        with patch.object(p, "network_command", side_effect=lambda args: list(map(str, args))):
            commands = p.build_commands(root, toolchain, pins, "node-baseline")
        self.assertEqual(set(commands), {"build", "clippy"})
        for command in commands.values():
            self.assertEqual([command[index + 1] for index, value in enumerate(command) if value == "--package"], ["foks-celld-guarded", "celld@0.6.2"])
            self.assertEqual(command[command.index("--bin") + 1], "celld")
            self.assertEqual(command[command.index("--target") + 1], p.TARGET)
            self.assertNotIn("--features", command)
            self.assertNotIn("--all-targets", command)
            self.assertIn("--locked", command)
            self.assertIn("--offline", command)
        self.assertEqual(commands["build"][-1], "--message-format=json-render-diagnostics")
        self.assertEqual(commands["clippy"][-3:], ["--", "-D", "warnings"])
        with self.assertRaisesRegex(ValueError, "mode"):
            p.selected_mode({"profile": p.A1}, "node-baseline")

    def test_compiler_artifact_requires_actual_unique_bin_source_features_and_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            binary = root / "runtime/target" / p.TARGET / "debug/celld"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"actual executable")
            binary.chmod(0o700)
            target = {"package_id": "celld-id", "name": "celld", "kind": ["bin"], "crate_types": ["bin"], "src_path": str(root / "runtime/source/crates/celld/main.rs"), "features": []}
            artifact = {"reason": "compiler-artifact", "package_id": "celld-id", "target": {key: target[key] for key in ("name", "kind", "crate_types", "src_path")}, "features": [], "executable": str(binary), "profile": {"test": False}}
            finished = {"reason": "build-finished", "success": True}
            output = root / "build.stdout"
            def write(messages):
                output.write_text("".join(json.dumps(message) + "\n" for message in messages))
            write([artifact, finished])
            self.assertEqual(p.verify_node_artifact(output, {"node_target": target}, root)["sha256"], p.digest(b"actual executable"))
            foundation = {**target, "features": p.FOUNDATION_MODES["node-foundation"]["celld_features"]}
            write([{**artifact, "features": foundation["features"]}, finished])
            self.assertEqual(p.verify_node_artifact(output, {"node_target": foundation}, root)["artifact"], foundation)
            with self.assertRaisesRegex(ValueError, "unexpected executable"):
                p.verify_node_artifact(output, {"node_target": target}, root)
            write([artifact, finished])
            with self.assertRaisesRegex(ValueError, "unexpected executable"):
                p.verify_node_artifact(output, {"node_target": foundation}, root)
            changes = [lambda item: item.update(package_id="harness-id"), lambda item: item.update(features=["foks-guarded-qualification"]), lambda item: item["target"].update(src_path="/unreviewed/main.rs"), lambda item: item["target"].update(kind=["lib"]), lambda item: item.update(executable="/different/celld"), lambda item: item["profile"].update(test=True)]
            for change in changes:
                altered = copy.deepcopy(artifact)
                change(altered)
                write([altered, finished])
                with self.subTest(change=change), self.assertRaises(ValueError):
                    p.verify_node_artifact(output, {"node_target": target}, root)
            for messages in ([artifact], [artifact, artifact, finished], [artifact, {"reason": "build-finished", "success": False}], [finished], [artifact, finished, finished]):
                write(messages)
                with self.subTest(messages=messages), self.assertRaises(ValueError):
                    p.verify_node_artifact(output, {"node_target": target}, root)
            write([artifact, {"reason": "build-finished", "success": 1}])
            with self.assertRaisesRegex(ValueError, "boolean"):
                p.verify_node_artifact(output, {"node_target": target}, root)
            output.write_text(json.dumps(artifact) + '\n{"reason":"build-finished","success":false,"success":true}\n')
            with self.assertRaisesRegex(ValueError, "unclassified compiler message"):
                p.verify_node_artifact(output, {"node_target": target}, root)
            write([artifact, finished])
            binary.unlink()
            binary.symlink_to(output)
            with self.assertRaisesRegex(ValueError, "regular nonsymlink"):
                p.verify_node_artifact(output, {"node_target": target}, root)

    def test_node_monitor_keeps_original_resource_floor_and_adds_evidence_cap(self):
        profile = self.profile()
        with patch.object(p, "resource_refusal", return_value=("free-space admission refusal", {"free_bytes": 1})), patch.object(p, "directory_bytes", return_value=1):
            self.assertEqual(p.node_monitor(ROOT, Path("/evidence"), profile)[0], "free-space admission refusal")
        with patch.object(p, "resource_refusal", return_value=(None, {})), patch.object(p, "directory_bytes", return_value=profile["limits"]["evidence_bytes"] + 1):
            self.assertEqual(p.node_monitor(ROOT, Path("/evidence"), profile)[0], "node evidence allowance exceeded")

    def test_node_report_requires_bound_success_and_separate_physical_cleanup(self):
        profile = self.profile()
        binary = {"path": "/actual/celld", "sha256": "a" * 64}
        ports = dict(zip(p.NODE_ROLES, range(31001, 31006)))
        children = [{"label": label, "pid": index + 100, "argv": ["/actual/command"], "node": None, "directory": None, "ports": {}, "generation": "actual-generation" if label in ("a", "b", "b-restart", "c") else None, "started_ns": 10, "exit_ns": 20, "returncode": 2 if label == "invalid-option" else 0, "attributed_pids": [index + 100], "retired": True, "log": "/evidence/actual.log"} for index, label in enumerate(("version", "help", "invalid-option", "moto", "deploy", "a", "b", "b-restart", "c"))]
        for child in children[-4:]:
            suffix = "b" if child["label"] in ("b", "b-restart") else "ac"
            child.update(argv=[binary["path"]], node="actual-node", directory="/evidence/node", ports={"public": ports["public_" + suffix], "internal": ports["internal_" + suffix]})
        report = {"format": 1, "profile": p.NODE, "claim": p.NODE_CLAIM, "status": "passed", "node_lock_sha256": p.NODE_PROFILE_SHA256, "binary": binary, "program": profile["program"], "ports": ports, "run_id": "b" * 32, "startup": {"actual": "observations"}, "sequence": {"completed": True, "steps": ["actual canonical witnesses"], "incarnations": children[-4:], "error": None}, "cleanup": {"completed": True, "children": children, "storage_tasks_joined": True, "active_responses": 0, "outer_group_disappearance_observed": False, "error": None}, "observation_limits": {"final_transport_grace_observed": False, "internal_task_drain_observed": False}}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text(json.dumps(report))
            self.assertEqual(p.verify_node_report(path, profile, binary, ports, p.NODE_PROFILE_SHA256), report)
            changes = [
                lambda value: value.update(format=True), lambda value: value.update(status="incomplete"), lambda value: value.update(node_lock_sha256="0" * 64),
                lambda value: value["binary"].update(sha256="0" * 64), lambda value: value["program"]["js"].update(bytes=1),
                lambda value: value["sequence"].update(completed=1), lambda value: value["sequence"]["incarnations"].pop(),
                lambda value: value["cleanup"].update(completed=1), lambda value: value["cleanup"].update(storage_tasks_joined=1),
                lambda value: value["cleanup"]["children"][0].update(retired=1), lambda value: value["cleanup"]["children"][0].update(returncode=True),
                lambda value: value["cleanup"].update(active_responses=False), lambda value: value["cleanup"].update(outer_group_disappearance_observed=True),
                lambda value: value["observation_limits"].update(internal_task_drain_observed=0), lambda value: value.update(extra="ignored"),
            ]
            for change in changes:
                altered = copy.deepcopy(report)
                change(altered)
                path.write_text(json.dumps(altered))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    p.verify_node_report(path, profile, binary, ports, p.NODE_PROFILE_SHA256)

    def test_narrow_command_log_allowance_keeps_group_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            script = "import os,signal; os.write(1,b'x'*4096); signal.pause()"
            with p.BuildSlot(root) as slot:
                with self.assertRaisesRegex(ValueError, "log allowance"):
                    p.run_owned([sys.executable, "-I", "-S", "-B", "-c", script], root, {"PATH": "/usr/bin:/bin"}, root / "evidence", "small-log", slot, 10, maximum_log_bytes=1024)
                record = p.read_json(root / "evidence/small-log.command.json")
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))


@unittest.skipUnless(sys.platform == "darwin" and Path("/usr/bin/sandbox-exec").is_file(), "actual reviewed Darwin network policy required")
class NodeNetworkPolicyTests(unittest.TestCase):
    @contextmanager
    def failure_diagnostics(self, root, name):
        # run_owned has already retained the command result and established or
        # refused group cleanup. Carry bounded evidence into the test exception
        # before TemporaryDirectory removes these local helper fixtures.
        try:
            yield
        except Exception as error:
            diagnostics = []
            for suffix in ("stdout", "stderr", "command.json"):
                path = root / "evidence" / (name + "." + suffix)
                try:
                    with path.open("rb") as stream:
                        raw = stream.read(65537)
                    text = raw[:65536].decode("utf-8", errors="replace")
                    # Invalid UTF-8 replacement may expand; bound the diagnostic
                    # itself in bytes as well as the file read.
                    encoded = text.encode("utf-8")
                    truncated = len(raw) > 65536 or len(encoded) > 65536
                    text = encoded[:65536].decode("utf-8", errors="ignore")
                    diagnostics.append(f"{suffix} (truncated={truncated}):\n{text}")
                except OSError as read_error:
                    diagnostics.append(f"{suffix}: unavailable ({type(read_error).__name__})")
            raise AssertionError(
                f"{name}: {type(error).__name__}: {str(error)[:1024]}\n" + "\n".join(diagnostics)
            ) from error

    def test_failure_diagnostics_survive_fixture_cleanup_and_are_bounded(self):
        with self.assertRaises(AssertionError) as failure:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                (root / "evidence").mkdir()
                for suffix in ("stdout", "stderr", "command.json"):
                    (root / "evidence" / ("probe." + suffix)).write_bytes(b"captured-" + b"x" * 65536 + b"unread-tail")
                with self.failure_diagnostics(root, "probe"):
                    raise ValueError("command failed")
        message = str(failure.exception)
        self.assertIn("probe: ValueError: command failed", message)
        self.assertEqual(message.count("(truncated=True)"), 3)
        self.assertEqual(message.count("captured-"), 3)
        self.assertNotIn("unread-tail", message)
        self.assertLess(len(message.encode()), 3 * 65536 + 1024)
        self.assertFalse(root.exists())

    def test_real_isolated_controller_startup_preserves_exact_environment(self):
        profile = p.load_node_profile(ROOT)
        ports = dict(zip(p.NODE_ROLES, range(31001, 31006)))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            environment = p.node_environment(ROOT, profile, root, ports)
            # Execute definitions only: the controller's main/Run and fixture
            # imports are not entered. The real isolated interpreter must keep
            # the exact explicit whitelist through its platform startup.
            script = r'''import runpy
namespace=runpy.run_path(sys.argv[1],run_name='foks_startup_environment_probe')
Path=namespace['Path']; os=namespace['os']; json=namespace['json']
controller=Path(sys.argv[1]).resolve(strict=True); directory=Path(sys.argv[3])
raw=namespace['regular_bytes'](sys.argv[2],131072)
assert namespace['identity'](raw)['sha256']==namespace['PROFILE_SHA256']
profile=namespace['strict_json'](raw,131072)
check=namespace['check_environment']; observe=namespace['startup_observation']
expected=check(profile,directory); original=dict(os.environ)
assert original==expected
startup=observe(profile,controller)
assert sys.flags.utf8_mode==1
assert original['LC_ALL']=='C'
assert original['__CF_USER_TEXT_ENCODING']=='0x1F5:0x0:0x0'
rejected=[]
def refuse(label,altered):
 try:
  os.environ.clear(); os.environ.update(altered)
  try:
   check(profile,directory)
  except namespace['Refused'] as error:
   assert str(error)=='controller environment differs from closed whitelist'
   rejected.append(label)
  else:
   raise AssertionError('environment mutation accepted: '+label)
 finally:
  os.environ.clear(); os.environ.update(original)
 assert check(profile,directory)==expected
for key in sorted(expected):
 altered=dict(expected); del altered[key]; refuse('missing:'+key,altered)
 altered=dict(expected); altered[key]+='-changed'; refuse('changed:'+key,altered)
extra='FOKS_UNREVIEWED_STARTUP_KEY'; assert extra not in expected
refuse('extra:'+extra,dict(expected,**{extra:'unreviewed'}))
assert check(profile,directory)==expected
assert observe(profile,controller)==startup
assert not any(name.split('.')[0] in ('moto','boto3','botocore') for name in sys.modules)
result={'environment_keys':sorted(expected),'rejected':rejected,'startup':startup,'utf8_mode':sys.flags.utf8_mode}
encoded=json.dumps(result,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n'
assert len(encoded.encode('utf-8'))<=65536
sys.stdout.write(encoded)
'''
            bootstrap = "import sys; sys.path[:] = " + repr(profile["launch"]["sys_path"]) + ";\n" + script
            argv = [profile["launch"]["python"], *profile["launch"]["flags"], "-c", bootstrap,
                    str(ROOT / "run-node.py"), str(ROOT / "node.lock.json"), str(root)]
            with p.BuildSlot(root) as slot, self.failure_diagnostics(root, "startup-environment"):
                record = p.run_owned(p.network_command(argv), root, environment, root / "evidence",
                                     "startup-environment", slot, 15, maximum_log_bytes=65536)
                observed = p.read_json(root / "evidence/startup-environment.stdout", 65536)
                expected_rejections = [f"{kind}:{key}" for key in sorted(environment) for kind in ("missing", "changed")]
                expected_rejections.append("extra:FOKS_UNREVIEWED_STARTUP_KEY")
                self.assertEqual(observed["environment_keys"], sorted(environment))
                self.assertEqual(observed["rejected"], expected_rejections)
                self.assertEqual(observed["utf8_mode"], 1)
                self.assertEqual(observed["startup"]["flags"], {key: 1 for key in ("isolated", "ignore_environment", "no_site", "dont_write_bytecode")})
                self.assertEqual(observed["startup"]["sys_path"], profile["launch"]["sys_path"])
                self.assertEqual(observed["startup"]["executable"], str(Path(profile["launch"]["python"]).resolve(strict=True)))
                self.assertEqual(observed["startup"]["python_version"], profile["fixture"]["versions"]["python"])
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))

    def test_real_sandbox_native_inspector_observes_live_zombie_and_reaped_child(self):
        profile = p.load_node_profile(ROOT)
        before = p.node_inputs(ROOT, profile)
        ports = p.choose_node_ports()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            environment = p.node_environment(ROOT, profile, root, ports)
            script = r'''import runpy
namespace=runpy.run_path(sys.argv[1],run_name='foks_process_inspector_probe')
Path=namespace['Path']; os=namespace['os']; json=namespace['json']
time=namespace['time']; signal=namespace['signal']
directory=Path(sys.argv[3]); controller=Path(sys.argv[1]).resolve(strict=True)
raw=namespace['regular_bytes'](sys.argv[2],131072)
assert namespace['identity'](raw)['sha256']==namespace['PROFILE_SHA256']
profile=namespace['strict_json'](raw,131072)
environment=namespace['check_environment'](profile,directory)
namespace['startup_observation'](profile,controller)
clock=namespace['Clock'](time.monotonic_ns()+10_000_000_000)
children=namespace['Children'](clock,directory,environment,profile)
assert os.getpid()==os.getpgrp()
ready=directory/'owned.ready'
child_source=('import sys;sys.path[:]= '+repr(profile['launch']['sys_path'][:2])+
 ';import pathlib,signal;signal.signal(signal.SIGTERM,lambda *_:sys.exit(0));'
 'pathlib.Path(sys.argv[1]).write_bytes(b"ready");signal.pause()')
argv=[profile['launch']['python'],*profile['launch']['flags'],'-c',child_source,str(ready)]
live=zombie=reaped=None
try:
 child=children.spawn('owned',argv)
 while not ready.exists():
  children.tick(clock.deadline); time.sleep(min(.01,clock.remaining(clock.deadline)))
 assert ready.read_bytes()==b'ready'
 children.scan(clock.deadline,force=True)
 live=children.table[child.process.pid]
 assert live[:2]==(os.getpid(),os.getpgrp()) and live[2] in (1,2,3,4)
 child.expected_exit=True
 child.process.send_signal(signal.SIGTERM)
 # scan does not poll/wait the child: preserve the unreaped-zombie witness.
 while True:
  clock.check(); children.scan(clock.deadline,force=True)
  row=children.table[child.process.pid]
  assert row[:2]==(os.getpid(),os.getpgrp())
  if row[2]==5:
   zombie=row; break
 assert child.process.returncode is None
 assert children.wait(child,clock.deadline)==0
 assert child.retired and child.process.pid not in children.table
 reaped=True
finally:
 cleanup=children.cleanup()
assert cleanup['completed'] and cleanup['error'] is None
assert live is not None and zombie is not None and reaped is True
assert all(row['retired'] for row in cleanup['children'])
assert not any(name.split('.')[0] in ('moto','boto3','botocore') for name in sys.modules)
result={'controller':os.getpid(),'group':os.getpgrp(),'child':child.process.pid,
        'live':live,'zombie':zombie,'reaped_absent':reaped,'cleanup':cleanup}
encoded=json.dumps(result,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n'
assert len(encoded.encode('utf-8'))<=65536
sys.stdout.write(encoded)
'''
            bootstrap = "import sys; sys.path[:] = " + repr(profile["launch"]["sys_path"]) + ";\n" + script
            argv = [profile["launch"]["python"], *profile["launch"]["flags"], "-c", bootstrap,
                    str(ROOT / "run-node.py"), str(ROOT / "node.lock.json"), str(root)]
            with p.BuildSlot(root) as slot, self.failure_diagnostics(root, "native-process-inspection"):
                record = p.run_owned(p.node_network_command(argv, ports), root, environment,
                                     root / "evidence", "native-process-inspection", slot, 15,
                                     maximum_log_bytes=65536)
                observed = p.read_json(root / "evidence/native-process-inspection.stdout", 65536)
                self.assertEqual((observed["controller"], observed["group"]), (record["process_group"],) * 2)
                self.assertEqual(observed["live"][:2], [observed["controller"], observed["group"]])
                self.assertIn(observed["live"][2], (1, 2, 3, 4))
                self.assertEqual(observed["zombie"], [observed["controller"], observed["group"], 5])
                self.assertIs(observed["reaped_absent"], True)
                self.assertIs(observed["cleanup"]["completed"], True)
                self.assertIsNone(observed["cleanup"]["error"])
                self.assertEqual([row["pid"] for row in observed["cleanup"]["children"]], [observed["child"]])
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(record["remaining_escaped_children"])
                self.assertFalse(p.group_alive(record["process_group"]))
                self.assertEqual(p.node_inputs(ROOT, profile), before)

    def test_five_real_roles_allow_bidirectional_traffic_and_deny_unlisted_outbound(self):
        with tempfile.TemporaryDirectory() as directory, p.socket.socket() as unlisted:
            root = Path(directory).resolve()
            # This endpoint belongs to the unsandboxed test parent. It must be
            # live, held, and outside the five roles when the child probes it.
            unlisted.bind(("127.0.0.1", 0))
            unlisted.listen(1)
            unlisted.settimeout(2)
            unlisted_port = unlisted.getsockname()[1]
            ports = p.choose_node_ports()
            self.assertEqual(len(set(ports.values())), 5)
            self.assertNotIn(unlisted_port, ports.values())

            def exact(peer, count):
                result = b""
                while len(result) < count:
                    part = peer.recv(count - len(result))
                    self.assertTrue(part)
                    result += part
                return result

            with p.socket.create_connection(("127.0.0.1", unlisted_port), timeout=2) as client:
                with unlisted.accept()[0] as server:
                    server.settimeout(2)
                    client.sendall(b"live")
                    self.assertEqual(exact(server, 4), b"live")
                    server.sendall(b"held")
                    self.assertEqual(exact(client, 4), b"held")
            script = '''import errno,json,socket,sys
ports=json.loads(sys.argv[1]); unlisted_port=int(sys.argv[2]); listeners=[]
assert unlisted_port not in ports.values()
def exact(peer,n):
 result=b''
 while len(result)<n:
  part=peer.recv(n-len(result)); assert part; result+=part
 return result
for port in ports.values():
 s=socket.socket(); s.bind(('127.0.0.1',port)); s.listen(); s.settimeout(2); listeners.append(s)
for port,listener in zip(ports.values(),listeners):
 with socket.create_connection(('127.0.0.1',port),timeout=2) as client:
  with listener.accept()[0] as server:
   client.sendall(b'allowed'); assert exact(server,7)==b'allowed'
   server.sendall(b'reply'); assert exact(client,5)==b'reply'
with socket.socket() as denied:
 denied.settimeout(.3)
 try:
  denied.connect(('127.0.0.1',unlisted_port))
 except OSError as error:
  assert error.errno in (errno.EPERM,errno.EACCES), ('unexpected outbound error',error.errno)
  denied_errno=error.errno
 else:
  raise AssertionError('unlisted outbound traffic allowed')
for listener in listeners: listener.close()
print('five roles bidirectional; live unlisted outbound denied',unlisted_port,denied_errno)
'''
            with p.BuildSlot(root) as slot, self.failure_diagnostics(root, "policy-roles"):
                record = p.run_owned(p.node_network_command([sys.executable, "-I", "-S", "-B", "-c", script, json.dumps(ports), str(unlisted_port)], ports), root, {"PATH": "/usr/bin:/bin"}, root / "evidence", "policy-roles", slot, 15)
                self.assertTrue(record["cleanup_established"])
                self.assertFalse(p.group_alive(record["process_group"]))

    def test_unlisted_bind_does_not_admit_inbound_traffic(self):
        ports = p.choose_node_ports()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            ready = root / "ready.json"
            script = '''import errno,json,os,pathlib,socket,sys
ports=json.loads(sys.argv[2]); held=[]
for port in ports.values():
 listener=socket.socket(); listener.bind(('127.0.0.1',port)); held.append(listener)
allowed=held[0]; allowed.listen(); allowed.settimeout(3)
with socket.socket() as listener:
 listener.bind(('127.0.0.1',0)); listener.settimeout(2)
 denied_port=listener.getsockname()[1]; assert denied_port not in ports.values()
 try:
  listener.listen()
 except OSError as error:
  assert error.errno in (errno.EPERM,errno.EACCES), ('unexpected listen error',error.errno)
  denied_errno=error.errno
 else:
  raise AssertionError('unlisted listen succeeded; no qualified accept-denial fallback')
 ready=pathlib.Path(sys.argv[1]); temporary=ready.with_suffix('.new')
 temporary.write_text(json.dumps({'allowed':ports['s3'],'denied':denied_port,'operation':'listen','errno':denied_errno})); os.replace(temporary,ready)
 with allowed.accept()[0] as peer:
  peer.settimeout(2); received=b''
  while len(received)<4:
   part=peer.recv(4-len(received)); assert part; received+=part
  assert received==b'ping'; peer.sendall(b'pong')
for listener in held: listener.close()
print('external parent allowed-port control passed; unlisted listen denied',denied_port,denied_errno)
'''
            controls = []
            def monitor():
                if ready.exists() and not controls:
                    observed = json.loads(ready.read_text())
                    self.assertEqual(set(observed), {"allowed", "denied", "operation", "errno"})
                    self.assertEqual(observed["allowed"], ports["s3"])
                    self.assertIs(type(observed["denied"]), int)
                    self.assertNotIn(observed["denied"], ports.values())
                    self.assertEqual(observed["operation"], "listen")
                    self.assertIs(type(observed["errno"]), int)
                    self.assertIn(observed["errno"], (errno.EPERM, errno.EACCES))
                    with p.socket.create_connection(("127.0.0.1", observed["allowed"]), timeout=1) as peer:
                        peer.settimeout(1)
                        peer.sendall(b"ping")
                        received = b""
                        while len(received) < 4:
                            part = peer.recv(4 - len(received))
                            self.assertTrue(part)
                            received += part
                        self.assertEqual(received, b"pong")
                    controls.append(observed)
                return None, {}
            with p.BuildSlot(root) as slot, self.failure_diagnostics(root, "policy-inbound"):
                record = p.run_owned(p.node_network_command([sys.executable, "-I", "-S", "-B", "-c", script, str(ready), json.dumps(ports)], ports), root, {"PATH": "/usr/bin:/bin"}, root / "evidence", "policy-inbound", slot, 15, monitor)
                self.assertTrue(record["cleanup_established"])
                self.assertEqual(len(controls), 1)
                self.assertFalse(p.group_alive(record["process_group"]))

    def test_port_policy_rejects_ambiguous_or_non_integer_roles(self):
        ports = dict(zip(p.NODE_ROLES, range(31001, 31006)))
        for altered in ({**ports, "s3": True}, {**ports, "s3": ports["public_ac"]}, {**ports, "external": 80}, {key: value for key, value in ports.items() if key != "s3"}, {**ports, "s3": "1234"}):
            with self.subTest(ports=altered), self.assertRaisesRegex(ValueError, "port allocation"):
                p.node_network_command(["/bin/true"], altered)
        self.assertEqual(p.node_network_command(["/bin/true"], ports)[2], p.NODE_POLICY.format(**ports))
        # Seatbelt accepts its special localhost spelling here, not a numeric
        # host. Actual listeners/clients still use the pinned IPv4 address.
        profile = p.read_json(ROOT / "node.lock.json")
        self.assertEqual(profile["network"]["address"], "127.0.0.1")
        self.assertEqual(profile["network"]["template"], p.NODE_POLICY)
        self.assertEqual(p.NODE_POLICY.count('tcp "localhost:'), 11)
        self.assertNotIn('tcp "127.0.0.1:', p.NODE_POLICY)
        self.assertNotIn('tcp "*:', p.NODE_POLICY)
        self.assertEqual(p.network_command(["/bin/true"])[2], "(version 1) (allow default) (deny network*)")


if __name__ == "__main__":
    unittest.main()

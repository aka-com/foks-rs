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


def a2_fixture(root):
    contents = {
        "Cargo.toml": b'[workspace]\nmembers = ["crates/celld"]\n',
        "Cargo.lock": b"version = 3\n", "unchanged.txt": b"unchanged upstream\n",
    }
    contents.update({name: ("before " + name + "\nshared\nlast\n").encode() for name in p.PATCH_PATHS if name not in p.PATCH_ADDITIONS})
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

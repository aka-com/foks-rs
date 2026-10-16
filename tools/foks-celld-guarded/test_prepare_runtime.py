"""Preparation/resource regressions. No Cargo, V8 build or network invocation."""
import copy
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


if __name__ == "__main__":
    unittest.main()

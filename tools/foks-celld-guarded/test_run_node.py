#!/usr/bin/env python3
"""Controller regressions. These small fixtures do not qualify celld or Moto."""

import copy
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

# The suite is bound to the macOS lab host: node.lock.json names interpreter
# and library paths under the lab root and Homebrew, and the controllers
# refuse anything but Darwin on arm64. Elsewhere the module reports one
# skipped test. load_tests applies only when this module is loaded as a test
# module, so test_check_graph can still import its fixtures.
LAB_ROOT = Path("/private/tmp/foks-celld-lab")
LAB_ONLY = f"lab-only suite: requires macOS on arm64 and the lab root {LAB_ROOT}"


def load_tests(loader, tests, pattern):
    import platform

    if sys.platform == "darwin" and platform.machine() == "arm64" and LAB_ROOT.is_dir():
        return tests

    @unittest.skip(LAB_ONLY)
    class LabHostRequired(unittest.TestCase):
        def test_lab_host(self):
            pass

    return loader.loadTestsFromTestCase(LabHostRequired)


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("a3a_run_node", ROOT / "run-node.py")
node = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(node)


def profile():
    raw = (ROOT / "node.lock.json").read_bytes()
    if node.identity(raw)["sha256"] != node.PROFILE_SHA256:
        raise AssertionError("test process profile changed")
    return node.strict_json(raw, 131072)


def encode(value):
    return json.dumps(value, separators=(",", ":")).encode()


def lease(name="node-b", expiry=2000, generation="generation-b"):
    return {"node": name, "addr": "127.0.0.1:32002", "expires_ms": expiry,
            "probe_public_key": generation, "ownership_index_generation": generation,
            "peer_protocol": 5, "paced_handoff": True,
            "load": {"sampled_ms": 1, "pressured": False, "memory_headroom": True, "draining": False}}


def census(kind="dormant"):
    value = {key: 0 for key in node.ZERO_COUNTS}
    value.update(owned_cells=0 if kind == "empty" else 1, residents=[],
                 phases={} if kind == "empty" else {kind: 1})
    if kind == "resident":
        value["residents"] = ["BaselineCell:abc"]
        value["occupied"] = 1
    return value


def handoff_log():
    return (b'INFO event="drain_token_acquired" waited_ms=0 expires_ms=999\n'
            b'INFO event="cell_handoff_accepted" cell=BaselineCell:abc released_epoch=5 successor=node-b successor_epoch=6 attempts=1\n'
            b'INFO event="cell_handoff_timing" cell=BaselineCell:abc quiesce_us=1 durability_us=2 stop_us=3 release_us=4 adopt_us=5 total_us=15\n'
            b'INFO event="drain_token_released"\n')


class FakeTime:
    def __init__(self):
        self.mono = 10_000_000_000
        self.wall = 1_000_000_000_000

    def clock(self):
        return node.Clock(self.mono + 100_000_000_000, lambda: self.mono, lambda: self.wall)

    def advance(self, amount):
        self.mono += amount
        self.wall += amount


class Oracles(unittest.TestCase):
    def test_profile_is_literal_and_load_has_no_fixture_import(self):
        self.assertNotIn("moto", sys.modules)
        self.assertNotIn("boto3", sys.modules)
        profile = node.strict_json((ROOT / "node.lock.json").read_bytes(), 131072)
        self.assertEqual(node.identity((ROOT / "node.lock.json").read_bytes())["sha256"], node.PROFILE_SHA256)
        self.assertEqual(profile["program"], {"js": node.identity(node.WORKER_JS), "wrangler": node.identity(node.WRANGLER_JSON)})
        self.assertIn(b"await this.state.storage.sync()", node.WORKER_JS)

    def worker_fixture(self, status, raw, label="a"):
        run = object.__new__(node.Run)
        run.clock = FakeTime().clock()
        run.run_id = "a" * 32
        observation = {"status": status, "response": {"bytes": len(raw), "sha256": node.identity(raw)["sha256"]}}
        run.response = mock.Mock(return_value=(status, raw, observation))
        run.step = mock.Mock()
        child = mock.Mock(label=label, ports={"public": 32001})
        return run, child, observation

    def test_worker_success_binds_exact_marker_count_and_request(self):
        marker = "a" * 32 + "-one"
        raw = encode({"fixture": "foks-celld-a3a-v1", "marker": marker, "rows": 1})
        for write, label in ((True, "a"), (False, "b-restart")):
            with self.subTest(write=write, label=label):
                run, child, observation = self.worker_fixture(200, raw, label)
                deadline = run.clock.phase(15)
                run.worker(child, "one", write=write)
                run.response.assert_called_once_with(label + ("-write" if write else "-read"),
                                                     32001, "POST" if write else "GET",
                                                     "/write" if write else "/read", deadline,
                                                     marker.encode() if write else b"")
                run.step.assert_called_once_with(label + ("-write-synced" if write else "-read-restored"),
                                                 response=observation, marker=marker, rows=1,
                                                 fresh_directory=label == "a")

    def test_worker_rejects_inexact_success_shape_value_and_count(self):
        good = {"fixture": "foks-celld-a3a-v1", "marker": "a" * 32 + "-one", "rows": 1}
        values = [[], None, {**good, "extra": 1}, {**good, "fixture": "other"},
                  {**good, "marker": "a" * 32 + "-two"}]
        values.extend({name: value for name, value in good.items() if name != missing} for missing in good)
        values.extend({**good, "rows": rows} for rows in (0, -1, 2, True, False, 1.0, "1", None, [], {}))
        for raw in [*(encode(value) for value in values), b"{", b"\xff"]:
            with self.subTest(raw=raw):
                run, child, _ = self.worker_fixture(200, raw)
                with self.assertRaises(node.Refused):
                    run.worker(child, "one", write=True)
                run.step.assert_not_called()

    def test_worker_http_failure_precedes_json_and_has_bounded_escaped_evidence(self):
        for status, raw in ((500, b"Worker failed: SQL error: sqlite_version"),
                            (503, b"\xff\n" + b"x" * 256 + b"unread-tail"),
                            (400, b"{"), (301, b'"unexpected redirect"')):
            with self.subTest(status=status):
                run, child, _ = self.worker_fixture(status, raw)
                with mock.patch.object(node, "strict_json", side_effect=AssertionError("failure body parsed as success")):
                    with self.assertRaises(node.Refused) as failure:
                        run.worker(child, "one", write=True)
                message = str(failure.exception)
                self.assertIn("Worker HTTP status " + str(status), message)
                self.assertIn(repr(raw[:128]), message)
                self.assertNotIn("unread-tail", message)
                self.assertNotIn("\n", message)
                self.assertLess(len(message.encode()), 1024)
                run.step.assert_not_called()

    def test_strict_json_rejects_duplicates_nonfinite_bad_utf8_and_bounds(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}', b'{"a":1e999}', b'"\xff"', b"["):
            with self.subTest(raw=raw), self.assertRaises(node.Refused):
                node.strict_json(raw)
        with self.assertRaises(node.Refused):
            node.strict_json(b" " * 65537)
        self.assertEqual(node.strict_json(b'{"x":1}'), {"x": 1})

    def test_candidate_checks_identity_generation_and_all_eligibility_fields(self):
        good = lease()
        self.assertEqual(node.candidate(good, "node-b", "127.0.0.1:32002", 1000), "generation-b")
        mutations = [({"expires_ms": 1000}, None), ({"expires_ms": True}, None),
                     ({"node": "wrong"}, None), ({"addr": "127.0.0.1:1"}, None),
                     ({"probe_public_key": "wrong"}, None), ({"peer_protocol": True}, None),
                     ({"paced_handoff": False}, None), ({"sampled_ms": 0}, "load"),
                     ({"pressured": True}, "load"), ({"memory_headroom": False}, "load"),
                     ({"draining": True}, "load"), ({"draining": "false"}, "load")]
        for changes, field in mutations:
            changed = copy.deepcopy(good)
            (changed[field] if field else changed).update(changes)
            with self.subTest(changes=changes), self.assertRaises(node.Refused):
                node.candidate(changed, "node-b", "127.0.0.1:32002", 1000)
        with self.assertRaises(node.Refused):
            node.candidate(good, "node-b", "127.0.0.1:32002", 1000, "old-generation")
        del good["load"]["draining"]
        with self.assertRaises(node.Refused):
            node.candidate(good, "node-b", "127.0.0.1:32002", 1000)
        good["load"]["draining"] = None
        good["load"]["memory_headroom"] = None
        node.candidate(good, "node-b", "127.0.0.1:32002", 1000)

    def test_state_dormant_is_distinct_from_restore_and_empty(self):
        node.state_census(census(), "dormant")
        node.state_census(census("empty"), "empty")
        self.assertEqual(node.state_census(census("resident"), "resident"), "BaselineCell:abc")
        for bad in (census("resident"), census("empty")):
            with self.assertRaises(node.Refused):
                node.state_census(bad, "dormant")
        for change in ({"owned_cells": True}, {"restoring": 1}, {"residents": ["BaselineCell:abc"]},
                       {"phases": {"Dormant": 1}}, {"phases": {"dormant": 1, "remote": 1}}):
            with self.subTest(change=change), self.assertRaises(node.Refused):
                node.state_census(census() | change, "dormant")
        with self.assertRaises(node.Refused):
            node.state_census(census("resident"), "resident", "another-cell")

    def test_owner_keeps_identity_and_epoch_and_rejects_bool(self):
        self.assertEqual(node.owner({"node": "b", "epoch": 5}, "b", 5), 5)
        for value in ({"node": "a", "epoch": 5}, {"node": "b", "epoch": True},
                      {"node": "b", "epoch": 6}, {"node": "b", "epoch": 5, "extra": 0}):
            with self.assertRaises(node.Refused):
                node.owner(value, "b", 5)

    def test_old_node_expiry_tombstone_and_exact_missing_branches(self):
        old = lease("node-a", expiry=999)
        own = {"node": "node-b", "epoch": 6}
        args = (1000, True, own, "node-b", 6)
        self.assertEqual(node.old_node_ineligible(old, copy.deepcopy(old), None, *args), "observed-expired-node")
        tombstone = old | {"expires_ms": 0, "log": {"state": "sealed", "epoch": 1, "ensemble": [], "tiered": 0, "active": False}}
        self.assertEqual(node.old_node_ineligible(old, tombstone, None, *args), "observed-sealed-expired-node")
        no_log_tombstone = old | {"expires_ms": 0}
        for last in (old, old | {"log": None}):
            for current in (no_log_tombstone, no_log_tombstone | {"log": None}):
                with self.subTest(last_log_present="log" in last, current_log_present="log" in current):
                    self.assertEqual(node.old_node_ineligible(last, current, None, *args),
                                     "observed-no-log-expired-tombstone")
        missing = {"status": 404, "code": "NoSuchKey"}
        self.assertEqual(node.old_node_ineligible(old, None, missing, *args), "observed-no-log-key-absence")
        cases = [(old | {"log": {"state": "open"}}, None, missing),
                 (old, None, {"status": 403, "code": "NoSuchKey"}),
                 (old, None, {"status": 404, "code": "NotFound"}),
                 (old, None, None), (old, old | {"ownership_index_generation": "changed"}, None),
                 (old, old | {"expires_ms": 1001}, None),
                 (old, tombstone | {"log": {"state": "open"}}, None)]
        for prior_log in ({"state": "open"}, tombstone["log"]):
            for current in (no_log_tombstone, no_log_tombstone | {"log": None}):
                cases.append((old | {"log": prior_log}, current, None))
        for key in ("node", "addr", "ownership_index_generation", "probe_public_key"):
            cases.append((old, no_log_tombstone | {key: "changed"}, None))
        cases.append((old | {"expires_ms": 1001}, no_log_tombstone, None))
        for last, current, absence in cases:
            with self.subTest(current=current, absence=absence), self.assertRaises(node.Refused):
                node.old_node_ineligible(last, current, absence, *args)
        with self.assertRaises(node.Refused):
            node.old_node_ineligible(old, old, None, 1000, False, own, "node-b", 6)
        with self.assertRaises(node.Refused):
            node.old_node_ineligible(old, old, None, 1000, True, own, "node-b", 7)
        for retired, current_owner, live_epoch in ((False, own, 6), (True, {"node": "node-a", "epoch": 6}, 6),
                                                  (True, own, 7)):
            with self.subTest(retired=retired, owner=current_owner, epoch=live_epoch), self.assertRaises(node.Refused):
                node.old_node_ineligible(old, no_log_tombstone, None, 1000, retired, current_owner, "node-b", live_epoch)

    def test_handoff_requires_exact_order_cell_successor_and_new_epoch(self):
        raw = handoff_log()
        self.assertEqual(node.handoff_witness(raw, "BaselineCell:abc", "node-b", 5)["epoch"], 6)
        changes = [(b"successor=node-b", b"successor=node-c"), (b"successor_epoch=6", b"successor_epoch=5"),
                   (b"released_epoch=5", b"released_epoch=4"), (b"cell=BaselineCell:abc", b"cell=other"),
                   (b'event="cell_handoff_timing"', b'event="other"'), (b"total_us=15", b"total_us=unknown")]
        for old, new in changes:
            with self.subTest(old=old), self.assertRaises(node.Refused):
                node.handoff_witness(raw.replace(old, new), "BaselineCell:abc", "node-b", 5)
        lines = raw.splitlines(keepends=True)
        with self.assertRaises(node.Refused):
            node.handoff_witness(b"".join([lines[0], lines[2], lines[1], lines[3]]), "BaselineCell:abc", "node-b", 5)
        with self.assertRaises(node.Refused):
            node.handoff_witness(raw + lines[1], "BaselineCell:abc", "node-b", 5)

    def test_all_canonical_cuts_and_plain_lines_fail_even_with_status_zero(self):
        for event in node.CUT_EVENTS:
            with self.subTest(event=event), self.assertRaises(node.Refused):
                node.handoff_witness(handoff_log() + f'INFO event="{event}"\n'.encode(), "BaselineCell:abc", "node-b", 5)
        for message in (*node.CUT_LINES, "celld preserve drain reached its deadline", "SELF-FENCE: lost authority"):
            with self.subTest(message=message), self.assertRaises(node.Refused):
                node.parse_logs((message + "\n").encode())
        node.successful_exit(0, 34_999_999_999)
        for status, elapsed in ((0, 35_000_000_000), (0, -1), (1, 1), (-15, 1), (True, 1)):
            with self.assertRaises(node.Refused):
                node.successful_exit(status, elapsed)

    def test_unknown_truncated_and_unparseable_logs_cannot_supply_witness(self):
        for raw in (b'event="cell_handoff_accepted"', b"\xff\n", b"\x1b[31mcolored\n",
                    b" " * 65537 + b"\n", b'event="a" event="b"\n', b"INFO unknown\n"):
            with self.subTest(raw=raw[:40]), self.assertRaises(node.Refused):
                node.handoff_witness(raw, "BaselineCell:abc", "node-b", 5)

    def test_preserve_needs_positive_event_and_bound_marker(self):
        raw = b'INFO event="clean_reload_prepared" stale_live_databases_pruned=0\n'
        good = encode({"node": "b", "generation": "generation-b"})
        result = node.preserve_witness(raw, good, "b", "generation-b")
        self.assertEqual(result["marker_identity"], node.identity(good))
        for log, marker in ((b"", good), (raw + raw, good),
                            (raw, encode({"node": "b", "generation": "old"})),
                            (raw, encode({"node": "a", "generation": "generation-b"}))):
            with self.assertRaises(node.Refused):
                node.preserve_witness(log, marker, "b", "generation-b")

    def test_original_deadline_clock_anomaly_and_cancellation(self):
        fake = FakeTime()
        clock = fake.clock()
        first = clock.phase(15)
        fake.advance(10_000_000_000)
        self.assertEqual(clock.phase(15, first), first)
        fake.advance(5_000_000_000)
        with self.assertRaises(node.Refused):
            clock.check(first)
        fake.wall += 1_000_000_000
        clock.sample()  # Exactly one second is allowed.
        fake.wall += 1
        with self.assertRaises(node.Refused):
            clock.sample()
        fake = FakeTime()
        clock = fake.clock()
        fake.mono -= 1
        with self.assertRaises(node.Refused):
            clock.sample()
        fake = FakeTime()
        clock = fake.clock()
        clock.cancelled = True
        with self.assertRaises(node.Refused):
            clock.check()

    def test_candidate_readiness_does_not_renew_startup_budget(self):
        fake = FakeTime()
        run = object.__new__(node.Run)
        run.clock = fake.clock()
        child = mock.Mock(startup_deadline=fake.mono + 10_000_000_000)
        run.node_lease = mock.Mock()
        fake.advance(10_000_000_000)
        with self.assertRaisesRegex(node.Refused, "deadline"):
            run.ready_candidate(child)
        run.node_lease.assert_not_called()

    def test_http_exact_framing_and_bounds(self):
        self.assertEqual(node.decode_http(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc"), (200, b"abc"))
        self.assertEqual(node.decode_http(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n1\r\nc\r\n0\r\n\r\n"), (200, b"abc"))
        failures = [b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabc",
                    b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\ncontent-length: 3\r\n\r\nabc",
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n0\r\n\r\n",
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na",
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nextra",
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\nabc"]
        for raw in failures:
            with self.subTest(raw=raw), self.assertRaises(node.Refused):
                node.decode_http(raw)
        with self.assertRaises(node.Refused):
            node.decode_http(b"HTTP/1.1 200 OK\r\n\r\n" + b"x" * 65537)

    def test_closed_arguments_reject_duplicate_extra_and_port_alias(self):
        argv = ["--profile", str(ROOT / "node.lock.json"), "--binary", "/fixed/celld",
                "--binary-sha256", "0" * 64, "--directory", "/fixed/run", "--deadline-ns", "100",
                "--node-lock-sha256", node.PROFILE_SHA256]
        for index, role in enumerate(node.ROLES):
            argv += ["--" + role.replace("_", "-") + "-port", str(30000 + index)]
        self.assertEqual(node.parse_arguments(argv).s3_port, 30000)
        for bad in (argv + ["--unknown", "1"], argv[:-2] + ["--s3-port", "30000"],
                    argv[:-1] + ["30000"], argv[:-1] + ["true"]):
            with self.assertRaises((node.Refused, SystemExit)):
                node.parse_arguments(bad)

    def test_raw_storage_capture_preserves_errors_without_consuming_success_stream(self):
        storage = object.__new__(node.Storage)
        storage.raw_response = threading.local()
        response = mock.Mock(status_code=404, content=b"<Error><Code>NoSuchKey</Code></Error>")
        model = mock.Mock()
        model.name = "GetObject"
        storage.capture_response(response, model)
        self.assertEqual(storage.raw_response.body, response.content)
        storage.raw_response.body = None
        response.status_code = 200
        storage.capture_response(response, model)
        self.assertIsNone(storage.raw_response.body)
        response.status_code, response.content = 500, b"x" * 65537
        with self.assertRaises(node.Refused):
            storage.capture_response(response, model)

    def test_pid_attribution_is_transitive_and_strict(self):
        table = node.parse_process_table(b"10 1 10 2\n11 10 10 3\n12 11 10 5\n20 1 20 2\n")
        self.assertEqual(node.descendants(table, {10}), {10, 11, 12})
        self.assertEqual(table[12], (11, 10, 5))
        for raw in (b"", b"1 2\n", b"1 a 2 3\n", b"1 2 3 4\n1 4 5 2\n",
                    b"0 0 0 2\n", b"1 2 3 0\n", b"1 2 3 6\n", b"1 2 3 -1\n",
                    b"2147483648 1 1 2\n", b"1 2147483648 1 2\n", b"1 1 2147483648 2\n",
                    b"1" * 11 + b" 1 1 2\n", b"x" * 65537):
            with self.subTest(raw=raw[:60]), self.assertRaises(node.Refused):
                node.parse_process_table(raw)
        with self.assertRaises(UnicodeError):
            node.parse_process_table(b"\xff 1 1 2\n")


class NativeInspectorTests(unittest.TestCase):
    def setUp(self):
        # Load only definitions. Native library loading is behind main/bind_library;
        # deterministic tests below supply finite fake syscall results.
        self.inspector = {"__name__": "foks_native_inspector_definitions"}
        with mock.patch("ctypes.CDLL", side_effect=AssertionError("native load during definitions")):
            exec(compile(node.PROCESS_INSPECTOR, "<fixed-process-inspector>", "exec"), self.inspector)
        self.ctypes = self.inspector["ctypes"]
        self.base = {
            100: {"ppid": 1, "pgid": 100, "status": 2, "uid": 501},
            101: {"ppid": 100, "pgid": 100, "status": 2, "uid": 501},
            400: {"ppid": 100, "pgid": 100, "status": 5, "uid": 501},
        }

    def api(self, enumerations=None, entries=None):
        pending = list(enumerations if enumerations is not None else ([0, 100, 101, 400],) * 2)
        data = self.base if entries is None else entries

        def list_pids(buffer, size):
            self.assertEqual(size, 4096 * 4)
            value = pending.pop(0)
            if type(value) is int:
                return value
            for index, pid in enumerate(value):
                buffer[index] = pid
            return len(value)

        def info(pid, flavor, arg, pointer, size):
            self.assertEqual((flavor, arg, size), (13, 1, 64))
            value = data[pid]
            if isinstance(value, tuple):
                result, error = value
                self.ctypes.set_errno(error)
                return result
            target = pointer._obj
            target.pid = pid
            for name, value in value.items():
                setattr(target, name, value)
            return 64

        api = mock.Mock()
        api.proc_listallpids.side_effect = list_pids
        api.proc_pidinfo.side_effect = info
        return api

    def rows(self, api):
        return self.inspector["process_rows"](api, 101, 100, 100, 501)

    def test_fixed_abi_binding_and_profile_match(self):
        self.inspector["check_abi"]()
        descriptor = profile()["launch"]["process_inspection"]
        self.assertEqual(descriptor["source"], node.identity(node.PROCESS_INSPECTOR.encode()))
        structure = self.inspector["ShortBSD"]
        self.assertEqual(descriptor["abi"]["fields"], [[name, getattr(structure, name).offset] for name, _ in structure._fields_])
        self.assertEqual((self.ctypes.sizeof(structure), self.ctypes.alignment(structure)),
                         (descriptor["abi"]["bytes"], descriptor["abi"]["alignment"]))
        self.assertEqual((self.inspector["CAPACITY"], self.inspector["OUTPUT_BYTES"],
                          self.inspector["FLAVOR"], self.inspector["INCLUDE_ZOMBIES"]),
                         (4096, 65536, descriptor["abi"]["flavor"], descriptor["abi"]["arg"]))
        library = mock.Mock()
        with mock.patch.object(self.ctypes, "CDLL", return_value=library) as load, \
                mock.patch.object(self.inspector["sys"], "platform", "darwin"), \
                mock.patch.object(self.inspector["os"], "uname", return_value=mock.Mock(machine="arm64")):
            self.assertIs(self.inspector["bind_library"](), library)
        load.assert_called_once_with(descriptor["library"], mode=self.ctypes.RTLD_LOCAL, use_errno=True)
        self.assertEqual(library.proc_listallpids.argtypes, [self.ctypes.c_void_p, self.ctypes.c_int])
        self.assertIs(library.proc_listallpids.restype, self.ctypes.c_int)
        self.assertEqual(library.proc_pidinfo.argtypes, [self.ctypes.c_int, self.ctypes.c_int, self.ctypes.c_uint64, self.ctypes.c_void_p, self.ctypes.c_int])
        self.assertIs(library.proc_pidinfo.restype, self.ctypes.c_int)

    def test_finite_enumeration_and_kernel_zero_rules_apply_to_both_passes(self):
        self.assertEqual(self.inspector["enumerate_pids"](self.api(enumerations=[[0, 100, 101]])), {100, 101})
        for values in (0, -1, 4096, 4097, [0, 0, 100], [-1, 100], [100, 100]):
            for second in (False, True):
                api = self.api(enumerations=([100, 101, 400], values) if second else (values,))
                with self.subTest(values=values, second=second), self.assertRaises(RuntimeError):
                    self.rows(api)

    def test_native_status_survives_private_table_and_zero_is_excluded(self):
        api = self.api()
        rows = self.rows(api)
        self.assertNotIn(0, rows)
        self.assertEqual(rows[400], (100, 100, 5, 501))
        encoded = self.inspector["encode_rows"](rows)
        self.assertEqual(node.parse_process_table(encoded)[400], (100, 100, 5))
        self.assertEqual(api.proc_listallpids.call_count, 2)
        self.assertEqual(api.proc_pidinfo.call_count, 3)

    def test_permission_partial_and_unknown_errors_are_never_absence(self):
        for result, error in ((0, node.errno.EPERM), (0, node.errno.EACCES), (0, 0),
                              (0, node.errno.EIO), (-1, node.errno.ESRCH), (63, 0), (65, 0)):
            data = {**self.base, 400: (result, error)}
            with self.subTest(result=result, errno=error), self.assertRaisesRegex(RuntimeError, "failed or partial"):
                self.rows(self.api(entries=data))

    def test_esrch_omission_requires_second_enumeration_absence(self):
        data = {**self.base, 400: (0, node.errno.ESRCH)}
        rows = self.rows(self.api(enumerations=([100, 101, 400], [100, 101]), entries=data))
        self.assertNotIn(400, rows)
        with self.assertRaisesRegex(RuntimeError, "reappeared"):
            self.rows(self.api(entries=data))
        # A required witness cannot be silently omitted even after ESRCH confirmation.
        data = {**self.base, 101: (0, node.errno.ESRCH)}
        with self.assertRaisesRegex(RuntimeError, "missing owned"):
            self.rows(self.api(enumerations=([100, 101, 400], [100, 400]), entries=data))

    def test_record_identity_status_and_witnesses_are_mandatory(self):
        for target, key, value in ((400, "pid", 401), (400, "ppid", 2147483648),
                                   (400, "pgid", 2147483648), (400, "status", 0), (400, "status", 6),
                                   (101, "ppid", 99), (101, "pgid", 99), (101, "uid", 0),
                                   (100, "pgid", 99), (100, "uid", 0)):
            data = copy.deepcopy(self.base)
            data[target][key] = value
            with self.subTest(target=target, key=key, value=value), self.assertRaises(RuntimeError):
                self.rows(self.api(entries=data))
        for second in ([100, 400], [101, 400]):
            with self.subTest(second=second), self.assertRaisesRegex(RuntimeError, "missing owned"):
                self.rows(self.api(enumerations=([100, 101, 400], second)))

    def test_source_and_parent_witness_refusals_do_not_authorize_output(self):
        actual = profile()
        argv = node.process_inspector_command(actual, 100, 100)
        self.assertEqual(argv[:5], [actual["launch"]["python"], "-I", "-S", "-B", "-c"])
        self.assertEqual(argv[6:], ["100", "100"])
        self.assertIn(repr(actual["launch"]["sys_path"][:2]), argv[5])
        self.assertNotIn(actual["launch"]["sys_path"][2], argv[5])
        changed = copy.deepcopy(actual)
        changed["launch"]["process_inspection"]["source"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(node.Refused, "source changed"):
            node.process_inspector_command(changed, 100, 100)
        good = {100: (1, 100, 2), 101: (100, 100, 2)}
        node.check_process_witnesses(good, 101, 100, 1, 100)
        for table in ({100: good[100]}, {101: good[101]}, {**good, 101: (99, 100, 2)},
                      {**good, 101: (100, 99, 2)}, {**good, 100: (2, 100, 2)},
                      {**good, 100: (1, 99, 2)}):
            with self.subTest(table=table), self.assertRaises(node.Refused):
                node.check_process_witnesses(table, 101, 100, 1, 100)
        with self.assertRaisesRegex(RuntimeError, "output bound"):
            self.inspector["encode_rows"]({pid: (2147483647, 2147483647, 5, 501)
                                           for pid in range(1_000_000, 1_004_096)})


class InspectionBoundaries(unittest.TestCase):
    def children(self):
        return node.Children(FakeTime().clock(), Path("/unused"), {}, profile(), isolated=False)

    def helper(self):
        helper = mock.Mock(pid=123456, returncode=0)
        helper.stdout, helper.stderr = mock.Mock(), mock.Mock()
        helper.stdout.fileno.return_value, helper.stderr.fileno.return_value = 101, 102
        helper.poll.return_value = None
        helper.wait.return_value = 0
        return helper

    def test_only_the_earlier_caller_deadline_is_phase_expiry(self):
        with mock.patch.object(node.time, "monotonic_ns", return_value=10):
            self.assertEqual(node.inspection_remaining_ns(20, 30), 10)
            with self.assertRaises(node.InspectionPhaseExpired):
                node.inspection_remaining_ns(10, 30)
            for deadline, helper_deadline in ((30, 10), (10, 10), (5, 9)):
                with self.subTest(deadline=deadline, helper_deadline=helper_deadline):
                    with self.assertRaises(node.Refused) as caught:
                        node.inspection_remaining_ns(deadline, helper_deadline)
                    self.assertNotIsInstance(caught.exception, node.InspectionPhaseExpired)

    def test_command_validation_cannot_renew_or_outlive_original_helper_deadline(self):
        children = self.children()
        with mock.patch.object(node.subprocess, "Popen") as spawn, \
                mock.patch.object(node, "process_inspector_command", wraps=node.process_inspector_command) as command, \
                mock.patch.object(node, "inspection_remaining_ns", side_effect=[100, node.InspectionPhaseExpired("caller boundary")]):
            with self.assertRaises(node.InspectionPhaseExpired):
                children.process_table(100)
        command.assert_called_once_with(children.profile, os.getpid(), children.group)
        spawn.assert_not_called()
        children.profile["launch"]["process_inspection"]["source"]["sha256"] = "0" * 64
        with mock.patch.object(node.subprocess, "Popen") as spawn, \
                mock.patch.object(node, "inspection_remaining_ns", return_value=100):
            with self.assertRaisesRegex(node.Refused, "source changed"):
                children.process_table(100)
        spawn.assert_not_called()

    def test_complete_native_output_requires_helper_witness_before_disposal(self):
        children = self.children()
        for valid in (False, True):
            with self.subTest(valid=valid):
                helper = self.helper()
                helper.poll.return_value = 0
                rows = f"{os.getpid()} {os.getppid()} {children.group} 2\n"
                if valid:
                    rows += f"{helper.pid} {os.getpid()} {children.group} 2\n"
                reads = {101: [rows.encode(), b""], 102: [b""]}
                with mock.patch.object(node.subprocess, "Popen", return_value=helper), \
                        mock.patch.object(node.os, "set_blocking"), \
                        mock.patch.object(node.select, "select", side_effect=lambda streams, *_: (list(streams), [], [])), \
                        mock.patch.object(node.os, "read", side_effect=lambda fd, _: reads[fd].pop(0)), \
                        mock.patch.object(node, "inspection_remaining_ns", return_value=100):
                    if valid:
                        table = children.process_table(100)
                        self.assertEqual(table, {os.getpid(): (os.getppid(), children.group, 2)})
                        self.assertNotIn(helper.pid, table)
                    else:
                        with self.assertRaisesRegex(node.Refused, "missing inspection"):
                            children.process_table(100)
                self.assertEqual(helper.wait.call_count, 2)
                self.assertEqual(helper.wait.call_args, mock.call(timeout=3))
                helper.kill.assert_not_called()
                helper.stdout.close.assert_called_once_with()
                helper.stderr.close.assert_called_once_with()

    def test_read_phase_expiry_cannot_escape_before_helper_disposition(self):
        for dispose_failure in (False, True):
            with self.subTest(dispose_failure=dispose_failure):
                helper = self.helper()
                if dispose_failure:
                    helper.wait.side_effect = subprocess.TimeoutExpired("ps", 3)
                with mock.patch.object(node.subprocess, "Popen", return_value=helper), \
                        mock.patch.object(node.os, "set_blocking"), \
                        mock.patch.object(node, "inspection_remaining_ns", side_effect=[
                            100, 100, node.InspectionPhaseExpired("caller boundary")]):
                    expected = subprocess.TimeoutExpired if dispose_failure else node.InspectionPhaseExpired
                    with self.assertRaises(expected):
                        self.children().process_table(100)
                helper.kill.assert_called_once_with()
                helper.wait.assert_called_once_with(timeout=3)
                helper.stdout.close.assert_called_once_with()
                helper.stderr.close.assert_called_once_with()

    def test_wait_timeout_rechecks_the_same_deadline_and_reaps_helper(self):
        for boundary in (node.InspectionPhaseExpired("caller boundary"),
                         node.Refused("process inspection timeout"), 100):
            with self.subTest(boundary=boundary):
                helper = self.helper()
                helper.wait.side_effect = [subprocess.TimeoutExpired("ps", 0.1), 0]
                with mock.patch.object(node.subprocess, "Popen", return_value=helper), \
                        mock.patch.object(node.os, "set_blocking"), \
                        mock.patch.object(node.select, "select", return_value=([helper.stdout, helper.stderr], [], [])), \
                        mock.patch.object(node.os, "read", return_value=b""), \
                        mock.patch.object(node, "inspection_remaining_ns", side_effect=[100, 100, 100, 100, boundary]):
                    with self.assertRaises(node.Refused) as caught:
                        self.children().process_table(100)
                self.assertEqual(isinstance(caught.exception, node.InspectionPhaseExpired),
                                 isinstance(boundary, node.InspectionPhaseExpired))
                if boundary == 100:
                    self.assertIn("wait failed", str(caught.exception))
                helper.kill.assert_called_once_with()
                self.assertEqual(helper.wait.call_count, 2)
                self.assertEqual(helper.wait.call_args, mock.call(timeout=3))
                helper.stdout.close.assert_called_once_with()
                helper.stderr.close.assert_called_once_with()

    def owned_child(self, exited=False):
        children = self.children()
        process = mock.Mock(pid=123456, returncode=0 if exited else None)
        process.poll.side_effect = lambda: process.returncode

        def signal_child(sig):
            if sig == signal.SIGKILL:
                process.returncode = -signal.SIGKILL

        def wait_child(timeout):
            if process.returncode is None:
                raise subprocess.TimeoutExpired("owned child", timeout)
            return process.returncode

        process.send_signal.side_effect = signal_child
        process.wait.side_effect = wait_child
        child = node.Child("fixture", process, Path("/unused/fixture.log"), 1, ["fixture"])
        children.items.append(child)
        return children, child, process

    def test_term_phase_expiry_forces_fresh_kill_and_final_inspection(self):
        for at_initial_scan in (True, False):
            with self.subTest(at_initial_scan=at_initial_scan):
                children, child, process = self.owned_child()
                stale_pid = 123457
                child.descendants.add(stale_pid)
                children.table = {stale_pid: (process.pid, children.group, 2)}
                calls = []
                expiry_call = 1 if at_initial_scan else 2

                def scan(deadline, force):
                    self.assertTrue(force)
                    calls.append(deadline)
                    if len(calls) == expiry_call:
                        # The loop case leaves an old empty table. It still must
                        # enter KILL rather than claiming absence from that table.
                        if not at_initial_scan:
                            self.assertEqual(children.table, {})
                        raise node.InspectionPhaseExpired("TERM boundary")
                    children.table = ({process.pid: (os.getpid(), children.group, 2)}
                                      if len(calls) == expiry_call + 1 else {})

                with mock.patch.object(children, "scan", side_effect=scan), \
                        mock.patch.object(node.os, "kill", side_effect=AssertionError("stale PID signaled")):
                    result = children.cleanup()
                self.assertTrue(result["completed"], result)
                self.assertIsNone(result["error"])
                self.assertTrue(child.retired)
                expected = [mock.call(signal.SIGKILL)] if at_initial_scan else [
                    mock.call(signal.SIGTERM), mock.call(signal.SIGKILL)]
                self.assertEqual(process.send_signal.call_args_list, expected)
                self.assertEqual(len(calls), expiry_call + 3)
                self.assertEqual(children.table, {})

    def test_real_inspection_errors_remain_sticky_after_later_absence(self):
        for fault in (node.Refused("process table syntax"), node.Refused("unattributed process"),
                      node.Refused("child escaped owned group"), PermissionError("inspection denied")):
            with self.subTest(fault=fault):
                children, child, _ = self.owned_child(exited=True)
                stale_pid = 123457
                child.descendants.add(stale_pid)
                children.table = {stale_pid: (child.process.pid, children.group, 2)}
                calls = []

                def scan(deadline, force):
                    calls.append(deadline)
                    if len(calls) == 1:
                        raise fault
                    children.table = {}

                with mock.patch.object(children, "scan", side_effect=scan), \
                        mock.patch.object(node.os, "kill", side_effect=AssertionError("stale PID signaled")):
                    result = children.cleanup()
                self.assertFalse(result["completed"])
                self.assertTrue(child.retired)
                self.assertTrue(result["error"])
                self.assertEqual(len(calls), 3)  # initial, TERM loop, final

    def test_final_inspection_expiry_is_not_a_successful_cleanup(self):
        children, child, _ = self.owned_child(exited=True)
        calls = []

        def scan(deadline, force):
            calls.append(deadline)
            if len(calls) == 3:
                raise node.InspectionPhaseExpired("final boundary")
            children.table = {}

        with mock.patch.object(children, "scan", side_effect=scan):
            result = children.cleanup()
        self.assertFalse(result["completed"])
        self.assertFalse(child.retired)
        self.assertIn("final child snapshot incomplete", result["error"][0])
        self.assertEqual(len(calls), 3)

    def test_kill_inspection_expiry_stays_fatal_despite_final_absence(self):
        children, child, process = self.owned_child(exited=True)
        calls = []

        def scan(deadline, force):
            calls.append(deadline)
            if len(calls) <= 2:
                raise node.InspectionPhaseExpired("cleanup phase boundary")
            children.table = {}

        with mock.patch.object(children, "scan", side_effect=scan):
            result = children.cleanup()
        self.assertFalse(result["completed"])
        self.assertTrue(child.retired)
        self.assertEqual(len(result["error"]), 1)
        self.assertIn("InspectionPhaseExpired", result["error"][0])
        self.assertEqual(len(calls), 3)
        process.send_signal.assert_not_called()


class ActualBoundedFixtures(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="a3a-controller-test-")
        self.directory = Path(self.temporary.name).resolve()
        self.clock = node.Clock(time.monotonic_ns() + 30_000_000_000)
        self.environment = {"PATH": "/usr/bin:/bin", "TMPDIR": str(self.directory)}
        # Tests must not adopt or kill the test runner's other group members.
        self.children = node.Children(self.clock, self.directory, self.environment, profile(), isolated=False)
        self.last_cleanup = None

    def tearDown(self):
        try:
            if self.last_cleanup is None:
                self.last_cleanup = self.children.cleanup()
            self.assertTrue(self.last_cleanup["completed"], self.last_cleanup)
        finally:
            self.temporary.cleanup()

    def wait_file(self, path, deadline=None):
        deadline = deadline or time.monotonic_ns() + 5_000_000_000
        while not path.exists():
            self.children.tick(deadline)
            threading.Event().wait(0.01)
        return path.read_bytes()

    def sleeping_child(self, name="moto"):
        ready = self.directory / (name + ".ready")
        source = ("import pathlib,signal,sys; "
                  "signal.signal(signal.SIGTERM,lambda *_:sys.exit(0)); "
                  f"pathlib.Path({str(ready)!r}).write_bytes(b'ready'); "
                  "signal.pause()")
        child = self.children.spawn(name, [sys.executable, "-I", "-S", "-B", "-c", source])
        self.wait_file(ready)
        return child

    def test_real_signal_reaches_child_without_inherited_mask(self):
        child = self.sleeping_child()
        child.expected_exit = True
        child.process.send_signal(signal.SIGTERM)
        self.assertEqual(self.children.wait(child, self.clock.phase(5)), 0)
        self.assertTrue(child.retired)
        self.assertNotIn(child.process.pid, self.children.table)

    def test_real_timeout_keeps_handle_until_cleanup(self):
        child = self.sleeping_child()
        with self.assertRaises(node.Refused):
            self.children.wait(child, time.monotonic_ns() - 1)
        self.assertIsNone(child.process.poll())
        self.last_cleanup = self.children.cleanup()
        self.assertTrue(self.last_cleanup["completed"])
        self.assertEqual(child.process.returncode, 0)

    def test_two_children_cleanup_when_second_exits_unexpectedly(self):
        first = self.sleeping_child("first")
        second = self.children.spawn("second", [sys.executable, "-I", "-S", "-B", "-c", "raise SystemExit(7)"])
        second.process.wait(timeout=5)
        with self.assertRaises(node.Refused):
            self.children.tick()
        self.assertIsNone(first.process.poll())
        self.last_cleanup = self.children.cleanup()
        self.assertTrue(self.last_cleanup["completed"])
        self.assertEqual(first.process.returncode, 0)
        self.assertEqual(second.process.returncode, 7)

    def test_retired_pid_is_never_signaled_again(self):
        child = self.sleeping_child()
        child.expected_exit = True
        child.process.send_signal(signal.SIGTERM)
        self.children.wait(child, self.clock.phase(5))
        with mock.patch.object(node.os, "kill", side_effect=AssertionError("retired PID signaled")):
            self.last_cleanup = self.children.cleanup()
        self.assertTrue(self.last_cleanup["completed"])

    def test_unknown_scan_never_reports_cleanup_success(self):
        child = self.sleeping_child()
        with mock.patch.object(self.children, "scan", side_effect=node.Refused("unknown attribution")):
            result = self.children.cleanup()
        self.assertFalse(result["completed"])
        self.assertIsNotNone(child.process.poll())
        # A real fresh scan is still required before this test releases its fixture.
        self.last_cleanup = self.children.cleanup()

    def test_parent_exit_does_not_retire_its_observed_live_child(self):
        ready, gate = self.directory / "grand.ready", self.directory / "parent.exit"
        grand_source = ("import pathlib,signal; signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                        f"pathlib.Path({str(ready)!r}).write_text('ready'); signal.pause()")
        parent_source = ("import pathlib,subprocess,sys,time; "
                         f"p=subprocess.Popen([sys.executable,'-I','-S','-B','-c',{grand_source!r}]); "
                         f"gate=pathlib.Path({str(gate)!r}); "
                         "exec('while not gate.exists(): time.sleep(0.01)')")
        child = self.children.spawn("parent", [sys.executable, "-I", "-S", "-B", "-c", parent_source])
        self.wait_file(ready)
        self.children.scan(self.clock.phase(5), force=True)
        self.assertEqual(len(child.descendants), 2)
        gate.write_bytes(b"exit")
        child.expected_exit = True
        with self.assertRaisesRegex(node.Refused, "descendants remain"):
            self.children.wait(child, self.clock.phase(5))
        self.assertFalse(child.retired)
        self.last_cleanup = self.children.cleanup()
        self.assertTrue(self.last_cleanup["completed"], self.last_cleanup)

    def test_native_helper_itself_is_not_unattributed_work(self):
        self.children.scan(self.clock.phase(5), force=True)
        self.assertFalse(any(parent == os.getpid() for parent, _, _ in self.children.table.values()))
        with mock.patch.object(self.children, "process_table", return_value={os.getpid(): (1, self.children.group, 2), 123456: (1, self.children.group, 2)}):
            self.children.isolated = True
            with self.assertRaisesRegex(node.Refused, "unattributed"):
                self.children.scan(self.clock.phase(5), force=True)
            self.children.isolated = False

    def test_port_reuse_requires_retirement(self):
        first = self.sleeping_child()
        first.node, first.ports = "a", {"public": 30001, "internal": 30002}
        with self.assertRaisesRegex(node.Refused, "port reuse"):
            self.children.spawn("c", [sys.executable, "-c", "pass"], node="c", ports={"public": 30001, "internal": 30002})

    def test_real_http_fully_consumes_response_and_closes_connection(self):
        errors = []
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(5)
            port = listener.getsockname()[1]

            def serve():
                try:
                    with listener.accept()[0] as connection:
                        connection.settimeout(5)
                        request = bytearray()
                        while b"\r\n\r\n" not in request:
                            part = connection.recv(4096)
                            if not part or len(request) + len(part) > 4096:
                                raise AssertionError("request fixture framing")
                            request.extend(part)
                        connection.sendall(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n1\r\nc\r\n0\r\n\r\n")
                except BaseException as error:
                    errors.append(error)

            server = threading.Thread(target=serve)
            server.start()
            try:
                result = node.http_exchange(port, "GET", "/read", b"", self.clock.phase(5), self.clock)
                self.assertEqual(result, (200, b"abc"))
            finally:
                server.join(timeout=5)
            self.assertFalse(server.is_alive())
            self.assertFalse(errors, errors)

    def test_real_http_uses_absolute_deadline_and_closes_client(self):
        accepted, release, peer_closed = threading.Event(), threading.Event(), threading.Event()
        errors = []
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(5)
            port = listener.getsockname()[1]

            def serve():
                try:
                    with listener.accept()[0] as connection:
                        connection.settimeout(5)
                        request = bytearray()
                        while b"\r\n\r\n" not in request:
                            part = connection.recv(4096)
                            if not part:
                                raise AssertionError("request truncated before barrier")
                            request.extend(part)
                            if len(request) > 4096:
                                raise AssertionError("request fixture bound")
                        if not request.endswith(b"\r\n\r\n"):
                            raise AssertionError("unexpected request body")
                        accepted.set()
                        release.wait(5)
                        if connection.recv(4096) == b"":
                            peer_closed.set()
                except BaseException as error:
                    errors.append(error)

            server = threading.Thread(target=serve)
            server.start()
            fake = FakeTime()
            clock = fake.clock()
            deadline = clock.phase(15)

            def tick():
                if accepted.is_set():
                    fake.advance(15_000_000_000)

            try:
                with self.assertRaisesRegex(node.Refused, "deadline"):
                    node.http_exchange(port, "GET", "/read", b"", deadline, clock, tick)
            finally:
                release.set()
                server.join(timeout=5)
            self.assertFalse(server.is_alive())
            self.assertFalse(errors, errors)
            self.assertTrue(peer_closed.is_set())

    def test_storage_task_cannot_be_forgotten_on_timeout(self):
        started, release = threading.Event(), threading.Event()
        store = object.__new__(node.Storage)
        store.pending, store.finished = [], 0
        store.clock, store.tick = self.clock, lambda deadline=None: None
        store.client = mock.Mock()

        def operation():
            started.set()
            release.wait(5)
            return "done"

        task = store.start(operation)
        self.assertTrue(started.wait(5))
        try:
            with self.assertRaises(node.Refused):
                store.finish(task, time.monotonic_ns() - 1)
            self.assertEqual(store.pending, [task])
            self.assertTrue(task.thread.is_alive())
        finally:
            release.set()
            self.assertTrue(store.cleanup())
        self.assertFalse(task.thread.is_alive())
        self.assertEqual(store.pending, [])

    def test_log_and_evidence_bounds_refuse_with_owned_child_cleanup(self):
        child = self.sleeping_child()
        with child.output.open("r+b") as output:
            output.truncate(33_554_433)
        with self.assertRaisesRegex(node.Refused, "log bound"):
            self.children.tick()
        evidence = node.Evidence(self.directory, self.clock)
        large = self.directory / "large"
        with large.open("wb") as output:
            output.truncate(268_435_457)
        with self.assertRaisesRegex(node.Refused, "payload bound"):
            evidence.check(force=True)
        large.unlink()
        with child.output.open("r+b") as output:
            output.truncate(0)
        self.last_cleanup = self.children.cleanup()

    def test_report_write_failure_occurs_after_child_cleanup_and_restores_handlers(self):
        child = self.sleeping_child()
        fake_run = mock.Mock()
        fake_run.children, fake_run.storage = self.children, None
        fake_run.active_responses, fake_run.nodes, fake_run.steps = 0, [], []
        fake_run.directory, fake_run.run_id = self.directory, "0" * 32
        fake_run.ports = dict(zip(node.ROLES, range(30001, 30006)))
        args = mock.Mock(directory=str(self.directory), deadline_ns=time.monotonic_ns() + 20_000_000_000,
                         binary="/fixed/celld", binary_sha256="0" * 64, profile=str(ROOT / "node.lock.json"))
        handlers = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        with mock.patch.object(node, "parse_arguments", return_value=args), \
                mock.patch.object(node, "validate_inputs", return_value={}), \
                mock.patch.object(node, "check_environment", return_value=self.environment), \
                mock.patch.object(node, "startup_observation", side_effect=lambda *_: {}), \
                mock.patch.object(node, "fixture_versions", return_value={}), \
                mock.patch.object(node, "Run", return_value=fake_run), \
                mock.patch.object(node, "save_report", side_effect=OSError("injected report failure")):
            with self.assertRaisesRegex(OSError, "report failure"):
                node.main([])
        self.assertEqual(child.process.returncode, 0)
        self.assertTrue(child.retired)
        self.assertEqual({sig: signal.getsignal(sig) for sig in handlers}, handlers)
        self.last_cleanup = self.children.cleanup()


if __name__ == "__main__":
    unittest.main()

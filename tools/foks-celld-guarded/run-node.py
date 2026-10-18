#!/usr/bin/env python3
"""Bounded ordinary-celld local baseline; no whole-node teardown claim."""

WORKER_JS = b"""export class BaselineCell {
  constructor(state) {
    this.state = state;
    this.sql = state.storage.sql;
    this.sql.exec("CREATE TABLE IF NOT EXISTS marker (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL)").toArray();
  }
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (request.method === "POST" && path === "/write") {
      if (!request.body) return new Response("missing marker", {status:400});
      const reader = request.body.getReader();
      const pieces = []; let length = 0;
      try {
        while (true) {
          const part = await reader.read();
          if (part.done) break;
          if (part.value.length === 0) continue;
          length += part.value.length;
          if (length > 36) { await reader.cancel(); return new Response("oversized marker", {status:413}); }
          pieces.push(part.value);
        }
      } finally { reader.releaseLock(); }
      const bytes = new Uint8Array(length); let offset = 0;
      for (const piece of pieces) { bytes.set(piece, offset); offset += piece.length; }
      const value = new TextDecoder().decode(bytes);
      if (!/^[a-f0-9]{32}-(one|two)$/.test(value)) return new Response("invalid marker", {status:400});
      this.sql.exec("INSERT INTO marker(id,value) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value", value).toArray();
      await this.state.storage.sync();
    } else if (request.method !== "GET" || path !== "/read") {
      return new Response("unsupported", {status:404});
    }
    const rows = this.sql.exec("SELECT value FROM marker WHERE id=1").toArray();
    return new Response(JSON.stringify({fixture:"foks-celld-a3a-v1", marker:rows.length ? rows[0].value : null, rows:rows.length}), {headers:{"content-type":"application/json"}});
  }
}
export default {
  fetch(request, env) {
    return env.BASELINE.get(env.BASELINE.idFromName("single")).fetch(request);
  }
};
"""
WRANGLER_JSON = b"""{"compatibility_date":"2026-01-01","durable_objects":{"bindings":[{"class_name":"BaselineCell","name":"BASELINE"}]},"main":"worker.mjs","migrations":[{"new_sqlite_classes":["BaselineCell"],"tag":"v1"}],"name":"foks-celld-a3a","no_bundle":true}
"""

# A private subprocess program: definitions are testable without loading libproc.
# The fixed caller supplies only its real PID/group, never an executable or ABI.
PROCESS_INSPECTOR = r"""
import ctypes
import errno
import os
import sys

CAPACITY = 4096
OUTPUT_BYTES = 65536
FLAVOR = 13
INCLUDE_ZOMBIES = 1
LIBRARY = '/usr/lib/libproc.dylib'

class ShortBSD(ctypes.Structure):
    _layout_ = 'gcc-sysv'
    _fields_ = [
        ('pid', ctypes.c_uint32), ('ppid', ctypes.c_uint32),
        ('pgid', ctypes.c_uint32), ('status', ctypes.c_uint32),
        ('comm', ctypes.c_char * 16), ('flags', ctypes.c_uint32),
        ('uid', ctypes.c_uint32), ('gid', ctypes.c_uint32),
        ('ruid', ctypes.c_uint32), ('rgid', ctypes.c_uint32),
        ('svuid', ctypes.c_uint32), ('svgid', ctypes.c_uint32),
        ('reserved', ctypes.c_uint32),
    ]

def check(condition, message):
    if not condition:
        raise RuntimeError(message)

def check_abi():
    check(ctypes.sizeof(ctypes.c_int) == 4 and ctypes.sizeof(ctypes.c_uint32) == 4
          and ctypes.sizeof(ctypes.c_uint64) == 8, 'native integer ABI')
    check(ctypes.sizeof(ShortBSD) == 64 and ctypes.alignment(ShortBSD) == 4,
          'short BSD ABI size/alignment')
    expected = [('pid', 0), ('ppid', 4), ('pgid', 8), ('status', 12),
                ('comm', 16), ('flags', 32), ('uid', 36), ('gid', 40),
                ('ruid', 44), ('rgid', 48), ('svuid', 52), ('svgid', 56), ('reserved', 60)]
    check([(name, getattr(ShortBSD, name).offset) for name, _ in ShortBSD._fields_] == expected,
          'short BSD field layout')

def bind_library():
    check(sys.platform == 'darwin' and os.uname().machine == 'arm64', 'native process platform')
    check_abi()
    library = ctypes.CDLL(LIBRARY, mode=ctypes.RTLD_LOCAL, use_errno=True)
    library.proc_listallpids.argtypes = [ctypes.c_void_p, ctypes.c_int]
    library.proc_listallpids.restype = ctypes.c_int
    library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                   ctypes.c_void_p, ctypes.c_int]
    library.proc_pidinfo.restype = ctypes.c_int
    return library

def enumerate_pids(library):
    buffer = (ctypes.c_int * CAPACITY)()
    ctypes.set_errno(0)
    count = library.proc_listallpids(buffer, ctypes.sizeof(buffer))
    check(0 < count < CAPACITY, 'process enumeration failed or full')
    values = [int(buffer[index]) for index in range(count)]
    check(values.count(0) <= 1 and all(value >= 0 for value in values), 'invalid kernel/PID row')
    positive = [value for value in values if value > 0]
    check(len(positive) == len(set(positive)), 'duplicate positive PID')
    # PID0 is the explicit kernel special case, never an owned-child identity.
    return set(positive)

def process_rows(library, own_pid, controller, group, uid):
    check(all(type(value) is int and 0 < value <= 2147483647
              for value in (own_pid, controller, group)), 'owned process identity')
    check(type(uid) is int and 0 <= uid <= 4294967295 and own_pid != controller,
          'owned process UID/identity')
    listed = enumerate_pids(library)
    rows, vanished = {}, set()
    for pid in sorted(listed):
        info = ShortBSD()
        ctypes.set_errno(0)
        size = library.proc_pidinfo(pid, FLAVOR, INCLUDE_ZOMBIES,
                                    ctypes.byref(info), ctypes.sizeof(info))
        if size == 0 and ctypes.get_errno() == errno.ESRCH:
            vanished.add(pid)
            continue
        check(size == 64, 'process info failed or partial')
        check(info.pid == pid and info.ppid <= 2147483647 and info.pgid <= 2147483647
              and 1 <= info.status <= 5, 'process info identity/status')
        rows[pid] = (int(info.ppid), int(info.pgid), int(info.status), int(info.uid))
    second = enumerate_pids(library)
    check(not vanished.intersection(second), 'vanished PID reappeared; incomplete snapshot')
    check(all(pid in rows and pid in second for pid in (own_pid, controller)),
          'missing owned process witness')
    check(rows[own_pid][0] == controller and rows[own_pid][1] == group
          and rows[own_pid][3] == uid and rows[controller][1] == group
          and rows[controller][3] == uid, 'owned process witness mismatch')
    return rows

def encode_rows(rows):
    result = bytearray()
    for pid, (parent, group, status, _uid) in sorted(rows.items()):
        line = ('%d %d %d %d\n' % (pid, parent, group, status)).encode('ascii')
        check(len(result) + len(line) <= OUTPUT_BYTES, 'process output bound')
        result.extend(line)
    check(result, 'empty process table')
    return bytes(result)

def main():
    check(len(sys.argv) == 3 and all(value.isascii() and value.isdecimal()
                                   and len(value) <= 10 for value in sys.argv[1:]),
          'private process inspector arguments')
    controller, group = map(int, sys.argv[1:])
    check(os.getppid() == controller and os.getpgrp() == group, 'inspector caller changed')
    rows = process_rows(bind_library(), os.getpid(), controller, group, os.getuid())
    sys.stdout.buffer.write(encode_rows(rows))

if __name__ == '__main__':
    main()
"""

import argparse
import errno
import hashlib
import json
import math
import os
from pathlib import Path
import re
import select
import signal
import socket
import stat
import subprocess
import sys
import threading
import time

PROFILE_SHA256 = "af0c4f23c08fd2c1d17126c0b7c2d68720ecd275840945de444eae7d46bcc866"
ROLES = ("s3", "public_ac", "internal_ac", "public_b", "internal_b")
ZERO_COUNTS = (
    "occupied", "activating", "restoring", "activation_waiting", "capacity_waiting",
    "quiescing", "evicting", "releasing", "adopting",
)
CUT_EVENTS = frozenset({
    "connection_drain_forced", "shutdown_frame_flush_cut", "shutdown_pre_drain_forced",
    "shutdown_handoff_not_started", "shutdown_handoff_stalled", "drain_token_release_deadline",
    "clean_reload_abandoned", "drain_token_bypassed", "drain_token_renew_lost",
    "drain_token_release_ambiguous", "drain_token_release_superseded", "core_actor_exit",
    "node_lease_watchdog_fence", "node_lease_record_missing_fence", "node_lease_record_mismatch_fence",
})
CUT_LINES = frozenset({
    "celld durability quiesce exceeded the process deadline",
    "celld local durability shutdown exceeded the process deadline",
})
# Fixed tracing text fields; fixture identifiers contain no quotes/whitespace.
FIELD = re.compile(r'(?:^|\s)([a-zA-Z_][a-zA-Z_0-9]*)=("[^"\r\n]*"|[^\s]+)')
HEX64 = re.compile(r"[0-9a-f]{64}\Z")


class Refused(RuntimeError):
    """An incomplete or contradictory observation cannot qualify."""


class InspectionPhaseExpired(Refused):
    """The caller's earlier phase ended; this is not an inspection result."""


def require(condition, message):
    if not condition:
        raise Refused(message)


def integer(value, label, minimum=0):
    require(type(value) is int and minimum <= value <= (1 << 64) - 1, label)
    return value


def text_id(value, label):
    require(isinstance(value, str) and 0 < len(value) <= 512
            and all(33 <= ord(c) <= 126 and c not in '"\\' for c in value), label)
    return value


def strict_json(raw, maximum=65536):
    require(isinstance(raw, bytes) and len(raw) <= maximum, "JSON bound")

    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON field")
            result[key] = value
        return result

    def bad_constant(_):
        raise Refused("non-finite JSON number")

    def finite_float(value):
        result = float(value)
        require(math.isfinite(result), "non-finite JSON number")
        return result

    try:
        return json.loads(raw, object_pairs_hook=pairs, parse_constant=bad_constant, parse_float=finite_float)
    except (UnicodeError, ValueError, RecursionError) as error:
        raise Refused("invalid bounded JSON") from error


def identity(raw):
    return {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def regular_bytes(path, maximum):
    path = Path(path)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, "regular file bound")
    with path.open("rb") as stream:
        raw = stream.read(maximum + 1)
    require(len(raw) <= maximum and len(raw) == info.st_size, "file changed during read")
    return raw


def file_hash(path, check=lambda: None):
    path = Path(path)
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode), "hash input is not a regular file")
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        while part := stream.read(1024 * 1024):
            check()
            size += len(part)
            digest.update(part)
    after = path.lstat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
            and size == before.st_size, "hash input changed")
    return {"bytes": size, "sha256": digest.hexdigest()}


class Clock:
    """One original deadline; phase caps are intersections, never renewals."""

    def __init__(self, deadline_ns, monotonic=time.monotonic_ns, wall=time.time_ns):
        self.monotonic = monotonic
        self.wall = wall
        self.anchor_mono = self.last_mono = monotonic()
        self.anchor_wall = wall()
        integer(deadline_ns, "original deadline")
        require(self.anchor_mono < deadline_ns <= self.anchor_mono + 600_000_000_000,
                "original 600-second deadline")
        self.deadline = deadline_ns
        self.maximum_drift_ns = 0
        self.cancelled = False

    def sample(self):
        mono, wall = self.monotonic(), self.wall()
        require(type(mono) is int and type(wall) is int and mono >= self.last_mono,
                "monotonic clock moved backwards")
        drift = (wall - self.anchor_wall) - (mono - self.anchor_mono)
        self.maximum_drift_ns = max(self.maximum_drift_ns, abs(drift))
        self.last_mono = mono
        require(abs(drift) <= 1_000_000_000, "wall/monotonic clock anomaly")
        return mono, wall

    def check(self, deadline=None):
        require(not self.cancelled, "controller signal cancellation")
        now, _ = self.sample()
        require(now < min(self.deadline, deadline or self.deadline), "original/phase deadline expired")
        return now

    def phase(self, seconds, parent=None):
        return min(self.deadline, parent or self.deadline, self.check() + seconds * 1_000_000_000)

    def remaining(self, deadline):
        return (min(deadline, self.deadline) - self.check(deadline)) / 1_000_000_000

    def now_ms(self):
        self.check()
        return self.sample()[1] // 1_000_000


def candidate(lease, node, address, now_ms, generation=None):
    require(isinstance(lease, dict), "node lease object")
    require(lease.get("node") == node and lease.get("addr") == address, "node lease identity/address")
    value = text_id(lease.get("ownership_index_generation"), "node process generation")
    require(lease.get("probe_public_key") == value, "node probe/generation mismatch")
    require(generation is None or generation == value, "node process generation changed")
    require(integer(lease.get("expires_ms"), "node lease expiry") > now_ms, "expired candidate")
    require(type(lease.get("peer_protocol")) is int and lease["peer_protocol"] == 5,
            "candidate protocol")
    require(lease.get("paced_handoff") is True, "candidate does not accept paced handoff")
    load = lease.get("load")
    require(isinstance(load, dict), "candidate load")
    integer(load.get("sampled_ms"), "candidate sample", 1)
    require(load.get("pressured") is False, "candidate pressure")
    for key in ("memory_headroom", "draining"):
        require(key in load and (load[key] is None or type(load[key]) is bool),
                "candidate optional load field")
    require(load["memory_headroom"] is not False and load["draining"] is not True,
            "candidate unavailable")
    return value


def owner(value, node, epoch=None):
    require(isinstance(value, dict) and set(value) == {"node", "epoch"}, "owner object")
    require(value["node"] == node, "unexpected cell owner")
    actual = integer(value["epoch"], "owner epoch", 1)
    require(epoch is None or epoch == actual, "unexpected owner epoch")
    return actual


def state_census(state, kind, cell=None):
    require(isinstance(state, dict), "state object")
    count = 0 if kind == "empty" else 1
    require(type(state.get("owned_cells")) is int and state["owned_cells"] == count,
            "unexpected owned-cell count")
    residents = state.get("residents")
    phases = state.get("phases")
    require(isinstance(residents, list) and isinstance(phases, dict), "state census shape")
    require(all(isinstance(k, str) and type(v) is int and v >= 0 for k, v in phases.items()),
            "state phase counters")
    require(sum(phases.values()) == count, "extra/missing cell phase")
    if kind in ("empty", "dormant"):
        require(residents == [], "receiver already activated")
        for key in ZERO_COUNTS:
            require(type(state.get(key)) is int and state[key] == 0, "receiver not dormant/empty")
        if kind == "dormant":
            require(phases.get("dormant") == 1, "missing dormant cell")
        return None
    require(kind == "resident" and len(residents) == 1 and phases.get("resident") == 1,
            "expected one resident")
    actual = text_id(residents[0], "resident cell")
    require("/" not in actual and actual not in (".", ".."), "unsafe cell diagnostic key")
    require(cell is None or actual == cell, "unexpected resident cell")
    require(type(state.get("occupied")) is int and state["occupied"] == 1, "not occupied")
    for key in ZERO_COUNTS[1:]:
        require(type(state.get(key)) is int and state[key] == 0, "resident transition incomplete")
    return actual


def old_node_ineligible(last, current, missing, now_ms, retired, current_owner, live_node, live_epoch):
    require(retired is True, "old node has not retired")
    owner(current_owner, live_node, live_epoch)
    expiry = integer(last.get("expires_ms"), "last old-node expiry")
    require(expiry <= now_ms, "last old-node lease has not expired")
    if missing:
        require(missing == {"status": 404, "code": "NoSuchKey"} and current is None,
                "missing node is not an exact NoSuchKey observation")
        require(last.get("log") is None, "missing old node had folded log")
        return "observed-no-log-key-absence"
    require(isinstance(current, dict), "old node observation unavailable")
    for key in ("node", "addr", "ownership_index_generation", "probe_public_key"):
        require(current.get(key) == last.get(key), "old-node identity changed")
    current_expiry = integer(current.get("expires_ms"), "old-node expiry")
    require(current_expiry <= now_ms, "old-node lease still eligible")
    if current_expiry == 0:
        log = current.get("log")
        if log is None:
            require(last.get("log") is None, "no-log tombstone lost folded-log history")
            # Canonical GC CASes this dead record before deletion. Observing it
            # proves ineligibility only, not deletion, GC completion or PUT drain.
            return "observed-no-log-expired-tombstone"
        require(isinstance(log, dict) and log.get("state") == "sealed", "unproved sealed tombstone")
        integer(log.get("epoch"), "sealed log epoch", 1)
        integer(log.get("tiered"), "sealed log coverage")
        require(isinstance(log.get("ensemble"), list)
                and all(isinstance(v, str) for v in log["ensemble"])
                and type(log.get("active")) is bool, "sealed log proof fields")
        return "observed-sealed-expired-node"
    return "observed-expired-node"


def clean_marker(raw, node, generation):
    value = strict_json(raw)
    require(value == {"node": node, "generation": generation}, "wrong clean-reload marker")
    return value


def parse_logs(raw):
    require(len(raw) <= 33_554_432 and (not raw or raw.endswith(b"\n")), "truncated/over-limit log")
    try:
        lines = raw.decode("utf-8", errors="strict").splitlines()
    except UnicodeError as error:
        raise Refused("unparseable node log") from error
    events = []
    for number, line in enumerate(lines):
        require("\x1b" not in line and "\x00" not in line and len(line) <= 65536, "invalid log line")
        require("SELF-FENCE" not in line and not line.startswith("celld preserve drain reached its ")
                and line not in CUT_LINES, "canonical fence/deadline cut")
        fields = {}
        for match in FIELD.finditer(line):
            key, value = match.groups()
            require(key not in fields, "ambiguous log field")
            fields[key] = value[1:-1] if value.startswith('"') else value
        event = fields.get("event")
        require(event not in CUT_EVENTS, "canonical shutdown cut: " + str(event))
        if event:
            events.append({"line": number, "fields": fields})
    return events


def handoff_witness(raw, cell, successor, prior_epoch):
    events = parse_logs(raw)
    selected = {}
    for event in events:
        kind, fields = event["fields"]["event"], event["fields"]
        if kind in ("drain_token_acquired", "drain_token_released", "cell_handoff_accepted", "cell_handoff_timing"):
            require(kind not in selected, "ambiguous repeated handoff witness")
            selected[kind] = event
    require(len(selected) == 4, "missing handoff witness")
    acquired, released = selected["drain_token_acquired"], selected["drain_token_released"]
    accepted, timing = selected["cell_handoff_accepted"], selected["cell_handoff_timing"]
    require(acquired["line"] < accepted["line"] < timing["line"] < released["line"],
            "handoff witness order")
    fields = accepted["fields"]
    require(fields.get("cell") == cell and fields.get("successor") == successor,
            "handoff cell/successor mismatch")
    require(fields.get("released_epoch") == str(prior_epoch), "handoff released epoch")
    raw_epoch = fields.get("successor_epoch", "")
    require(re.fullmatch(r"[1-9][0-9]{0,19}", raw_epoch) is not None, "handoff successor epoch")
    epoch = integer(int(raw_epoch), "handoff epoch", prior_epoch + 1)
    require(timing["fields"].get("cell") == cell, "processed handoff cell mismatch")
    for key in ("quiesce_us", "durability_us", "stop_us", "release_us", "adopt_us", "total_us"):
        value = timing["fields"].get(key, "")
        require(re.fullmatch(r"[0-9]{1,20}", value) is not None, "missing handoff timing")
        integer(int(value), "handoff timing overflow")
    return {"epoch": epoch, "events": selected}


def preserve_witness(raw, marker_raw, node, generation):
    events = [row for row in parse_logs(raw) if row["fields"]["event"] == "clean_reload_prepared"]
    require(len(events) == 1, "missing/ambiguous Preserve witness")
    return {"event": events[0], "marker": clean_marker(marker_raw, node, generation),
            "marker_identity": identity(marker_raw)}


def successful_exit(returncode, elapsed_ns):
    require(type(returncode) is int and returncode == 0, "nonzero node exit")
    require(type(elapsed_ns) is int and 0 <= elapsed_ns < 35_000_000_000,
            "node exit did not satisfy strict 35-second bound")


def decode_http(wire, body_limit=65536):
    require(len(wire) <= 131072, "HTTP wire bound")
    separator = wire.find(b"\r\n\r\n")
    require(0 <= separator <= 16384, "HTTP header bound/framing")
    lines = wire[:separator].split(b"\r\n")
    match = re.fullmatch(rb"HTTP/1\.[01] ([1-5][0-9]{2}) [\x20-\x7e]*", lines[0])
    require(match is not None, "HTTP status line")
    status_code = int(match[1])
    headers = {}
    for line in lines[1:]:
        pair = line.split(b":", 1)
        require(len(pair) == 2 and re.fullmatch(rb"[A-Za-z0-9!#$%&'*+.^_`|~-]+", pair[0]),
                "HTTP header syntax")
        key, value = pair[0].lower(), pair[1].strip()
        require(key not in headers and all(32 <= c < 127 or c == 9 for c in value),
                "duplicate/invalid HTTP header")
        headers[key] = value
    body = wire[separator + 4:]
    if b"transfer-encoding" in headers:
        require(headers[b"transfer-encoding"].lower() == b"chunked"
                and b"content-length" not in headers, "ambiguous HTTP framing")
        decoded, offset, chunks = bytearray(), 0, 0
        while True:
            end = body.find(b"\r\n", offset)
            require(end >= 0 and end - offset <= 16, "invalid chunk size")
            size_text = body[offset:end]
            require(re.fullmatch(rb"[0-9a-fA-F]{1,16}", size_text), "chunk extension/syntax")
            size, offset = int(size_text, 16), end + 2
            chunks += 1
            require(chunks <= 4096 and len(decoded) + size <= body_limit, "HTTP chunk bound")
            require(offset + size + 2 <= len(body) and body[offset + size:offset + size + 2] == b"\r\n",
                    "truncated chunk")
            if size == 0:
                require(offset + 2 == len(body), "unexpected HTTP trailer/data")
                body = bytes(decoded)
                break
            decoded.extend(body[offset:offset + size])
            offset += size + 2
    elif b"content-length" in headers:
        value = headers[b"content-length"]
        require(re.fullmatch(rb"[0-9]{1,10}", value) and int(value) == len(body), "HTTP content length")
    require(len(body) <= body_limit, "HTTP body bound")
    return status_code, body


def http_exchange(port, method, path, body, deadline, clock, tick=lambda: None):
    require(type(port) is int and 1024 <= port <= 65535, "HTTP port")
    require(method in ("GET", "POST") and path in (
        "/", "/read", "/write", "/state", "/.well-known/celld/health",
        "/shutdown", "/shutdown?handoff=preserve"), "HTTP closed method/path")
    require(isinstance(body, bytes) and len(body) <= 36, "HTTP request body bound")
    request = (f"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
               f"Connection: close\r\nContent-Length: {len(body)}\r\n\r\n").encode() + body
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
        client.setblocking(False)
        result = client.connect_ex(("127.0.0.1", port))
        if result not in (0, errno.EINPROGRESS, errno.EWOULDBLOCK):
            raise OSError(result, os.strerror(result))
        sent, received, connected = 0, bytearray(), result == 0
        while True:
            tick()
            remaining = clock.remaining(deadline)
            readers = [client] if connected and sent == len(request) else []
            writers = [client] if not connected or sent < len(request) else []
            ready_r, ready_w, _ = select.select(readers, writers, [], min(0.05, remaining))
            if ready_w:
                error = client.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)
                if error:
                    raise OSError(error, os.strerror(error))
                connected = True
                try:
                    sent += client.send(request[sent:])
                except BlockingIOError:
                    pass
            if ready_r:
                try:
                    part = client.recv(min(8192, 131073 - len(received)))
                except BlockingIOError:
                    continue
                if not part:
                    clock.check(deadline)
                    return decode_http(bytes(received))
                received.extend(part)
                require(len(received) <= 131072, "HTTP wire bound")


def parse_process_table(raw):
    require(0 < len(raw) <= 65536, "process table bound")
    result = {}
    for line in raw.decode("ascii", errors="strict").splitlines():
        parts = line.split()
        require(len(parts) == 4 and all(value.isdigit() and len(value) <= 10 for value in parts),
                "process table syntax")
        pid, parent, group, status = map(int, parts)
        require(0 < pid <= 2147483647 and pid not in result and len(result) < 4096,
                "duplicate or out-of-range process identity")
        require(parent <= 2147483647 and group <= 2147483647 and 1 <= status <= 5,
                "process parent/group/status")
        result[pid] = (parent, group, status)
    return result


def descendants(table, roots):
    known = set(roots)
    while True:
        extra = {pid for pid, (parent, _, _) in table.items() if parent in known} - known
        if not extra:
            return known
        known.update(extra)


def inspection_remaining_ns(deadline, helper_deadline):
    now = time.monotonic_ns()
    if now >= helper_deadline:
        raise Refused("process inspection timeout")
    if now >= deadline:
        raise InspectionPhaseExpired("process inspection phase expired")
    return min(deadline, helper_deadline) - now


def process_inspector_command(profile, controller, group):
    launch = profile["launch"]
    inspection = launch["process_inspection"]
    require(inspection["source"] == identity(PROCESS_INSPECTOR.encode("utf-8")),
            "process inspector source changed")
    require(inspection["backend"] == "darwin-libproc-shortbsdinfo"
            and inspection["bootstrap"] == "closed-stdlib-literal-v1"
            and inspection["library"] == "/usr/lib/libproc.dylib"
            and launch["flags"] == ["-I", "-S", "-B"]
            and len(launch["sys_path"]) == 3, "process inspector launch contract")
    require(all(type(value) is int and 0 < value <= 2147483647 for value in (controller, group)),
            "process inspector caller identity")
    # Native calls stay inside this killable child. No site, current directory,
    # venv package root, shell, library search or public mode enters its startup.
    bootstrap = "import sys; sys.path[:] = " + repr(launch["sys_path"][:2]) + ";\n" + PROCESS_INSPECTOR
    return [launch["python"], *launch["flags"], "-c", bootstrap, str(controller), str(group)]


def check_process_witnesses(table, helper_pid, controller, parent, group):
    require(helper_pid != controller and helper_pid in table and controller in table,
            "missing inspection helper/controller witness")
    require(table[helper_pid][:2] == (controller, group)
            and table[controller][:2] == (parent, group), "inspection helper/controller mismatch")


class Child:
    def __init__(self, label, process, output, started_ns, argv, directory=None, node=None, ports=None):
        self.label, self.process, self.output = label, process, output
        self.started_ns, self.argv = started_ns, argv
        self.directory, self.node, self.ports = directory, node, ports
        self.descendants = {process.pid}
        self.retired = False
        self.generation = None
        self.expected_exit = False
        self.startup_deadline = None
        self.exit_ns = None

    def row(self):
        return {"label": self.label, "pid": self.process.pid, "argv": self.argv,
                "node": self.node, "directory": str(self.directory) if self.directory else None,
                "ports": self.ports, "generation": self.generation, "started_ns": self.started_ns,
                "exit_ns": self.exit_ns, "returncode": self.process.returncode,
                "attributed_pids": sorted(self.descendants), "retired": self.retired,
                "log": str(self.output)}


class Children:
    """Actual handles stay owned until reaped and observed absent, including failures."""

    def __init__(self, clock, directory, environment, profile, isolated=True):
        self.clock, self.directory, self.environment = clock, directory, environment
        self.profile = profile
        self.group = os.getpgrp()
        require(not isolated or self.group == os.getpid(), "controller must lead its owned group")
        self.isolated = isolated
        self.items = []
        self.last_scan = 0
        self.table = {}
        self.cleaning = False

    def spawn(self, label, argv, environment=None, directory=None, node=None, ports=None):
        require(len(self.items) < 12 and re.fullmatch(r"[a-z0-9-]{1,24}", label), "child roster bound")
        if node:
            require(sum(c.node is not None and not c.retired for c in self.items) < 2, "two-node bound")
            for old in self.items:
                require(not old.node or old.retired or not set(old.ports.values()) & set(ports.values()),
                        "node port reuse before retirement")
        output = self.directory / (label + ".log")
        started = self.clock.check()
        # Main's handlers only set Clock.cancelled. No signal mask leaks into
        # children, and cancellation is observed only after retaining this handle.
        with output.open("xb") as stream:
            process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=stream,
                                       stderr=subprocess.STDOUT, cwd=self.directory,
                                       env=environment or self.environment, start_new_session=False)
        child = Child(label, process, output, started, list(argv), directory, node, ports)
        self.items.append(child)
        self.clock.check()
        return child

    def process_table(self, deadline):
        # A fixed read-only helper, also owned/reaped. Its output is bounded while read.
        helper_deadline = time.monotonic_ns() + 3_000_000_000
        inspection_remaining_ns(deadline, helper_deadline)
        controller, parent = os.getpid(), os.getppid()
        require(os.getpgrp() == self.group, "controller process group changed")
        argv = process_inspector_command(self.profile, controller, self.group)
        inspection_remaining_ns(deadline, helper_deadline)
        helper = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=self.environment)
        output, error = bytearray(), bytearray()
        try:
            streams = {helper.stdout: output, helper.stderr: error}
            for stream in streams:
                os.set_blocking(stream.fileno(), False)
            while streams:
                remaining = inspection_remaining_ns(deadline, helper_deadline)
                ready, _, _ = select.select(list(streams), [], [], min(0.02, remaining / 1e9))
                for stream in ready:
                    part = os.read(stream.fileno(), 4096)
                    if not part:
                        del streams[stream]
                    else:
                        streams[stream].extend(part)
                        require(len(streams[stream]) <= 65536, "process inspection output bound")
            try:
                helper.wait(timeout=inspection_remaining_ns(deadline, helper_deadline) / 1e9)
            except subprocess.TimeoutExpired as wait_error:
                # Only the same caller-boundary condition can be classified as
                # phase expiry. A helper-cap or unexpected wait failure is fatal.
                inspection_remaining_ns(deadline, helper_deadline)
                raise Refused("process inspection wait failed") from wait_error
            require(helper.returncode == 0 and not error, "process inspection failed")
            table = parse_process_table(bytes(output))
            check_process_witnesses(table, helper.pid, controller, parent, self.group)
            inspection_remaining_ns(deadline, helper_deadline)
            # Native self/caller witnesses are mandatory. Only this now-reaped helper
            # is removed; every other unattributed same-group PID still refuses.
            del table[helper.pid]
            return table
        finally:
            # A phase expiry is usable only after this helper is physically
            # reaped. Disposition failures replace it and remain fatal.
            try:
                if helper.poll() is None:
                    helper.kill()
                helper.wait(timeout=3)
            finally:
                try:
                    helper.stdout.close()
                finally:
                    helper.stderr.close()

    def scan(self, deadline, force=False):
        now = time.monotonic_ns()
        if not force and now - self.last_scan < 250_000_000:
            return
        self.table = self.process_table(deadline)
        self.last_scan = time.monotonic_ns()
        attributed = {os.getpid()}
        for child in self.items:
            child.descendants = descendants(self.table, child.descendants)
            attributed.update(child.descendants)
            for pid in child.descendants:
                require(pid not in self.table or self.table[pid][1] == self.group,
                        "child escaped owned group; cleanup remains uncertain")
            if child.retired:
                require(not child.descendants & self.table.keys(), "retired process identity reappeared")
        if self.isolated:
            group_members = {pid for pid, (_, group, _) in self.table.items() if group == self.group}
            require(not group_members - attributed, "unattributed process in owned group")

    def tick(self, deadline=None):
        deadline = deadline or self.clock.deadline
        self.clock.check(deadline)
        for child in self.items:
            require(child.output.lstat().st_size <= 33_554_432, "child log bound")
            status = child.process.poll()
            if status is not None and child.exit_ns is None:
                child.exit_ns = self.clock.sample()[0]
            require(self.cleaning or status is None or child.expected_exit or child.retired,
                    "unexpected child exit: " + child.label)
        self.scan(deadline)

    def wait(self, child, deadline):
        child.expected_exit = True
        while child.process.poll() is None:
            self.tick(deadline)
            time.sleep(min(0.02, self.clock.remaining(deadline)))
        child.process.wait()
        if child.exit_ns is None:
            child.exit_ns = self.clock.sample()[0]
        self.scan(deadline, force=True)
        require(not child.descendants & self.table.keys(), "child descendants remain after parent exit")
        child.retired = True
        return child.process.returncode

    def cleanup(self):
        # Two three-second grace phases, plus bounded helper disposition, parent
        # waits and a final scan. This grants no new operation deadline; the outer
        # owner retains responsibility for proving whole-group disappearance.
        self.cleaning = True
        errors = []
        for sig, seconds in ((signal.SIGTERM, 3), (signal.SIGKILL, 3)):
            end = time.monotonic_ns() + seconds * 1_000_000_000
            phase_expired = False
            fresh_scan = False
            try:
                self.scan(end, force=True)
                fresh_scan = True
            except InspectionPhaseExpired as error:
                if sig != signal.SIGTERM:
                    errors.append(type(error).__name__ + ": " + str(error)[:256])
                # No signal or completion decision may use the previous table.
                # TERM expiry always advances to a fresh KILL-stage inspection.
                continue
            except BaseException as error:
                errors.append(type(error).__name__ + ": " + str(error)[:256])
            for child in reversed(self.items):
                if child.retired:
                    continue  # Historical PIDs are not authority to signal a process.
                try:
                    if child.process.poll() is None:
                        child.process.send_signal(sig)
                except OSError as error:
                    errors.append(type(error).__name__)
                for pid in child.descendants - {child.process.pid}:
                    if not fresh_scan or pid not in self.table or self.table[pid][1] != self.group:
                        continue  # Unknown/escaped work is left to the outer owner.
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
                    except OSError as error:
                        errors.append(type(error).__name__)
            while time.monotonic_ns() < end:
                for child in self.items:
                    child.process.poll()
                try:
                    self.scan(end, force=True)
                    fresh_scan = True
                    if not any(c.descendants & self.table.keys() for c in self.items):
                        break
                except InspectionPhaseExpired as error:
                    phase_expired = True
                    fresh_scan = False
                    if sig != signal.SIGTERM:
                        errors.append(type(error).__name__ + ": " + str(error)[:256])
                    break
                except BaseException as error:
                    fresh_scan = False
                    errors.append(type(error).__name__ + ": " + str(error)[:256])
                    break
                time.sleep(0.02)
            if fresh_scan and not phase_expired and not any(c.descendants & self.table.keys() for c in self.items):
                break
        reaped = set()
        for child in self.items:
            try:
                child.process.wait(timeout=0.1)
                reaped.add(child)
                if child.exit_ns is None:
                    child.exit_ns = time.monotonic_ns()
            except BaseException as error:
                errors.append(child.label + ": " + type(error).__name__)
        try:
            # After direct reaping, validate known descendants' absence and, in
            # the isolated controller, reject any unattributed same-group work.
            # PPID attribution alone cannot recover every already-orphaned fork.
            self.scan(time.monotonic_ns() + 3_000_000_000, force=True)
            for child in reaped:
                if not child.descendants & self.table.keys():
                    child.retired = True
            require(not any(c.descendants & self.table.keys() for c in self.items), "remaining attributed process")
        except BaseException as error:
            errors.append(type(error).__name__ + ": final child snapshot incomplete")
        return {"completed": not errors and all(c.retired for c in self.items),
                "children": [c.row() for c in self.items], "error": errors[:16] or None}


class Evidence:
    def __init__(self, directory, clock):
        self.directory, self.clock = directory, clock
        self.serial = 0
        self.last_scan = 0
        self.raw = directory / "observations"
        self.raw.mkdir()

    def check(self, force=False, deadline=None):
        self.clock.check(deadline)
        now = time.monotonic_ns()
        if not force and now - self.last_scan < 500_000_000:
            return
        total, entries = 0, 0
        for base, dirs, files in os.walk(self.directory, followlinks=False):
            for name in dirs + files:
                self.clock.check(deadline)
                path = Path(base) / name
                try:
                    info = path.lstat()
                except FileNotFoundError:
                    continue  # A live node can remove its own transient file.
                require(stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode), "evidence symlink/special file")
                entries += 1
                total += info.st_size if stat.S_ISREG(info.st_mode) else 0
                require(entries <= 25000 and total <= 268_435_456, "evidence payload bound")
        self.last_scan = now

    def save(self, label, raw):
        require(re.fullmatch(r"[a-z0-9-]{1,48}", label) and len(raw) <= 65536, "observation bound")
        self.serial += 1
        require(self.serial <= 4096, "observation-count bound")
        path = self.raw / f"{self.serial:04d}-{label}.bin"
        with path.open("xb") as stream:
            stream.write(raw)
        self.check()
        return {"path": str(path), **identity(raw)}


class StorageTask:
    def __init__(self, function):
        self.value = None
        self.error = None

        def invoke():
            try:
                self.value = function()
            except BaseException as error:
                self.error = error

        self.thread = threading.Thread(target=invoke, name="a3a-storage", daemon=False)


class Storage:
    def __init__(self, endpoint, clock, tick):
        # No fixture module import occurs until main has validated the isolated
        # interpreter, exact profile, environment and literal source identities.
        import boto3
        from botocore.config import Config
        from botocore.exceptions import ClientError

        self.client_error = ClientError
        self.clock, self.tick = clock, tick
        self.pending = []
        self.finished = 0
        self.raw_response = threading.local()
        self.client = boto3.client(
            "s3", endpoint_url=endpoint, region_name="us-east-1",
            aws_access_key_id="foks-local-dummy", aws_secret_access_key="foks-local-dummy-secret",
            config=Config(signature_version="s3v4", connect_timeout=1, read_timeout=3,
                          retries={"total_max_attempts": 1}, max_pool_connections=2,
                          s3={"addressing_style": "path"}),
        )
        self.client.meta.events.register("after-call.s3.*", self.capture_response)

    def capture_response(self, http_response, model, **_kwargs):
        # Botocore emits this before raising ClientError. Keep the actual error
        # bytes, including the exact NoSuchKey observation, per request thread.
        # Successful GetObject is streaming and is bounded/read/closed below.
        if http_response.status_code >= 300 or model.name != "GetObject":
            raw = http_response.content
            require(isinstance(raw, bytes) and len(raw) <= 65536, "storage diagnostic body bound")
            self.raw_response.body = raw

    def start(self, function):
        require(len(self.pending) < 2, "storage task concurrency bound")
        task = StorageTask(function)
        self.pending.append(task)
        try:
            task.thread.start()
        except BaseException:
            self.pending.remove(task)
            raise
        return task

    def finish(self, task, deadline):
        while task.thread.is_alive():
            self.tick(deadline)
            task.thread.join(timeout=min(0.02, self.clock.remaining(deadline)))
        task.thread.join()
        self.pending.remove(task)
        self.finished += 1
        self.clock.check(deadline)
        if task.error is not None:
            raise task.error
        return task.value

    def call(self, function, deadline):
        return self.finish(self.start(function), deadline)

    def request(self, operation, **arguments):
        self.raw_response.body = None
        try:
            value = getattr(self.client, operation)(**arguments)
        except self.client_error as error:
            metadata = error.response.get("ResponseMetadata", {})
            raw = self.raw_response.body
            require(isinstance(raw, bytes), "missing raw storage error response")
            return {"status": metadata.get("HTTPStatusCode"),
                    "code": error.response.get("Error", {}).get("Code"), "body": raw, "etag": None}
        body = value.get("Body")
        try:
            raw = body.read(65537) if body is not None else self.raw_response.body
            require(isinstance(raw, bytes) and len(raw) <= 65536, "storage response bound")
        finally:
            if body is not None:
                body.close()
        return {"status": value.get("ResponseMetadata", {}).get("HTTPStatusCode"),
                "code": None, "body": raw, "etag": value.get("ETag"),
                "content_range": value.get("ContentRange")}

    def get(self, bucket, key, deadline):
        require(len(key) <= 1024 and key.startswith(("nodes/", "cells/", "qualification/")), "closed storage key")
        return self.call(lambda: self.request("get_object", Bucket=bucket, Key=key), deadline)

    def cleanup(self):
        # Stop node/Moto processes before this: blocked socket reads then either
        # finish or remain charged to the outer process owner. Never forget a task.
        self.client.close()
        end = time.monotonic_ns() + 3_000_000_000
        for task in self.pending:
            task.thread.join(timeout=max(0, (end - time.monotonic_ns()) / 1e9))
        joined = all(not task.thread.is_alive() for task in self.pending)
        if joined:
            for task in self.pending:
                task.thread.join()
            self.finished += len(self.pending)
            self.pending.clear()
        return joined


def startup_observation(profile, script):
    flags = {key: getattr(sys.flags, key) for key in
             ("isolated", "ignore_environment", "no_site", "dont_write_bytecode")}
    require(all(value == 1 for value in flags.values()), "interpreter isolation flags")
    require(sys.path == profile["launch"]["sys_path"], "interpreter search path")
    executable = Path(profile["launch"]["python"]).resolve(strict=True)
    require(Path(sys.executable).resolve(strict=True) == executable, "interpreter executable")
    require(sys.version_info[:3] == (3, 14, 3), "interpreter version")
    allowed_roots = [Path(value).resolve(strict=True) for value in sys.path]
    module_rows = []
    for name, module in sorted(sys.modules.items()):
        require(len(module_rows) < 4096, "module observation bound")
        path = getattr(module, "__file__", None)
        if path is None:
            spec = getattr(module, "__spec__", None)
            origin = getattr(spec, "origin", None)
            require(origin in (None, "built-in", "frozen"), "unknown module origin")
            continue
        resolved = Path(path).resolve(strict=True)
        require(resolved == script or any(resolved.is_relative_to(root) for root in allowed_roots),
                "module escaped pinned search roots: " + name[:128])
        module_rows.append({"module": name, "path": str(resolved)})
    return {"flags": flags, "sys_path": list(sys.path), "executable": str(executable),
            "prefix": sys.prefix, "base_prefix": sys.base_prefix,
            "python_version": ".".join(map(str, sys.version_info[:3])), "modules": module_rows}


def runtime_environment(profile, directory):
    result = dict(profile["launch"]["environment"])
    result["TMPDIR"] = str(directory / "tmp")
    empty = str(directory / "empty-native-config")
    result.update(OPENSSL_CONF_INCLUDE=empty, OPENSSL_MODULES=empty, SSL_CERT_DIR=empty,
                  SSL_CERT_FILE=profile["launch"]["certifi"])
    return result


def check_environment(profile, directory):
    expected = runtime_environment(profile, directory)
    require(dict(os.environ) == expected, "controller environment differs from closed whitelist")
    for name in ("tmp", "empty-native-config"):
        path = directory / name
        require(stat.S_ISDIR(path.lstat().st_mode) and path.resolve() == path, "runtime directory identity")
    require(not any((directory / "empty-native-config").iterdir()), "native config directory is not empty")
    return expected


def fixture_versions(profile):
    import boto3
    import botocore
    import moto

    result = {"python": ".".join(map(str, sys.version_info[:3])), "boto3": boto3.__version__,
              "botocore": botocore.__version__, "moto": moto.__version__}
    require(result == profile["fixture"]["versions"], "fixture module versions")
    return result


class Run:
    def __init__(self, arguments, profile, clock, environment):
        self.arguments, self.profile, self.clock = arguments, profile, clock
        self.directory = Path(arguments.directory)
        self.binary = Path(arguments.binary)
        self.environment = environment
        self.ports = {role: getattr(arguments, role + "_port") for role in ROLES}
        self.run_id = os.urandom(16).hex()
        self.bucket = "foks-a3a-" + self.run_id
        self.endpoint = "http://127.0.0.1:" + str(self.ports["s3"])
        self.children = Children(clock, self.directory, environment, profile)
        self.evidence = Evidence(self.directory, clock)
        self.storage = None
        self.active_responses = 0
        self.steps = []
        self.nodes = []
        self.cell = None

    def tick(self, deadline=None):
        self.children.tick(deadline)
        self.evidence.check(deadline=deadline)

    def binary_check(self):
        value = file_hash(self.binary, self.clock.check)
        require(value["sha256"] == self.arguments.binary_sha256, "celld executable changed")
        return value

    def step(self, name, **data):
        require(len(self.steps) < 64, "sequence step bound")
        mono, wall = self.clock.sample()
        self.clock.check()
        self.steps.append({"name": name, "monotonic_ns": mono, "wall_ns": wall, **data})

    def response(self, label, port, method, path, deadline, body=b""):
        deadline = min(deadline, self.clock.phase(15))
        started = self.clock.check(deadline)
        self.active_responses += 1
        try:
            status, raw = http_exchange(port, method, path, body, deadline, self.clock,
                                        lambda: self.tick(deadline))
        finally:
            self.active_responses -= 1
        evidence = self.evidence.save(label, raw)
        return status, raw, {"status": status, "response": evidence,
                             "started_ns": started, "finished_ns": self.clock.check(deadline)}

    def poll(self, function, deadline):
        while True:
            self.tick(deadline)
            self.clock.check(deadline)
            result = function()
            if result is not None:
                return result
            time.sleep(min(0.1, self.clock.remaining(deadline)))

    def cli(self, label, arguments, expected_status=0, text=None):
        self.binary_check()
        child = self.children.spawn(label, [str(self.binary), *arguments],
                                    self.environment | {"CELLD_TOKIO_THREADS": "2"})
        child.expected_exit = True
        deadline = self.clock.phase(120 if label == "deploy" else 15)
        status = self.children.wait(child, deadline)
        self.binary_check()
        raw = regular_bytes(child.output, 33_554_432)
        require(status == expected_status if expected_status is not None else status != 0, "CLI status")
        if text is not None:
            require(text in raw, "missing CLI observation")
        self.step(label, command=child.row(), output=identity(raw))

    def moto(self):
        launch = self.profile["launch"]
        bootstrap = ("import sys;sys.path=" + repr(launch["sys_path"]) + ";"
                     "import runpy;sys.argv=['moto.server']+sys.argv[1:];"
                     "runpy.run_module('moto.server',run_name='__main__')")
        child = self.children.spawn("moto", [launch["python"], "-I", "-S", "-B", "-c", bootstrap,
                                             "-H", "127.0.0.1", "-p", str(self.ports["s3"])])
        deadline = self.clock.phase(120)

        def ready():
            try:
                status, raw, observation = self.response("moto-health", self.ports["s3"], "GET", "/", deadline)
            except ConnectionRefusedError:
                return None
            require(status == 200 and b"ListAllMyBucketsResult" in raw, "unexpected S3 listener")
            require(("Running on " + self.endpoint).encode() in regular_bytes(child.output, 33_554_432),
                    "missing owned Moto listener observation")
            self.children.tick(deadline)
            return observation

        observation = self.poll(ready, deadline)
        self.storage = Storage(self.endpoint, self.clock, self.tick)
        self.step("moto-ready", endpoint=self.endpoint, child=child.row(), health=observation)

    def storage_preflight(self):
        deadline = self.clock.phase(15)
        store = self.storage

        def request(operation, **arguments):
            return store.call(lambda: store.request(operation, **arguments), deadline)

        created = request("create_bucket", Bucket=self.bucket)
        require(created["status"] == 200, "fresh fixture bucket creation")
        key, first = "qualification/conditional", b"abcdefgh"
        inserted = request("put_object", Bucket=self.bucket, Key=key, Body=first, IfNoneMatch="*")
        require(inserted["status"] == 200 and isinstance(inserted["etag"], str), "create-if-absent")
        duplicate = request("put_object", Bucket=self.bucket, Key=key, Body=b"wrong", IfNoneMatch="*")
        require(duplicate["status"] == 412 and duplicate["code"] == "PreconditionFailed", "duplicate create accepted")
        updated = request("put_object", Bucket=self.bucket, Key=key, Body=b"ijklmnop", IfMatch=inserted["etag"])
        require(updated["status"] == 200 and isinstance(updated["etag"], str), "conditional update")
        stale = request("put_object", Bucket=self.bucket, Key=key, Body=b"wrong", IfMatch=inserted["etag"])
        require(stale["status"] == 412 and stale["code"] == "PreconditionFailed", "stale IfMatch accepted")
        barrier = threading.Barrier(3)
        barrier_timeout = max(0.001, self.clock.remaining(deadline))

        def race(value):
            barrier.wait(timeout=barrier_timeout)
            return store.request("put_object", Bucket=self.bucket, Key=key, Body=value, IfMatch=updated["etag"])

        tasks = [store.start(lambda value=value: race(value)) for value in (b"12345678", b"ABCDEFGH")]
        barrier.wait(timeout=max(0.001, self.clock.remaining(deadline)))
        results = [store.finish(task, deadline) for task in tasks]
        require(sorted(value["status"] for value in results) == [200, 412], "same-ETag race did not have one winner")
        loser = next(value for value in results if value["status"] != 200)
        require(loser["code"] == "PreconditionFailed", "same-ETag loser was not conditional refusal")
        winner = (b"12345678", b"ABCDEFGH")[next(i for i, value in enumerate(results) if value["status"] == 200)]
        whole = request("get_object", Bucket=self.bucket, Key=key)
        ranged = request("get_object", Bucket=self.bucket, Key=key, Range="bytes=2-4")
        require(whole["status"] == 200 and whole["body"] == winner, "conditional winner not durable in fixture")
        require(ranged["status"] == 206 and ranged["body"] == winner[2:5]
                and ranged["content_range"] == "bytes 2-4/8", "byte-range preflight")
        observations = []
        for value in (created, inserted, duplicate, updated, stale, *results, whole, ranged):
            observations.append({key: item for key, item in value.items() if key != "body"}
                                | {"body": self.evidence.save("s3-preflight", value["body"])})
        self.step("s3-preflight", bucket=self.bucket, results=observations)

    def deploy(self):
        project = self.directory / "project"
        project.mkdir()
        (project / "worker.mjs").write_bytes(WORKER_JS)
        (project / "wrangler.json").write_bytes(WRANGLER_JSON)
        require(sorted(path.name for path in project.iterdir()) == ["worker.mjs", "wrangler.json"],
                "unexpected deployment input")
        self.cli("deploy", ["deploy", str(project), "--bucket", "s3://" + self.bucket,
                            "--endpoint", self.endpoint, "--region", "us-east-1"])
        require((project / "worker.mjs").read_bytes() == WORKER_JS
                and (project / "wrangler.json").read_bytes() == WRANGLER_JSON, "deployment inputs changed")

    def start_node(self, label, letter, restart=False):
        require([child.label for child in self.nodes] == {
            "a": [], "b": ["a"], "b-restart": ["a", "b"], "c": ["a", "b", "b-restart"],
        }[label], "closed node incarnation order")
        directory = self.directory / ("node-" + letter)
        if restart:
            previous = self.nodes[-1]
            require(previous.label == "b" and previous.retired and previous.directory == directory,
                    "restart before prior B retirement")
            require((directory / ".clean-reload.json").is_file(), "restart certificate absent")
        else:
            directory.mkdir()
            require(not any(directory.iterdir()), "fresh node directory")
        node = "foks-a3a-" + letter + "-" + self.run_id
        pair = {"public": self.ports["public_b" if letter == "b" else "public_ac"],
                "internal": self.ports["internal_b" if letter == "b" else "internal_ac"]}
        environment = self.environment | self.profile["node_settings"] | {
            "CELLD_NODE": node, "CELLD_WATCH": str(directory),
        }
        argv = [str(self.binary), "--bucket", "s3://" + self.bucket, "--endpoint", self.endpoint,
                "--region", "us-east-1", "--listen", "127.0.0.1:" + str(pair["public"]),
                "--internal-listen", "127.0.0.1:" + str(pair["internal"]),
                "--advertise", "127.0.0.1:" + str(pair["internal"])]
        self.binary_check()
        child = self.children.spawn(label, argv, environment, directory, node, pair)
        self.nodes.append(child)
        deadline = min(self.clock.deadline, child.started_ns + 120_000_000_000)
        child.startup_deadline = deadline

        def healthy():
            try:
                status, raw, observation = self.response(label + "-health", pair["public"], "GET",
                                                          "/.well-known/celld/health", deadline)
            except ConnectionRefusedError:
                return None
            parsed = strict_json(raw)
            require((status == 200 and parsed == {"ok": True})
                    or (status == 503 and parsed == {"ok": False}), "unexpected health listener")
            if status == 503:
                return None
            log = regular_bytes(child.output, 33_554_432)
            require((f"celld listening on 127.0.0.1:{pair['public']} (ownership=").encode() in log,
                    "missing actual public listener observation")
            require((f"celld internal listening on 127.0.0.1:{pair['internal']} "
                     f"(advertise=127.0.0.1:{pair['internal']})\n").encode() in log,
                    "missing actual internal listener observation")
            return observation

        health = self.poll(healthy, deadline)
        def sampled_lease():
            lease, evidence = self.node_lease(child, deadline)
            require(isinstance(lease, dict) and isinstance(lease.get("load"), dict), "initial lease load")
            if type(lease["load"].get("sampled_ms")) is int and lease["load"]["sampled_ms"] == 0:
                return None
            child.generation = candidate(lease, node, "127.0.0.1:" + str(pair["internal"]), self.clock.now_ms())
            return lease, evidence

        lease, lease_evidence = self.poll(sampled_lease, deadline)
        self.step(label + "-ready", child=child.row(), health=health, lease=lease_evidence,
                  startup_elapsed_ns=self.clock.check(deadline) - child.started_ns)
        return child, lease

    def object(self, label, key, deadline, absent=False):
        result = self.storage.get(self.bucket, key, min(deadline, self.clock.phase(15)))
        if absent and result["status"] == 404 and result["code"] == "NoSuchKey":
            observation = {"key": key, "status": 404, "code": "NoSuchKey",
                           "body": self.evidence.save(label, result["body"])}
            return None, observation
        require(result["status"] == 200 and result["code"] is None, "storage GET failed: " + label)
        value = strict_json(result["body"])
        return value, {"key": key, "status": 200, "etag": result["etag"],
                       "body": self.evidence.save(label, result["body"])}

    def node_lease(self, child, deadline):
        require(child.process.poll() is None, "lease observation needs a live child")
        result, evidence = self.object(child.label + "-lease", "nodes/" + child.node + ".json", deadline)
        require(child.process.poll() is None, "node exited during lease observation")
        return result, evidence

    def snapshot(self, child, kind, deadline):
        lease, lease_evidence = self.node_lease(child, deadline)
        candidate(lease, child.node, "127.0.0.1:" + str(child.ports["internal"]),
                  self.clock.now_ms(), child.generation)
        status, raw, observation = self.response(child.label + "-state", child.ports["internal"],
                                                "GET", "/state", deadline)
        require(status == 200 and child.process.poll() is None, "state observation needs a live child")
        state = strict_json(raw)
        cell = state_census(state, kind, self.cell)
        return cell, {"state": observation, "lease": lease_evidence, "generation": child.generation}

    def worker(self, child, suffix, write=False):
        deadline = self.clock.phase(15)
        marker = self.run_id + "-" + suffix
        status, raw, observation = self.response(child.label + ("-write" if write else "-read"),
                                                child.ports["public"], "POST" if write else "GET",
                                                "/write" if write else "/read", deadline,
                                                marker.encode() if write else b"")
        # The full bounded body is already retained by response(). Report HTTP
        # failure before attempting success JSON; repr escapes the short preview.
        require(status == 200, "Worker HTTP status " + str(status) + "; body preview " + repr(raw[:128]))
        value = strict_json(raw)
        require(isinstance(value, dict) and set(value) == {"fixture", "marker", "rows"}
                and value["fixture"] == "foks-celld-a3a-v1" and value["marker"] == marker
                and type(value["rows"]) is int and value["rows"] == 1,
                "Worker marker/sync response")
        self.step(child.label + ("-write-synced" if write else "-read-restored"),
                  response=observation, marker=marker, rows=value["rows"],
                  fresh_directory=child.label in ("a", "b", "c"))

    def cell_owner(self, child, deadline, epoch=None):
        value, evidence = self.object(child.label + "-owner", "cells/" + self.cell + "/own.json", deadline)
        return owner(value, child.node, epoch), value, evidence

    def ready_candidate(self, child):
        deadline = child.startup_deadline
        require(type(deadline) is int, "missing original startup deadline")
        self.clock.check(deadline)

        def check():
            lease, evidence = self.node_lease(child, deadline)
            # Sampling can lag the initial lease publication. Only this one exact
            # not-ready state is retried; malformed/pressured/draining peers refuse.
            load = lease.get("load")
            require(isinstance(load, dict), "candidate load shape")
            if type(load.get("sampled_ms")) is int and load["sampled_ms"] == 0:
                return None
            candidate(lease, child.node, "127.0.0.1:" + str(child.ports["internal"]),
                      self.clock.now_ms(), child.generation)
            _, state = self.snapshot(child, "empty", deadline)
            self.step(child.label + "-candidate", lease=evidence, state=state)
            return lease

        return self.poll(check, deadline)

    def shutdown(self, child, preserve=False, sigterm=False):
        require(self.active_responses == 0 and not self.storage.pending, "work still active before shutdown")
        started = self.clock.check()
        deadline = self.clock.phase(50)
        child.expected_exit = True
        if sigterm:
            child.process.send_signal(signal.SIGTERM)
            trigger = {"signal": "SIGTERM", "pid": child.process.pid}
        else:
            status, raw, trigger = self.response(child.label + "-shutdown", child.ports["internal"],
                                                 "POST", "/shutdown?handoff=preserve" if preserve else "/shutdown",
                                                 min(deadline, self.clock.phase(15)))
            require(status == 200 and strict_json(raw) == {"ok": True}, "canonical shutdown response")
        status = self.children.wait(child, deadline)
        elapsed = child.exit_ns - started
        successful_exit(status, elapsed)
        self.binary_check()
        raw = regular_bytes(child.output, 33_554_432)
        parse_logs(raw)
        return raw, {"trigger": trigger, "started_ns": started, "exit_ns": child.exit_ns,
                     "elapsed_ns": elapsed, "child": child.row(), "log": identity(raw)}

    def handoff(self, donor, receiver, epoch, sigterm=False):
        # Immediately re-read candidate eligibility under the original request cap.
        deadline = self.clock.phase(15)
        lease, eligibility = self.node_lease(receiver, deadline)
        candidate(lease, receiver.node, "127.0.0.1:" + str(receiver.ports["internal"]),
                  self.clock.now_ms(), receiver.generation)
        last_donor, donor_evidence = self.node_lease(donor, deadline)
        candidate(last_donor, donor.node, "127.0.0.1:" + str(donor.ports["internal"]),
                  self.clock.now_ms(), donor.generation)
        raw, exit_observation = self.shutdown(donor, sigterm=sigterm)
        witness = handoff_witness(raw, self.cell, receiver.node, epoch)
        deadline = self.clock.phase(15)
        _, dormant = self.snapshot(receiver, "dormant", deadline)
        accepted_epoch, _, ownership = self.cell_owner(receiver, deadline, witness["epoch"])
        self.step(donor.label + "-handoff", eligibility=eligibility, last_donor_lease=donor_evidence,
                  exit=exit_observation, witness=witness, dormant=dormant, owner=ownership)
        return accepted_epoch, last_donor

    def preserve(self, child):
        deadline = self.clock.phase(15)
        _, state = self.snapshot(child, "resident", deadline)
        require(not (child.directory / ".clean-reload.json").exists(), "stale Preserve marker")
        raw, exit_observation = self.shutdown(child, preserve=True)
        marker_raw = regular_bytes(child.directory / ".clean-reload.json", 65536)
        witness = preserve_witness(raw, marker_raw, child.node, child.generation)
        marker_evidence = self.evidence.save(child.label + "-clean-reload", marker_raw)
        self.step(child.label + "-preserve", state=state, exit=exit_observation,
                  witness=witness, certificate=marker_evidence)

    def expire_old_a(self, child, last, live, epoch):
        deadline = self.clock.phase(120)

        def observe():
            self.children.scan(deadline, force=True)
            require(child.retired and not child.descendants & self.children.table.keys(), "old A not retired")
            now = self.clock.now_ms()
            if integer(last.get("expires_ms"), "old A captured expiry") > now:
                return None
            value, evidence = self.object("old-a-key", "nodes/" + child.node + ".json", deadline, absent=True)
            if value is not None and integer(value.get("expires_ms"), "old A actual expiry") > now:
                # Its last renewal can precede physical exit but follow our saved read.
                # Preserve that exact newer same-identity observation for the expiry wait.
                for key in ("node", "addr", "ownership_index_generation", "probe_public_key"):
                    require(value.get(key) == last.get(key), "old A identity changed")
                last.update(value)
                return None
            live_lease, _ = self.node_lease(live, deadline)
            candidate(live_lease, live.node, "127.0.0.1:" + str(live.ports["internal"]),
                      now, live.generation)
            _, actual_owner, ownership = self.cell_owner(live, deadline, epoch)
            missing = {"status": 404, "code": "NoSuchKey"} if value is None else None
            branch = old_node_ineligible(last, value, missing, now, child.retired, actual_owner, live.node, epoch)
            self.step("old-a-ineligible", branch=branch, node=evidence, owner=ownership,
                      last_observed_identity={key: last.get(key) for key in
                                              ("node", "addr", "ownership_index_generation", "probe_public_key", "expires_ms", "log")},
                      outstanding_put_termination_proved=False, gc_execution_proved=False)
            return True

        self.poll(observe, deadline)

    def execute(self):
        self.cli("version", ["--version"], text=b"celld 0.6.2 (debug)\n")
        self.cli("help", ["--help"], text=b"--bucket")
        self.cli("invalid-option", ["--foks-a3a-invalid-option"], expected_status=None,
                 text=b"unknown command or option: --foks-a3a-invalid-option")
        self.moto()
        self.storage_preflight()
        self.deploy()
        a, _ = self.start_node("a", "a")
        self.worker(a, "one", write=True)
        self.worker(a, "one")
        self.cell, state = self.snapshot(a, "resident", self.clock.phase(15))
        epoch_a, _, ownership = self.cell_owner(a, self.clock.phase(15))
        self.step("a-owned", cell=self.cell, state=state, owner=ownership, epoch=epoch_a)
        b, _ = self.start_node("b", "b")
        self.ready_candidate(b)
        epoch_b, last_a = self.handoff(a, b, epoch_a)
        self.worker(b, "one")
        self.worker(b, "two", write=True)
        self.preserve(b)
        restarted, _ = self.start_node("b-restart", "b", restart=True)
        self.worker(restarted, "two")
        require(not (restarted.directory / ".clean-reload.json").exists(), "restart marker not consumed")
        restarted_epoch, _, _ = self.cell_owner(restarted, self.clock.phase(15))
        require(restarted_epoch >= epoch_b, "restart ownership epoch regressed")
        self.step("b-marker-consumption", consumed=True, exact_generation_resume_proved=False,
                  previous_generation=b.generation, generation=restarted.generation,
                  previous_epoch=epoch_b, epoch=restarted_epoch)
        self.expire_old_a(a, last_a, restarted, restarted_epoch)
        c, _ = self.start_node("c", "c")
        self.ready_candidate(c)
        self.handoff(restarted, c, restarted_epoch, sigterm=True)
        self.worker(c, "two")
        self.preserve(c)
        require([node.label for node in self.nodes] == ["a", "b", "b-restart", "c"], "incarnation roster")
        self.evidence.check(force=True)


def parse_arguments(argv):
    fields = ("profile", "binary", "binary-sha256", "directory", "deadline-ns", "node-lock-sha256",
              *(role.replace("_", "-") + "-port" for role in ROLES))
    require(len(argv) == len(fields) * 2 and set(argv[::2]) == {"--" + key for key in fields},
            "exact controller flags required once each")
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    for key in fields:
        parser.add_argument("--" + key, required=True, type=int if key.endswith("-port") or key == "deadline-ns" else str)
    arguments = parser.parse_args(argv)
    require(HEX64.fullmatch(arguments.binary_sha256) and arguments.node_lock_sha256 == PROFILE_SHA256,
            "artifact/profile hash argument")
    ports = [getattr(arguments, role + "_port") for role in ROLES]
    require(all(type(port) is int and 1024 <= port <= 65535 for port in ports) and len(set(ports)) == 5,
            "five distinct loopback ports")
    return arguments


def validate_inputs(arguments, script):
    root = script.parent
    require(Path(arguments.profile) == root / "node.lock.json", "fixed node profile path")
    raw = regular_bytes(arguments.profile, 131072)
    require(identity(raw)["sha256"] == PROFILE_SHA256, "node profile changed")
    profile = strict_json(raw, 131072)
    require(profile["program"] == {"js": identity(WORKER_JS), "wrangler": identity(WRANGLER_JSON)},
            "literal program changed")
    process_inspector_command(profile, os.getpid(), os.getpgrp())
    require(Path(arguments.binary) == root / "runtime/target/aarch64-apple-darwin/debug/celld",
            "fixed celld executable path")
    require(Path(arguments.binary).resolve(strict=True) == Path(arguments.binary), "executable symlink path")
    directory = Path(arguments.directory)
    require(directory.resolve(strict=True) == directory and stat.S_ISDIR(directory.lstat().st_mode)
            and directory.name == "run" and directory.parent.parent == root / "runtime/evidence"
            and re.fullmatch(r"node-[a-z0-9_-]{1,32}", directory.parent.name), "fixed fresh evidence parent")
    require(not any((directory / name).exists() or (directory / name).is_symlink()
                    for name in ("report.json", "report.json.tmp", "project", "observations", "node-a", "node-b", "node-c")),
            "controller evidence paths are not fresh")
    return profile


def save_report(directory, report):
    raw = (json.dumps(report, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()
    require(len(raw) <= 2_097_152, "report size bound")
    temporary, final = directory / "report.json.tmp", directory / "report.json"
    require(not final.exists() and not final.is_symlink(), "report already exists")
    with temporary.open("xb") as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, final)
    return {"path": str(final), "sha256": identity(raw)["sha256"], "status": report["status"]}


def main(argv=None):
    arguments = parse_arguments(sys.argv[1:] if argv is None else argv)
    script = Path(__file__).resolve(strict=True)
    profile = validate_inputs(arguments, script)
    clock = Clock(arguments.deadline_ns)
    environment = check_environment(profile, Path(arguments.directory))
    startup = startup_observation(profile, script)
    run = Run(arguments, profile, clock, environment)
    saved_handlers = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    failure = None
    cleanup = {"completed": False, "children": [], "storage_tasks_joined": False,
               "active_responses": 0, "outer_group_disappearance_observed": False, "error": None}

    def cancel(_signal, _frame):
        clock.cancelled = True

    try:
        for sig in saved_handlers:
            signal.signal(sig, cancel)
        try:
            startup["fixture_versions"] = fixture_versions(profile)
            startup["after_fixture_import"] = startup_observation(profile, script)
            run.execute()
        except BaseException as error:
            failure = type(error).__name__ + ": " + str(error)[:1024]
        finally:
            # Cleanup runs even if execution, an observation write, or cancellation
            # fails. A task is never detached just to produce a passed report.
            try:
                cleanup.update(run.children.cleanup())
            except BaseException as error:
                cleanup["error"] = [type(error).__name__ + ": child cleanup incomplete"]
            try:
                cleanup["storage_tasks_joined"] = run.storage is None or run.storage.cleanup()
            except BaseException as error:
                cleanup["storage_tasks_joined"] = False
                cleanup["error"] = (cleanup["error"] or []) + [type(error).__name__ + ": storage cleanup"]
            cleanup["active_responses"] = run.active_responses
            cleanup["completed"] = (cleanup["completed"] and cleanup["storage_tasks_joined"]
                                    and run.active_responses == 0)
        if failure is None:
            try:
                require(all(child.process.returncode == 0 for child in run.children.items
                            if child.label != "invalid-option"), "nonzero command/fixture exit")
                run.binary_check()
                require(identity(regular_bytes(arguments.profile, 131072))["sha256"] == PROFILE_SHA256,
                        "node profile changed during run")
                startup["final_module_observation"] = startup_observation(profile, script)
                run.evidence.check(force=True)
                require(cleanup["completed"], "controller cleanup incomplete")
            except BaseException as error:
                failure = type(error).__name__ + ": " + str(error)[:1024]
        report = {
            "format": 1, "profile": "a3a-canonical-node-baseline",
            "claim": "a3a-observed-canonical-handoff-restore-preserve",
            "status": "passed" if failure is None and cleanup["completed"] else "failed",
            "node_lock_sha256": PROFILE_SHA256,
            "binary": {"path": arguments.binary, "sha256": arguments.binary_sha256},
            "program": {"js": identity(WORKER_JS), "wrangler": identity(WRANGLER_JSON)},
            "ports": run.ports, "run_id": run.run_id, "startup": startup,
            "sequence": {"completed": failure is None, "steps": run.steps,
                         "incarnations": [child.row() for child in run.nodes], "error": failure},
            "cleanup": cleanup,
            "observation_limits": {"final_transport_grace_observed": False, "internal_task_drain_observed": False},
        }
        summary = save_report(run.directory, report)
        print(json.dumps(summary, sort_keys=True, separators=(",", ":")), flush=True)
        return 0 if report["status"] == "passed" else 1
    finally:
        for sig, handler in saved_handlers.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (Refused, OSError, ValueError) as error:
        print(type(error).__name__ + ": " + str(error)[:1024], file=sys.stderr, flush=True)
        raise SystemExit(1)

#!/usr/bin/env python3
"""Prepare pinned A1/A2 inputs, A3a nodes and explicit B1/B2/B3 and P1 stages.

verify checks only the original upstream archive. prepare also applies the closed
A2 patch in private staging. assets accepts the two reviewed local V8 assets.
No command downloads dependencies. Graph/build/runtime are explicit actions after review,
and share one fail-closed process-group build slot.
"""
import argparse
import ast
import contextlib
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import socket
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
NODE = "a3a-canonical-node-baseline"
NODE_CLAIM = "a3a-observed-canonical-handoff-restore-preserve"
NODE_BASE = "dca38eecb08263a3594f70025df7a309ed3682a5"
NODE_PROFILE_SHA256 = "af0c4f23c08fd2c1d17126c0b7c2d68720ecd275840945de444eae7d46bcc866"
NODE_SOURCE_PATHS = ("prepare-runtime.py", "check_graph.py", "test_prepare_runtime.py", "test_check_graph.py", "node.lock.json", "run-node.py", "test_run_node.py")
NODE_ROLES = ("s3", "public_ac", "internal_ac", "public_b", "internal_b")
NODE_POLICY = '(version 1)\n(allow default)\n(deny network*)\n(allow network-bind (local tcp "localhost:*"))\n' + "".join(
    "(allow network-" + direction + " " + " ".join('(' + address + ' tcp "localhost:{' + role + '}")' for role in NODE_ROLES) + ")\n"
    for direction, address in (("inbound", "local"), ("outbound", "remote"))
)
NODE_LIMITS = {"clock_drift_ns": 1_000_000_000, "diagnostic_bytes": 65536, "evidence_bytes": 256 * 1024**2, "log_bytes": 32 * 1024**2, "maximum_nodes": 2, "node_directories": 3, "node_incarnations": 4, "report_bytes": 2 * 1024**2, "request_seconds": 15, "scenario_seconds": 600, "shutdown_seconds": 50, "startup_seconds": 120, "successful_shutdown_ns": 35_000_000_000}
FIXTURE_LIMITS = {"bytes": 1024**3, "canonical_json_bytes": 8 * 1024**2, "entries": 25000, "native_files": 150, "symlink_hops": 32}
MACHO_MAGICS = {bytes.fromhex(value) for value in ("feedface", "cefaedfe", "feedfacf", "cffaedfe", "cafebabe", "bebafeca", "cafebabf", "bfbafeca")}
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
FOUNDATION = "node-foundation"
FOUNDATION_BASE = "fee8db31ae8df590e32bc02f8c8de815d9f7ecea"
FOUNDATION_PROFILE = "runtime-node.lock.json"
FOUNDATION_SERIES = "patches/node-series.json"
FOUNDATION_PATCH = "patches/0002-node-admission.patch"
FOUNDATION_MANIFEST = "runtime/evidence/accepted-a3a/prepared-inputs/source-manifest.json"
FOUNDATION_PATHS = tuple(sorted((
    "crates/celld/Cargo.toml", "crates/celld/lib.rs", "crates/celld/guarded_execution.rs",
    "crates/celld/guarded_node.rs", "crates/celld/guarded_node/control.rs", "crates/celld/guarded_node/tests.rs",
    "crates/celld/generation.rs", "crates/celld/actor.rs", "crates/celld/actor/effects.rs", "crates/celld/runtime.rs",
    "crates/celld/js.rs", "crates/celld/js/guarded_node.rs", "crates/logic/types.rs", "crates/logic/lib.rs",
)))
FOUNDATION_ADDITIONS = frozenset(("crates/celld/guarded_node.rs", "crates/celld/guarded_node/control.rs", "crates/celld/guarded_node/tests.rs", "crates/celld/js/guarded_node.rs"))
FOUNDATION_BASE_IMAGES = {
    "runtime_lock": ("runtime.lock.json", "4e94be5e3a8f86007bcdc9c0814ce2fa6dfb043d612d006a20e47b490df67b49"),
    "patch_series": ("patches/series.json", "bf15b77b32416914a1d96426aa58fc65d1b436e0645c73dcc11ad91f6e9c0ea6"),
    "node_profile": ("node.lock.json", NODE_PROFILE_SHA256),
    "source_manifest": (FOUNDATION_MANIFEST, "0b618d7be50b44d5a8eeecbbab851ef0e3c3c9b2de64c5e33afbc6f9259489c5"),
}
FOUNDATION_LOCK_BEFORE = "6d838801444d99b44acf0b8ef6872c80e24929660557ec4dc0a39a704c6dcb0d"
FOUNDATION_EDGES = ["foks-hosted-transport", "foks-metadata-core"]
FOUNDATION_MODES = {
    "node-foundation-default": {"root_features": [], "celld_features": [], "celld_foks_edges": [], "direct_root_tokio": False},
    "node-foundation": {"root_features": ["node-qualification"], "celld_features": ["foks-guarded-node-qualification", "foks-guarded-qualification"], "celld_foks_edges": FOUNDATION_EDGES, "direct_root_tokio": False},
    "node-foundation-lifecycle": {"root_features": ["lifecycle"], "celld_features": ["foks-guarded-qualification"], "celld_foks_edges": [], "direct_root_tokio": True},
}
FOUNDATION_TEST_PREFIX = "guarded_execution::node::tests::b1_"
FOUNDATION_TESTS = [FOUNDATION_TEST_PREFIX + name for name in (
    "role_geometry_counts_process_owner", "last_strong_and_registry_retirement_hold_role",
    "control_headroom_survives_full_invocation_debt", "control_lane_busy_does_not_consume_cleanup",
    "header_cells_are_process_owned_and_bounded", "original_stamp_never_renews",
    "debt_failure_sticky_and_cleanup_one_use", "owner_construction_and_payload_drop_order",
    "layout_and_role_ledger", "descriptor_accepts_exact_projection",
    "descriptor_rejects_each_extra_capability", "boot_reload_share_construction_selection",
    "marked_worker_refuses_before_engine_use", "generic_refusal_inventory",
    "exact_generation_never_falls_back", "resident_gate_accepts_exact_parent",
    "resident_gate_refuses_binding_matrix", "resident_gate_duplicate_is_inert",
    "soft_swap_gate_and_ordinary_gate_regression", "inactive_observation_is_not_processed_finish",
    "foundation_has_no_ready_or_action",
)]
FOUNDATION_EXCLUSIONS = ["guarded-readiness", "guarded-action", "same-process-serving", "host-az-recovery", "whole-node-retirement"]
# Pinned main/cli.rs print_help with DEFAULT_SHUTDOWN_TOTAL_MS=40000 and
# cli_output::Output::help's final newline. A prefix is not a help oracle.
FOUNDATION_HELP_IMAGE = (7752, "421243bdfe4c6f4e89834852eadd8becb82a1b3931f4d29fd80b5f9da4f6c4f9")
FOUNDATION_JEMALLOC_WARNING = b"<jemalloc>: option background_thread currently supports pthread only\n"
FOUNDATION_MEMORY_WARNING = re.compile(
    rb"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z  WARN celld::memory: "
    rb"the allocator will not run a background thread, so freed pages return only when a thread allocates again "
    rb"error=`name` or `mib` specifies an unknown/invalid value\.\n"
)
MAXIMUM_FOUNDATION_PATCH = 512 * 1024
# B2 alone needs the larger source diff; immutable prefix caps stay unchanged.
MAXIMUM_LIFETIMES_PATCH = 768 * 1024
MAXIMUM_FOUNDATION_SERIES = 1536 * 1024
MAXIMUM_FOUNDATION_SOURCE = 8 * 1024 * 1024
LIFETIMES = "node-lifetimes"
LIFETIMES_BASE = "23039e209d1b3b129a274dd65fe07f0abfdba8e0"
LIFETIMES_PATCH = "patches/0003-node-lifetimes.patch"
LIFETIMES_PROFILE = "runtime/evidence/accepted-b1/runtime-node.lock.json"
LIFETIMES_SERIES = "runtime/evidence/accepted-b1/patches/node-series.json"
LIFETIMES_MANIFEST = "runtime/evidence/accepted-b1/prepared-inputs/source-manifest.json"
LIFETIMES_ASSETS = "runtime/evidence/accepted-b1/assets-node-foundation.json"
LIFETIMES_BASE_IMAGES = {
    "foundation_profile": (LIFETIMES_PROFILE, "71347fc8ddb07d3fe894f02f0c7733fa247ebdfcadbe5acceadfa646c1230612"),
    "foundation_series": (LIFETIMES_SERIES, "6716ff40c680dddfc597ee0ce708e3a0140d3dd9fd81c925d97d54162bfa6073"),
    "foundation_source_manifest": (LIFETIMES_MANIFEST, "e59f9ce8ef088f8da741ddaf8e8ee488bfba27d508948bab55d0495b73728de2"),
}
LIFETIMES_ASSET_SHA256 = "4bf32d523d6406c92be8f66ee2a1f7c6dbb77d8e7e09610bd14cdd26fbaaac1c"
LIFETIMES_LOCK_SHA256 = "e05fe62986b04425769de96caae00224aeafa9513a9c38468a81410c28d65053"
LIFETIMES_PATHS = tuple(sorted((
    "crates/celld/guarded_execution.rs", "crates/celld/guarded_node.rs",
    "crates/celld/guarded_node/control.rs", "crates/celld/guarded_node/tests.rs",
    "crates/celld/generation.rs", "crates/celld/cell_runtime.rs", "crates/celld/actor.rs",
    "crates/celld/asyncrt.rs", "crates/celld/lib.rs",
    "crates/celld/actor/local_request.rs", "crates/celld/actor/effects.rs", "crates/celld/actor/production.rs",
    "crates/celld/host_channels.rs", "crates/celld/cell_dispatch.rs", "crates/celld/runtime.rs",
    "crates/celld/js.rs", "crates/celld/js/storage_ops.rs", "crates/celld/js/guarded.rs",
    "crates/celld/js/guarded_node.rs", "crates/celld/ltx_repl.rs", "crates/celld/ltx_replication.rs",
)))
LIFETIMES_MODES = {
    "node-lifetimes-default": FOUNDATION_MODES["node-foundation-default"],
    "node-lifetimes": FOUNDATION_MODES["node-foundation"],
    "node-lifetimes-lifecycle": FOUNDATION_MODES["node-foundation-lifecycle"],
}
LIFETIMES_TEST_PREFIX = "guarded_execution::node::tests::"
LIFETIMES_TESTS = FOUNDATION_TESTS + [LIFETIMES_TEST_PREFIX + "b2_" + name for name in (
    "effect_inventory_is_closed", "original_origin_cannot_be_rebound",
    "forward_saturation_preserves_cleanup", "cleanup_credit_and_child_layout_ledger",
    "actual_task_handle_outlives_delivery_wrapper", "native_child_outlives_dropped_await",
    "result_channel_disposal_precedes_reuse", "panic_and_unknown_result_quarantine",
    "cleanup_retry_does_not_replenish_credit", "stale_effect_cannot_complete_new_epoch",
    "generation_pin_blocks_drained_removal", "retired_generation_refuses_new_pin",
    "canonical_adoption_binding_has_real_path_epoch_vfs", "dedicated_driver_join_precedes_slot_consumption",
    "guarded_resident_refuses_ordinary_worker_routes", "sync_origin_uses_actual_parent_and_sample",
    "sync_auxiliary_is_single_flight", "sync_parent_binding_matrix",
    "finish_processed_is_not_inactive", "finish_delivery_loss_retains_control",
    "failed_turn_keeps_output_ticket_semantics", "epoch_attachment_precedes_activation_work",
    "bucket_registration_cannot_change_while_attached", "hydration_join_precedes_vfs_unregister",
    "sync_wait_timeout_keeps_native_work_owned", "compaction_cancel_keeps_native_result_owned",
    "guarded_background_queue_is_bounded", "cell_snapshot_holds_epoch_retirement",
    "native_close_failure_after_worker_drop_holds_slot", "retirement_requires_every_conjunct",
    "original_deadline_denies_new_work_but_allows_reserved_cleanup", "lifetimes_has_no_ready_action_or_frontend",
)]
FRONTEND = "node-qualification"
FRONTEND_BASE = "d22ff3deedf0f0a87f635aec2317a87410e39110"
FRONTEND_PATCH = "patches/0004-node-frontend.patch"
FRONTEND_PROFILE = "runtime/evidence/accepted-b2/runtime-node.lock.json"
FRONTEND_SERIES = "runtime/evidence/accepted-b2/patches/node-series.json"
FRONTEND_MANIFEST = "runtime/evidence/accepted-b2/prepared-inputs/source-manifest.json"
FRONTEND_ASSETS = "runtime/evidence/accepted-b2/assets-node-lifetimes.json"
FRONTEND_BASE_IMAGES = {
    "lifetimes_profile": (FRONTEND_PROFILE, "b005ea3af86f1a09243184f359321d51ba1f81caf4aff34d22e377de32d359d3"),
    "lifetimes_series": (FRONTEND_SERIES, "f7e5d4dc073cabbc7dcdec17e798163881f04da2a60952c8dae53f960577622e"),
    "lifetimes_source_manifest": (FRONTEND_MANIFEST, "b06b21c3014d4745e8ef1898a0165affd8f67026bb07962ff75188cafb5e3890"),
}
FRONTEND_ASSET_SHA256 = "53710086f7a73a54bb44ba00aae50ccaa060a10a3ff62ccd63ce4b67a9705dc6"
FRONTEND_ADDITIONS = frozenset(("crates/celld/main/guarded.rs", "crates/celld/guarded_node/output.rs"))
FRONTEND_UNION = tuple(sorted(set(FOUNDATION_PATHS) | set(LIFETIMES_PATHS) | {
    "crates/celld/main.rs", "crates/celld/main/cli.rs", "crates/celld/node_log.rs", *FRONTEND_ADDITIONS,
}))
# Exact fourth-patch roster; the immutable three-patch prefix stays separate.
FRONTEND_PATHS = tuple(sorted((
    "crates/celld/main.rs", "crates/celld/main/cli.rs", "crates/celld/main/guarded.rs",
    "crates/celld/lib.rs", "crates/celld/node_log.rs", "crates/celld/guarded_node.rs",
    "crates/celld/guarded_node/control.rs", "crates/celld/guarded_node/output.rs",
    "crates/celld/guarded_node/tests.rs", "crates/celld/runtime.rs", "crates/celld/actor.rs",
    "crates/celld/actor/local_request.rs", "crates/celld/cell_dispatch.rs",
    "crates/celld/host_channels.rs", "crates/celld/guarded_execution.rs",
)))
FRONTEND_MODES = {
    "node-qualification-default": FOUNDATION_MODES["node-foundation-default"],
    "node-qualification": FOUNDATION_MODES["node-foundation"],
    "node-qualification-lifecycle": FOUNDATION_MODES["node-foundation-lifecycle"],
}
OBSERVATIONS = "node-observations"
OBSERVATIONS_BASE = "9151df82364fa60177cb62f6645746ce1aff9b48"
OBSERVATIONS_PATCH = "patches/0005-node-observations.patch"
# Distinct from the series JSON envelope; the fixed four-patch prefix plus
# the fifth-patch ceiling fits this P1-only combined payload bound.
MAXIMUM_OBSERVATIONS_PATCHES = 1536 * 1024
OBSERVATIONS_PROFILE = "runtime/evidence/accepted-b3/runtime-node.lock.json"
OBSERVATIONS_SERIES = "runtime/evidence/accepted-b3/patches/node-series.json"
OBSERVATIONS_MANIFEST = "runtime/evidence/accepted-b3/prepared-inputs/source-manifest.json"
OBSERVATIONS_ASSETS = "runtime/evidence/accepted-b3/assets-node-qualification.json"
OBSERVATIONS_BASE_IMAGES = {
    "frontend_profile": (OBSERVATIONS_PROFILE, "1d127eb41af8eff863e38e15921dc695854378e6c1f196665a3f4ffca2eb7a0f"),
    "frontend_series": (OBSERVATIONS_SERIES, "4d5cc7dd369a0ee3371f575d58ed8b73f75403bb723d58977c1f960d367088ce"),
    "frontend_source_manifest": (OBSERVATIONS_MANIFEST, "46d6fe4d673ff467b2fb420a1e49a06dbbd9e6989f14208c22b1454b49922f07"),
}
OBSERVATIONS_ASSET_SHA256 = "4cd70ccf1bcdb80d7cffa8b86ab2eac210b881430f950106fed8e90797059083"
OBSERVATIONS_PATHS = tuple(sorted((
    "crates/celld/actor.rs", "crates/celld/guarded_execution.rs",
    "crates/celld/guarded_node.rs", "crates/celld/guarded_node/control.rs",
    "crates/celld/guarded_node/output.rs", "crates/celld/guarded_node/tests.rs",
    "crates/celld/runtime.rs", "crates/celld/main/guarded.rs",
)))
OBSERVATIONS_MODES = {
    "node-observations-default": FOUNDATION_MODES["node-foundation-default"],
    "node-observations": FOUNDATION_MODES["node-foundation"],
    "node-observations-lifecycle": FOUNDATION_MODES["node-foundation-lifecycle"],
}
NODE_STAGES = (FOUNDATION, LIFETIMES, FRONTEND, OBSERVATIONS)
NODE_MODES = {**FOUNDATION_MODES, **LIFETIMES_MODES, **FRONTEND_MODES, **OBSERVATIONS_MODES}
NODE_BINARY_MODES = ("node-foundation-default", "node-foundation", "node-lifetimes-default", "node-lifetimes", "node-qualification-default", "node-qualification", "node-observations-default", "node-observations")
NODE_LIFECYCLE_MODES = ("node-foundation-lifecycle", "node-lifetimes-lifecycle", "node-qualification-lifecycle", "node-observations-lifecycle")
FRONTEND_TEST_PREFIX = LIFETIMES_TEST_PREFIX
FRONTEND_TEST_RENAMES = {
    "b1_foundation_has_no_ready_or_action": "b1_foundation_requires_frontend_for_ready_and_action",
    "b2_lifetimes_has_no_ready_action_or_frontend": "b2_lifetimes_requires_frontend_for_ready_and_action",
}
FRONTEND_TESTS = [FRONTEND_TEST_PREFIX + FRONTEND_TEST_RENAMES.get(name.removeprefix(FRONTEND_TEST_PREFIX), name.removeprefix(FRONTEND_TEST_PREFIX)) for name in LIFETIMES_TESTS] + [FRONTEND_TEST_PREFIX + "b3_" + name for name in (
    "header_closed_matrix", "header_eof_and_original_deadline", "tenants_share_two_slots",
    "stop_admission_preserves_existing_owner", "output_reservation_is_one_shot",
    "output_owner_survives_pending_write", "tcp_pending_then_complete",
    "tcp_disconnect_disposes_before_retirement", "flush_pending_then_complete",
    "cancel_already_set_refuses_write", "canonical_cancel_during_write",
    "canonical_cancel_during_flush", "output_original_deadline", "bucket_probe_positive",
    "bucket_probe_transport_refusal", "bucket_probe_contract_refusal",
    "public_guard_survives_pending_owner", "gate_dispatch_disposal_fence",
    "both_shutdown_modes_require_guarded_drain", "persistent_alias_does_not_self_wake",
    "frontend_layout_ledger",
)]
OBSERVATIONS_TESTS = FRONTEND_TESTS + [FRONTEND_TEST_PREFIX + name for name in (
    'b4_observation_progress_requires_positive_partial_pending',
    'b4_observation_checked_progress_rejects_overflow',
    'b4_observation_phase_identity_and_stale_slot',
    'b4_observation_archive_is_finite',
    'b4_observation_proof_verdict_is_not_disposal',
    'b4_observation_c1_required_and_not_required',
    'b4_observation_processed_inactive_and_disposed_are_distinct',
    'b4_observation_public_guard_precedes_bank_free',
    'b4_observation_failed_retirement_retains_public_guard',
    'b4_observation_scalar_copy_does_not_pin_owner',
    'b4_observation_contention_refuses_without_false_free',
    'b4_observation_stream_bounds_and_layouts',
)]
FRONTEND_SCENARIOS = ("cold-sync-retire", "failed-turn-gated", "tcp-write-held", "tcp-disconnect", "shutdown-complete", "shutdown-cut")
FRONTEND_LARGE_SCENARIOS = ("tcp-write-held", "tcp-disconnect", "shutdown-cut")
FRONTEND_EXCLUSIONS = ["full-process-success", "full-driver-terminal-conjunction", "host-az-recovery", "production-metadata", "whole-node-retirement", "remote-put-termination"]
OBSERVATIONS_EXCLUSIONS = FRONTEND_EXCLUSIONS.copy()
FRONTEND_ROLES = ("tcp", "bucket")
FRONTEND_PORT_ENV = {"tcp": "FOKS_B3_TCP_PORT", "bucket": "FOKS_B3_BUCKET_PORT"}
FRONTEND_POLICY = '(version 1)\n(allow default)\n(deny network*)\n' + "".join(
    "(allow network-" + direction + " " + " ".join('(' + address + ' tcp "localhost:{' + role + '}")' for role in FRONTEND_ROLES) + ")\n"
    for direction, address in (("bind", "local"), ("inbound", "local"), ("outbound", "remote"))
)
FRONTEND_HELP_IMAGE = (8104, "f24c444c1931fae236d53eff71f236200adc448a196c2649587182930a20eb97")
FRONTEND_REFUSALS = {
    "frontend-missing-scenario": (["foks-guarded-node"], b"Error: foks-guarded-node requires a closed scenario\n"),
    "frontend-unsupported-scenario": (["foks-guarded-node", "unsupported"], b"Error: unsupported foks-guarded-node scenario\n"),
    "frontend-absent": (["foks-guarded-node", "cold-sync-retire"], b"Error: unknown command or option: foks-guarded-node; run `celld --help` for usage\n"),
}


# These are source-derived policies, never measurements or caller-supplied caps.
LIFETIMES_FIXED_POLICY = {
    "node_control": ("inline-observation", False, 4096),
    "invocation_record": ("invocation", True, 1048576),
    "effect_registry": ("process", True, 65536),
    "root_cell": ("inline-observation", False, 8192),
    "native_child_cell": ("inline-observation", False, 4096),
    "control_cell": ("inline-observation", False, 2048),
    "activity_completion": ("inline-observation", False, 8192),
    "epoch_jobs": ("invocation", True, 65536),
    "epoch_attachment": ("inline-observation", False, 1024),
    "generation_pin": ("inline-observation", False, 4096),
    "node_runtime": ("inline-observation", False, 8192),
    "job_context": ("inline-observation", False, 4096),
    "a2_capacity": ("process", True, 65536),
    "a2_control": ("invocation", True, 4096),
    "a2_cancellation": ("invocation", True, 64),
    "a2_events": ("process", False, 65536),
}
LIFETIMES_CONSTRUCTOR_POLICY = {
    "root/owner_read": ("R", ("F6",)), "root/node_lease_read": ("R", ("F6",)),
    "root/capacity_read": ("R", ("F6",)), "root/owner_cas": ("R", ("F6",)),
    "root/restore": ("R", ("F6",)), "root/start_runtime": ("R", ("F6",)),
    "root/driver": ("R", ("F3",)), "root/primary_await_durable": ("R", ("F4",)),
    "root/primary_verify_ownership": ("R", ("F4",)), "root/aux_await_durable": ("R", ("F5",)),
    "root/aux_verify_ownership": ("R", ("F5",)), "root/hydration": ("R", ("F0",)),
    "root/sync": ("R", ("F1",)), "root/compaction": ("R", ("F2",)),
    "root/terminal_proof": ("R", ("C1",)), "root/ensure_durable": ("R", ("C0",)),
    "root/wake": ("R", ("F7", "C5")), "root/stop": ("R", ("C2",)),
    "root/release": ("R", ("C3",)), "root/adopt": ("R", ("C4",)),
    "root/local_retirement": ("R", ("C6",)), "root/cancel": ("R", ("C7",)),
    "native/db_open": ("N", ("F6",)), "native/restored_alarm": ("N", ("F6",)),
    "native/worker_build": ("N", ("F6",)), "native/hydrator_open": ("N", ("F0",)),
    "native/hydrator_step": ("N", ("F0",)), "native/sync_capture": ("N", ("F1", "C0", "C1", "C6")),
    "native/snapshot": ("N", ("C6",)), "native/local_close": ("N", ("C6",)),
    "native/worker_destroy": ("N", ("C2",)), "native/read_file": ("N", ("F2",)),
    "native/opaque_lower_job": ("N", ("F2",)), "inline/publish": ("I", ("F6",)),
    "future/storage_sync": ("R", ("F3",)), "future/gate_dispatch": ("I", ()),
}
LIFETIMES_LAYOUT_FIELDS = ("format", "claim", "arc_convention", "fixed", "constructors", "controls", "limits")
LIFETIMES_FIXED_FIELDS = ("bytes", "alignment", "arc_bytes", "bucket")
LIFETIMES_CONSTRUCTOR_FIELDS = (
    "capture_bytes", "capture_alignment", "future_bytes", "future_alignment", "result_bytes", "result_alignment",
    "channel_payload_bytes", "channel_payload_alignment", "channel_bound_bytes", "owned_capacity_bound_bytes",
    "admitted_overlap_bytes", "ceiling_bytes", "creation_limit", "lanes", "bucket",
)
LIFETIMES_CONTROL_NAMES = ("cancel", "primary_finish", "aux_finish", "retirement")
LIFETIMES_CONTROL_FIELDS = ("bytes", "alignment", "ceiling_bytes")
LIFETIMES_LIMITS = {
    "forward_roots": 8, "cleanup_roots": 8, "controls": 4, "native_children_per_root": 1,
    "maximum_wrapper_handles": 32, "cleanup_reserve": 532480, "cleanup_root_bytes": 65536,
    "cleanup_child_creation_limit": 16, "cleanup_child_wrapper_bytes": 512,
    "typed_root_payload_bytes": 49152, "cleanup_owned_capacity_bytes": 8192, "maximum_alignment": 16,
    "a2_event_capacity": 512,
}


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


def load_pins(root=ROOT, stage=None):
    if stage is not None:
        if stage not in NODE_STAGES:
            fail("unsupported source stage")
        base = load_pins(root)
        profile = {FOUNDATION: load_foundation, LIFETIMES: load_lifetimes, FRONTEND: load_frontend, OBSERVATIONS: load_observations}[stage](root, base)
        return {**base, "profile": stage, "foundation": profile}
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
    if pins["profile"] == FOUNDATION:
        return foundation_series(root, pins["foundation"])
    if pins["profile"] == LIFETIMES:
        return lifetimes_series(root, pins["foundation"])
    if pins["profile"] == FRONTEND:
        return frontend_series(root, pins["foundation"])
    if pins["profile"] == OBSERVATIONS:
        return observations_series(root, pins["foundation"])
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
    if pins["profile"] in NODE_STAGES:
        modes = {FOUNDATION: FOUNDATION_MODES, LIFETIMES: LIFETIMES_MODES, FRONTEND: FRONTEND_MODES, OBSERVATIONS: OBSERVATIONS_MODES}[pins["profile"]]
        if mode not in modes:
            fail("unsupported graph/build mode for selected node profile")
        features = modes[mode]["root_features"]
        return ["--features", "foks-celld-guarded/" + features[0]] if features else []
    if mode not in ("identity", "lifecycle", "node-baseline") or (mode != "identity" and pins["profile"] != A2):
        fail("unsupported graph/build mode for input profile")
    return ["--features", "lifecycle"] if mode == "lifecycle" else []


def stage_for_mode(mode):
    if mode in FOUNDATION_MODES:
        return FOUNDATION
    if mode in LIFETIMES_MODES:
        return LIFETIMES
    if mode in FRONTEND_MODES:
        return FRONTEND
    if mode in OBSERVATIONS_MODES:
        return OBSERVATIONS
    if mode not in ("identity", "lifecycle", "node-baseline"):
        fail("unsupported graph/build mode")
    return None


def foundation_image(value, path, maximum=2 * 1024 * 1024):
    closed_fields(value, ("path", "bytes", "sha256"), "foundation image")
    if value["path"] != path or type(value["bytes"]) is not int or not 0 < value["bytes"] <= maximum or not isinstance(value["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", value["sha256"]):
        fail("invalid foundation image identity")


def exact_json(actual, expected, label):
    # JSON equality must not confuse booleans with numeric fields.
    if json.dumps(actual, sort_keys=True, separators=(",", ":"), allow_nan=False) != json.dumps(expected, sort_keys=True, separators=(",", ":"), allow_nan=False):
        fail(label + " mismatch")


def load_foundation(root, base):
    profile = read_json(root / FOUNDATION_PROFILE)
    return validate_foundation(root, base, profile, root / FOUNDATION_SERIES)


def validate_foundation(root, base, profile, series_path):
    # Only the two fixed loaders supply this path: current B1 or preserved B1.
    # No CLI/profile value chooses which historical bytes are trusted.
    closed_fields(profile, ("format", "profile", "target", "accepted_a2_commit", "accepted_a3a_commit", "base", "pins", "node_series", "cargo_lock", "qualification"), "foundation profile")
    if type(profile["format"]) is not int or profile["format"] != 1 or profile["profile"] != FOUNDATION or profile["target"] != TARGET or profile["accepted_a2_commit"] != NODE_BASE or profile["accepted_a3a_commit"] != FOUNDATION_BASE or base["profile"] != A2:
        fail("invalid foundation profile identity")
    closed_fields(profile["base"], FOUNDATION_BASE_IMAGES, "foundation base")
    for name, (path, checksum) in FOUNDATION_BASE_IMAGES.items():
        entry = profile["base"][name]
        foundation_image(entry, path)
        if entry["sha256"] != checksum:
            fail("foundation accepted base changed")
        checked_file(root / path, entry)
    exact_json(profile["pins"], base, "foundation inherited pins")
    foundation_image(profile["node_series"], FOUNDATION_SERIES, MAXIMUM_FOUNDATION_SERIES)
    lock = profile["cargo_lock"]
    closed_fields(lock, ("before", "expected", "added_edges"), "foundation lock")
    for side in ("before", "expected"):
        foundation_image(lock[side], "Cargo.lock")
    if lock["before"]["sha256"] != FOUNDATION_LOCK_BEFORE or lock["before"] == lock["expected"]:
        fail("foundation lock preimage/postimage identity changed")
    exact_json(lock["added_edges"], [{"from": "celld", "to": name} for name in FOUNDATION_EDGES], "foundation lock edges")
    actual_lock = {"path": "Cargo.lock", "bytes": (root / "Cargo.lock").stat().st_size, "sha256": file_hash(root / "Cargo.lock")}
    if actual_lock not in (lock["before"], lock["expected"]):
        fail("foundation lock is neither reviewed preimage nor postimage")
    checked_file(root / "Cargo.lock", actual_lock)
    qualification = profile["qualification"]
    closed_fields(qualification, ("claim", "source_roster", "tree", "program", "deployments", "tests", "modes", "not_qualified"), "foundation qualification")
    if qualification["claim"] != "b1-foundation-only" or qualification["source_roster"] != list(FOUNDATION_PATHS) or qualification["not_qualified"] != FOUNDATION_EXCLUSIONS:
        fail("foundation scope changed")
    tree = qualification["tree"]
    closed_fields(tree, ("files", "payload_bytes", "inventory_sha256"), "foundation tree")
    if type(tree["files"]) is not int or tree["files"] != base["upstream"]["files"] + len(PATCH_ADDITIONS | FOUNDATION_ADDITIONS) or type(tree["payload_bytes"]) is not int or not 0 < tree["payload_bytes"] <= MAXIMUM_FOUNDATION_SOURCE or not isinstance(tree["inventory_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", tree["inventory_sha256"]):
        fail("invalid foundation tree envelope")
    closed_fields(qualification["program"], ("primary_js", "fixture_js", "wasm", "primary_metadata", "fixture_metadata"), "foundation program")
    for image in qualification["program"].values():
        validate_image(image, git=False, positive=True)
    exact_json(qualification["program"]["wasm"], base["qualification"]["program"]["wasm"], "foundation unchanged Wasm")
    closed_fields(qualification["deployments"], ("primary", "fixture"), "foundation deployments")
    for name, entry in qualification["deployments"].items():
        closed_fields(entry, ("version", "prefix"), "foundation deployment")
        if not isinstance(entry["version"], str) or not re.fullmatch(r"[0-9a-f]{16}", entry["version"]) or entry["prefix"] != "deploy/foks-guarded-node-" + name + "/" + entry["version"]:
            fail("invalid foundation deployment identity")
    exact_json(qualification["tests"], {"prefix": FOUNDATION_TEST_PREFIX, "names": FOUNDATION_TESTS, "threads": 1}, "foundation test inventory")
    exact_json(qualification["modes"], FOUNDATION_MODES, "foundation modes")
    validate_foundation_series(root, profile, series_path)
    return profile


def foundation_series(root, profile):
    return validate_foundation_series(root, profile, root / FOUNDATION_SERIES)


def validate_foundation_series(root, profile, series_path):
    checked_file(series_path, profile["node_series"])
    series = read_json(series_path, MAXIMUM_FOUNDATION_SERIES)
    closed_fields(series, ("format", "profile", "accepted_a2_commit", "accepted_a3a_commit", "base_series", "patches"), "foundation patch series")
    if type(series["format"]) is not int or series["format"] != 1 or series["profile"] != FOUNDATION or series["accepted_a2_commit"] != NODE_BASE or series["accepted_a3a_commit"] != FOUNDATION_BASE:
        fail("invalid foundation series identity")
    exact_json(series["base_series"], profile["base"]["patch_series"], "foundation base series")
    patches = series["patches"]
    if not isinstance(patches, list) or len(patches) != 2:
        fail("foundation requires exactly two patches")
    exact_json(patches[0], patch_series(root, profile["pins"])["patches"][0], "foundation first patch")
    for selected in patches:
        if not isinstance(selected, dict) or type(selected.get("bytes")) is not int or not 0 < selected["bytes"] <= MAXIMUM_FOUNDATION_PATCH:
            fail("foundation patch allowance exceeded")
    selected = patches[1]
    closed_fields(selected, ("path", "bytes", "sha256", "files"), "foundation second patch")
    if selected["path"] != FOUNDATION_PATCH or not isinstance(selected["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", selected["sha256"]):
        fail("invalid foundation patch identity")
    files = selected["files"]
    if not isinstance(files, list) or len(files) != len(FOUNDATION_PATHS):
        fail("invalid foundation patch roster")
    seen = []
    for entry in files:
        closed_fields(entry, ("path", "mode", "before", "after"), "foundation patch file")
        if entry["path"] not in FOUNDATION_PATHS or entry["path"] in seen or entry["mode"] != "100644":
            fail("invalid foundation patch path/mode")
        seen.append(entry["path"])
        if entry["path"] in FOUNDATION_ADDITIONS:
            if entry["before"] is not None:
                fail("foundation addition has a preimage")
        else:
            validate_image(entry["before"])
        validate_image(entry["after"])
    if seen != list(FOUNDATION_PATHS):
        fail("foundation patch roster is not sorted")
    checked_file(root / FOUNDATION_PATCH, selected)
    return series


def foundation_lock(root, pins):
    """Only B1 preparation may precede lock materialization; graph/build cannot."""
    if pins["profile"] in NODE_STAGES:
        return checked_file(root / "Cargo.lock", pins["foundation"]["cargo_lock"]["expected"])
    return root / "Cargo.lock"


def lifetimes_constructor_policy(rule, lanes):
    return {
        "rule": rule, "lanes": list(lanes),
        "creation_limit": {lane: "inline" if rule == "I" else "cumulative" if lane.startswith("F") else 16 if rule == "N" else 1 for lane in lanes},
        "bucket": "inline-observation" if rule == "I" else "invocation",
        "ceiling_bytes": {"R": 57344, "N": 512, "I": 8192}[rule],
    }


def lifetimes_layout_contract():
    """The closed source contract contains policy, never measured type sizes."""
    return {
        "maximum_stderr_bytes": 1024 * 1024,
        "records": {"b1": {"tag": "FOKS_B1_LAYOUT ", "maximum_bytes": 4096}, "b2": {"tag": "FOKS_B2_LAYOUT ", "maximum_bytes": 65536}},
        "record_fields": list(LIFETIMES_LAYOUT_FIELDS), "fixed_fields": list(LIFETIMES_FIXED_FIELDS),
        "constructor_fields": list(LIFETIMES_CONSTRUCTOR_FIELDS), "control_fields": list(LIFETIMES_CONTROL_FIELDS),
        "fixed_policy": {name: {"bucket": bucket, "arc": arc, "ceiling_bytes": ceiling} for name, (bucket, arc, ceiling) in LIFETIMES_FIXED_POLICY.items()},
        "constructor_policy": {name: lifetimes_constructor_policy(rule, lanes) for name, (rule, lanes) in LIFETIMES_CONSTRUCTOR_POLICY.items()},
        "control_names": list(LIFETIMES_CONTROL_NAMES), "arc_convention": "repr-c-two-atomic-usize",
        "limits": dict(LIFETIMES_LIMITS),
    }


def accepted_foundation(root):
    # Hash-bound immutable prefix: never validate B1 against the new working
    # runtime-node.lock or node-series, which intentionally identify B2.
    for path, checksum in LIFETIMES_BASE_IMAGES.values():
        image = read_json(root / path)
        if file_hash(root / path) != checksum:
            fail("accepted foundation input changed")
        if not isinstance(image, dict):
            fail("invalid accepted foundation input")
    base = load_pins(root)
    return validate_foundation(root, base, read_json(root / LIFETIMES_PROFILE), root / LIFETIMES_SERIES)


def load_lifetimes(root, base):
    return validate_lifetimes(root, base, read_json(root / FOUNDATION_PROFILE), root / FOUNDATION_SERIES)


def validate_lifetimes(root, base, profile, series_path):
    accepted = accepted_foundation(root)
    closed_fields(profile, ("format", "profile", "target", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "base", "pins", "node_series", "cargo_lock", "qualification"), "lifetimes profile")
    identity = {"format": 1, "profile": LIFETIMES, "target": TARGET, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE}
    exact_json({name: profile[name] for name in identity}, identity, "lifetimes profile identity")
    closed_fields(profile["base"], (*FOUNDATION_BASE_IMAGES, *LIFETIMES_BASE_IMAGES), "lifetimes base")
    exact_json({name: profile["base"][name] for name in FOUNDATION_BASE_IMAGES}, accepted["base"], "lifetimes historical base")
    for name, (path, checksum) in LIFETIMES_BASE_IMAGES.items():
        entry = profile["base"][name]
        foundation_image(entry, path)
        if entry["sha256"] != checksum:
            fail("lifetimes accepted base changed")
        checked_file(root / path, entry)
    exact_json(profile["pins"], base, "lifetimes inherited pins")
    exact_json(profile["cargo_lock"], accepted["cargo_lock"], "lifetimes unchanged lock contract")
    if profile["cargo_lock"]["expected"]["sha256"] != LIFETIMES_LOCK_SHA256:
        fail("lifetimes accepted lock changed")
    # Unlike B1's pre-resolution source preparation, every B2 action requires
    # the already accepted lock. The historical before-image grants no access.
    checked_file(root / "Cargo.lock", profile["cargo_lock"]["expected"])
    foundation_image(profile["node_series"], FOUNDATION_SERIES, MAXIMUM_FOUNDATION_SERIES)
    qualification = profile["qualification"]
    closed_fields(qualification, ("claim", "source_roster", "tree", "program", "deployments", "tests", "modes", "not_qualified", "layout_evidence"), "lifetimes qualification")
    exact_json({name: qualification[name] for name in ("claim", "source_roster", "not_qualified")}, {"claim": "b2-lifetimes-components-only", "source_roster": list(LIFETIMES_PATHS), "not_qualified": FOUNDATION_EXCLUSIONS}, "lifetimes scope")
    tree = qualification["tree"]
    closed_fields(tree, ("files", "payload_bytes", "inventory_sha256"), "lifetimes tree")
    if type(tree["files"]) is not int or tree["files"] != accepted["qualification"]["tree"]["files"] or type(tree["payload_bytes"]) is not int or not 0 < tree["payload_bytes"] <= MAXIMUM_FOUNDATION_SOURCE or not isinstance(tree["inventory_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", tree["inventory_sha256"]):
        fail("invalid lifetimes tree envelope")
    for name in ("program", "deployments"):
        exact_json(qualification[name], accepted["qualification"][name], "lifetimes unchanged " + name)
    exact_json(qualification["tests"], {"prefix": LIFETIMES_TEST_PREFIX, "names": LIFETIMES_TESTS, "threads": 1}, "lifetimes test inventory")
    exact_json(qualification["modes"], LIFETIMES_MODES, "lifetimes modes")
    exact_json(qualification["layout_evidence"], lifetimes_layout_contract(), "lifetimes layout policy")
    validate_lifetimes_series(root, profile, accepted, series_path)
    return profile


def lifetimes_series(root, profile, accepted=None):
    accepted = accepted_foundation(root) if accepted is None else accepted
    return validate_lifetimes_series(root, profile, accepted, root / FOUNDATION_SERIES)


def validate_lifetimes_series(root, profile, accepted, series_path):
    checked_file(series_path, profile["node_series"])
    series = read_json(series_path, MAXIMUM_FOUNDATION_SERIES)
    closed_fields(series, ("format", "profile", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "base_series", "patches"), "lifetimes patch series")
    identity = {"format": 1, "profile": LIFETIMES, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE, "base_series": accepted["base"]["patch_series"]}
    exact_json({name: series[name] for name in identity}, identity, "lifetimes series identity")
    patches = series["patches"]
    if not isinstance(patches, list) or len(patches) != 3:
        fail("lifetimes requires exactly three patches")
    prefix = validate_foundation_series(root, accepted, root / LIFETIMES_SERIES)
    exact_json(patches[:2], prefix["patches"], "lifetimes immutable patch prefix")
    selected = patches[2]
    closed_fields(selected, ("path", "bytes", "sha256", "files"), "lifetimes third patch")
    foundation_image({name: selected[name] for name in ("path", "bytes", "sha256")}, LIFETIMES_PATCH, MAXIMUM_LIFETIMES_PATCH)
    files = selected["files"]
    if not isinstance(files, list) or len(files) != len(LIFETIMES_PATHS):
        fail("invalid lifetimes patch roster")
    for path, entry in zip(LIFETIMES_PATHS, files, strict=True):
        closed_fields(entry, ("path", "mode", "before", "after"), "lifetimes patch file")
        if entry["path"] != path or entry["mode"] != "100644":
            fail("lifetimes patch path/order/mode changed")
        validate_image(entry["before"])
        validate_image(entry["after"])
    checked_file(root / LIFETIMES_PATCH, selected)
    return series


def frontend_contract():
    return {
        "scenarios": list(FRONTEND_SCENARIOS), "large_output_scenarios": list(FRONTEND_LARGE_SCENARIOS),
        "header": {"bytes": 16, "retained_bytes": 17, "magic": "FOKSGN01", "version": 1, "tenant_ordinals": [0, 1, 2], "reserved_zero_bytes": 4, "requires_write_half_eof": True},
        "commands": {"begin": {"opcode": 1, "phases": [0]}, "observe": {"opcode": 2, "phases": [0, 1], "pending_phase": 1}, "release_refused": 3},
        "response": {"magic": "FOKSGO01", "version": 1, "base_bytes": 32, "status": {"success": 0, "gated_handler_failure": 1}, "length_encoding": "u32-le", "payload_bytes": 12, "failure_payload_zero": True, "large_tail": "repeated-actual-payload", "driver_or_proof_failure_publishes": False},
        "limits": {"connections": 4, "dispatchers": 2, "slots": 2, "request_seconds": 15, "control_seconds": 1, "ordinary_frame_bytes": 32, "maximum_frame_bytes": 4 * 1024**2, "process_bytes": 1024**2, "invocation_bytes": 8 * 1024**2, "report_bytes": 128 * 1024, "encoded_report_bytes": 65536},
        "retirement_ticket": "scalar-process-registry-sequence-no-record-weak",
        "output_allocation": "one-shot-original-debt-and-a0-response-pool",
        "control_allocation": "original-control-owner-and-a0-control-pool",
    }


FRONTEND_TRANSPORT_TESTS = {
    "deadline": ["deadline::tests::" + name for name in (
        "expired_scope_rejects_new_work_and_nested_cleanup_restores_it",
        "scope_restores_context_after_unwind_and_never_extends_budget",
        "absolute_deadline_preserves_original_instant_and_expiry",
    )],
    "memory": ["memory::tests::qualification_" + name for name in (
        "output_retains_capacity_and_leaves_control_available",
        "reservations_reject_failed_owner_without_new_charges",
        "output_preserves_small_fallback_but_control_does_not",
    )],
}


def frontend_native_contract():
    return {
        "package": "celld@0.6.2", "target": TARGET, "target_kind": "lib", "artifact_profile_test": True,
        "features": list(FRONTEND_MODES[FRONTEND]["celld_features"]), "jobs": 2,
        "compile_network": "deny-all", "list_network": "deny-all",
        "compile_args": ["--no-run", "--message-format=json"],
        "list_args": [FRONTEND_TEST_PREFIX, "--list"], "run_args": [FRONTEND_TEST_PREFIX, "--test-threads=1"],
        "run_network": {"address": "127.0.0.1", "roles": list(FRONTEND_ROLES), "environment": FRONTEND_PORT_ENV.copy(), "template": FRONTEND_POLICY, "sandbox_executable": "/usr/bin/sandbox-exec", "ipv6_qualified": False},
        "transport_tests": {"package": "foks-hosted-transport", "target_kind": "lib", "feature": "qualification", "network": "deny-all", "names": {name: list(names) for name, names in FRONTEND_TRANSPORT_TESTS.items()}},
    }


FRONTEND_LAYOUT_POLICY = {
    "frontend/connection": {"count": 4, "bucket": "process", "ceiling_bytes": 65536},
    "frontend/gate_dispatch": {"count": 2, "bucket": "process", "ceiling_bytes": 8192},
    "output/send": {"count": 1, "bucket": "invocation", "ceiling_bytes": 49152},
}
FRONTEND_FIXED_LAYOUTS = ("watch_sender", "watch_receiver", "retirement_observation", "public_request_guard")
FRONTEND_WATCH_COMPONENTS = ("notify", "rwlock_bool", "atomic_usize")
FRONTEND_CONSTRUCTOR_FIELDS = ("capture_bytes", "capture_align", "future_bytes", "future_align", "result_bytes", "result_align", "typed_bytes")
FRONTEND_PROCESS_PRECHARGE = {"frontend": 262144, "dispatcher": 16384, "report": 131072, "serialization": 65536}
FRONTEND_LAYOUT_FIELDS = ("format", "claim", "constructors", "fixed", "watch_components", "watch_derived_bytes", "watch_allocation_bound", "frame_bytes", "process_precharge", "original_invocation_limit")


def frontend_layout_contract():
    return {
        "prefix": "FOKS_B3_LAYOUT ", "maximum_record_bytes": 16384, "maximum_stderr_bytes": 1048576,
        "record_fields": list(FRONTEND_LAYOUT_FIELDS), "claim": "b3-frontend-layouts-only",
        "constructor_fields": list(FRONTEND_CONSTRUCTOR_FIELDS), "constructor_policy": {name: policy.copy() for name, policy in FRONTEND_LAYOUT_POLICY.items()},
        "fixed_names": list(FRONTEND_FIXED_LAYOUTS), "fixed_fields": ["bytes", "align"], "fixed_ceiling_bytes": 4096,
        "watch_components": list(FRONTEND_WATCH_COMPONENTS), "watch_formula": "9*round16(notify)+round16(rwlock_bool)+6*round16(atomic_usize)",
        "watch_allocation_bound": 4096, "frame_bytes": [32, 4194304], "process_precharge": FRONTEND_PROCESS_PRECHARGE.copy(),
        "original_invocation_limit": 8388608,
    }


def accepted_lifetimes(root):
    # Preserved B2 is validated against its own exact profile and series. The
    # mutable B3 profile cannot relabel a historical input or native receipt.
    for path, checksum in FRONTEND_BASE_IMAGES.values():
        value = read_json(root / path)
        if file_hash(root / path) != checksum or not isinstance(value, dict):
            fail("accepted lifetimes input changed")
    return validate_lifetimes(root, load_pins(root), read_json(root / FRONTEND_PROFILE), root / FRONTEND_SERIES)


def load_frontend(root, base):
    return validate_frontend(root, base, read_json(root / FOUNDATION_PROFILE), root / FOUNDATION_SERIES)


def validate_frontend(root, base, profile, series_path):
    accepted = accepted_lifetimes(root)
    closed_fields(profile, ("format", "profile", "target", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "accepted_b2_commit", "base", "pins", "node_series", "cargo_lock", "qualification"), "frontend profile")
    identity = {"format": 1, "profile": FRONTEND, "target": TARGET, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE, "accepted_b2_commit": FRONTEND_BASE}
    exact_json({name: profile[name] for name in identity}, identity, "frontend profile identity")
    closed_fields(profile["base"], (*FOUNDATION_BASE_IMAGES, *LIFETIMES_BASE_IMAGES, *FRONTEND_BASE_IMAGES), "frontend base")
    exact_json({name: profile["base"][name] for name in accepted["base"]}, accepted["base"], "frontend historical base")
    for name, (path, checksum) in FRONTEND_BASE_IMAGES.items():
        image = profile["base"][name]
        foundation_image(image, path)
        if image["sha256"] != checksum:
            fail("frontend accepted base changed")
        checked_file(root / path, image)
    exact_json(profile["pins"], base, "frontend inherited pins")
    exact_json(profile["cargo_lock"], accepted["cargo_lock"], "frontend unchanged lock contract")
    checked_file(root / "Cargo.lock", profile["cargo_lock"]["expected"])
    foundation_image(profile["node_series"], FOUNDATION_SERIES, MAXIMUM_FOUNDATION_SERIES)
    qualification = profile["qualification"]
    closed_fields(qualification, ("claim", "source_roster", "tree", "program", "deployments", "tests", "modes", "not_qualified", "layout_evidence", "frontend", "native_execution", "frontend_layout_evidence"), "frontend qualification")
    exact_json({name: qualification[name] for name in ("claim", "source_roster", "not_qualified")}, {"claim": "b3-frontend-components-only", "source_roster": list(FRONTEND_PATHS), "not_qualified": FRONTEND_EXCLUSIONS}, "frontend scope")
    tree = qualification["tree"]
    closed_fields(tree, ("files", "payload_bytes", "inventory_sha256"), "frontend tree")
    if type(tree["files"]) is not int or tree["files"] != accepted["qualification"]["tree"]["files"] + len(FRONTEND_ADDITIONS) or type(tree["payload_bytes"]) is not int or not 0 < tree["payload_bytes"] <= MAXIMUM_FOUNDATION_SOURCE or not isinstance(tree["inventory_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", tree["inventory_sha256"]):
        fail("invalid frontend tree envelope")
    for name in ("program", "deployments"):
        exact_json(qualification[name], accepted["qualification"][name], "frontend unchanged " + name)
    exact_json(qualification["tests"], {"prefix": FRONTEND_TEST_PREFIX, "names": FRONTEND_TESTS, "threads": 1}, "frontend test inventory")
    exact_json(qualification["modes"], FRONTEND_MODES, "frontend modes")
    exact_json(qualification["layout_evidence"], lifetimes_layout_contract(), "frontend retained B2 layout policy")
    exact_json(qualification["frontend"], frontend_contract(), "frontend protocol policy")
    exact_json(qualification["native_execution"], frontend_native_contract(), "frontend native test policy")
    exact_json(qualification["frontend_layout_evidence"], frontend_layout_contract(), "frontend layout policy")
    validate_frontend_series(root, profile, accepted, series_path)
    return profile


def frontend_series(root, profile, accepted=None):
    accepted = accepted_lifetimes(root) if accepted is None else accepted
    return validate_frontend_series(root, profile, accepted, root / FOUNDATION_SERIES)


def validate_frontend_series(root, profile, accepted, series_path):
    checked_file(series_path, profile["node_series"])
    series = read_json(series_path, MAXIMUM_FOUNDATION_SERIES)
    closed_fields(series, ("format", "profile", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "accepted_b2_commit", "base_series", "patches"), "frontend patch series")
    identity = {"format": 1, "profile": FRONTEND, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE, "accepted_b2_commit": FRONTEND_BASE, "base_series": accepted["base"]["patch_series"]}
    exact_json({name: series[name] for name in identity}, identity, "frontend series identity")
    patches = series["patches"]
    if not isinstance(patches, list) or len(patches) != 4:
        fail("frontend requires exactly four patches")
    prefix = validate_lifetimes_series(root, accepted, accepted_foundation(root), root / FRONTEND_SERIES)
    exact_json(patches[:3], prefix["patches"], "frontend immutable patch prefix")
    selected = patches[3]
    closed_fields(selected, ("path", "bytes", "sha256", "files"), "frontend fourth patch")
    foundation_image({name: selected[name] for name in ("path", "bytes", "sha256")}, FRONTEND_PATCH, MAXIMUM_FOUNDATION_PATCH)
    files = selected["files"]
    if not isinstance(files, list) or len(files) != len(FRONTEND_PATHS) or not set(FRONTEND_PATHS) <= set(FRONTEND_UNION):
        fail("invalid frontend patch roster")
    for path, entry in zip(FRONTEND_PATHS, files, strict=True):
        closed_fields(entry, ("path", "mode", "before", "after"), "frontend patch file")
        if entry["path"] != path or entry["mode"] != "100644":
            fail("frontend patch path/order/mode changed")
        if path in FRONTEND_ADDITIONS:
            if entry["before"] is not None:
                fail("frontend addition has a preimage")
        else:
            validate_image(entry["before"])
        validate_image(entry["after"])
    checked_file(root / FRONTEND_PATCH, selected)
    return series


# Exact P1 policy is separate from the preserved B3 frontend contract.
def observations_contract():
    return {'format': 1,
     'observation_fields': ['sequence',
                            'a2_generation',
                            'epoch',
                            'generation',
                            'requests',
                            'durable_ops',
                            'ownership_ops',
                            'positions',
                            'durable_positions',
                            'seen',
                            'first',
                            'tail',
                            'ordinal',
                            'native_joined',
                            'native_disposed',
                            'roots_joined',
                            'roots_disposed',
                            'frame_bytes',
                            'accepted_bytes',
                            'positive_pending_bytes',
                            'write_polls',
                            'pending_polls',
                            'tenant',
                            'bank_slot',
                            'a2_slot',
                            'phase',
                            'summary',
                            'request_dead',
                            'public_guard',
                            'durable_verdicts',
                            'ownership_verdicts',
                            'proof_sources',
                            'release_verdicts',
                            'c1',
                            'outcome',
                            'refusal'],
     'observe_request_v2': {'length': 16,
                            'magic': 'FOKSGN01',
                            'opcode': 2,
                            'opcode_offset': 9,
                            'reserved_zero': [12, 13, 14, 15],
                            'selector1': 'same existing backpressure scenario restriction',
                            'selector_offset': 11,
                            'tenant_offset': 10,
                            'v1': 'exact previous requests and response behavior preserved; v2 Begin refuses',
                            'version': 2,
                            'version_offset': 8},
     'projection': {'0': 'Admitted',
                    '1': 'PrimaryLocal',
                    '2': 'AuxLocal',
                    '3': 'PrimaryDurableSuccess',
                    '4': 'AuxDurableSuccess',
                    '5': 'PrimaryOwnershipSuccess',
                    '6': 'AuxOwnershipSuccess',
                    '7': 'PrimaryOutputReleased',
                    '8': 'AuxOutputReleased',
                    '9': 'DriverJoined',
                    '10': 'DriverDelivered',
                    '11': 'C1Required',
                    '12': 'C1Succeeded',
                    '13': 'C1NotRequired',
                    '14': 'ZeroWritePending',
                    '15': 'PositiveWritePending',
                    '16': 'SocketClosed',
                    '17': 'FrameDropped',
                    '18': 'AnswerDropped',
                    '19': 'PrimaryFinishProcessed',
                    '20': 'AuxFinishProcessed',
                    '21': 'PrimaryInactive',
                    '22': 'AuxInactive',
                    '23': 'PrimaryFinishDisposed',
                    '24': 'AuxFinishDisposed',
                    '25': 'WorkerDestroyed',
                    '26': 'C6Succeeded',
                    '27': 'OriginalRequestDropped',
                    '28': 'A2Free',
                    '29': 'PublicGuardDropped',
                    '30': 'BankFree',
                    '31': 'Quarantined'},
     'phase': {'0': 'NeverAdmitted',
               '1': 'Active',
               '2': 'Retiring',
               '3': 'PendingOwner',
               '4': 'Quarantined',
               '5': 'Free'},
     'milestones': {'0': 'Admitted',
                    '1': 'PrimaryLocal',
                    '10': 'AuxOutputReleased',
                    '11': 'DriverJoined',
                    '12': 'DriverDelivered',
                    '13': 'C1Required',
                    '14': 'C1Succeeded',
                    '15': 'C1NotRequired',
                    '16': 'ZeroWritePending',
                    '17': 'PositiveWritePending',
                    '18': 'SocketClosed',
                    '19': 'FrameDropped',
                    '2': 'AuxLocal',
                    '20': 'AnswerDropped',
                    '21': 'PrimaryFinishProcessed',
                    '22': 'AuxFinishProcessed',
                    '23': 'PrimaryInactive',
                    '24': 'AuxInactive',
                    '25': 'PrimaryFinishDisposed',
                    '26': 'AuxFinishDisposed',
                    '27': 'AuxSyncDisposed',
                    '28': 'DispatcherDisposed',
                    '29': 'WorkerDestroyed',
                    '3': 'PrimaryOutputSubmitted',
                    '30': 'C6Succeeded',
                    '31': 'TerminalConjunction',
                    '32': 'PayloadsDropped',
                    '33': 'OriginalRequestDropped',
                    '34': 'A2Free',
                    '35': 'PublicGuardDropped',
                    '36': 'BankFree',
                    '37': 'Quarantined',
                    '38': 'CancelProcessed',
                    '39': 'Retiring',
                    '4': 'AuxOutputSubmitted',
                    '5': 'PrimaryDurableSuccess',
                    '6': 'AuxDurableSuccess',
                    '7': 'PrimaryOwnershipSuccess',
                    '8': 'AuxOwnershipSuccess',
                    '9': 'PrimaryOutputReleased'},
     'native_indices': {'0': 'DbOpen',
                        '1': 'RestoredAlarm',
                        '10': 'OpaqueLowerJob',
                        '2': 'HydratorOpen',
                        '3': 'HydratorStep',
                        '4': 'SyncCapture',
                        '5': 'Snapshot',
                        '6': 'WorkerBuild',
                        '7': 'WorkerDestroy',
                        '8': 'LocalClose',
                        '9': 'ReadFile'},
     'c1': {'failed': 4, 'not_required': 1, 'required': 2, 'succeeded': 3, 'unseen': 0},
     'outcome': {'local_write_flush_ok': 1, 'transport_failed': 2, 'unseen': 0},
     'verdict': {'error': 2, 'ok': 1, 'unseen': 0},
     'proof_source': {'bucket': 2, 'fleet': 1, 'unseen': 0},
     'refusal': '0 means none; current closed Refusal discriminant+1. Never interpreted as successful disposal.',
     'structured_stream': {'atomic_snapshot': False,
                           'chunks_max': 8,
                           'encoder_capacity': 65536,
                           'end': 'guarded-observation-end',
                           'event_array': '64 Option<Event> entries; first count entries present, rest null; '
                                          'original A2 Event tuples/enum names unchanged.',
                           'events': 'guarded-a2-events',
                           'events_max': 16384,
                           'events_per_chunk': 64,
                           'events_total': 512,
                           'header': 'guarded-observation-header',
                           'header_end_max': 1024,
                           'item': 'guarded-invocation',
                           'item_max': 2048,
                           'items_max': 64,
                           'native_report_max': 4096,
                           'stream_max': 264192},
     'archive': {'entries': 64,
                 'entry_native_bytes_max': 512,
                 'native_bytes_max': 32768,
                 'copy_only': True,
                 'overwrites': False},
     'observe_response_v2': {'length': 32,
                             'magic': 'FOKSGS01',
                             'version': 2,
                             'version_offset': 8,
                             'tenant_offset': 9,
                             'selector_offset': 10,
                             'phase_offset': 11,
                             'sequence': {'offset': 12, 'bytes': 8, 'encoding': 'little-endian'},
                             'positive_pending_bytes': {'offset': 20, 'bytes': 4, 'encoding': 'little-endian'},
                             'frame_bytes': {'offset': 24, 'bytes': 4, 'encoding': 'little-endian'},
                             'projection': {'offset': 28, 'bytes': 4, 'encoding': 'little-endian'}},
     'field_semantics': {
         'public_guard': 'Immutable admission provenance: this invocation was admitted with a PublicRequestGuard. This is not a current-live assertion; actual destruction is recorded only by PublicGuardDropped and its first ordinal.',
         'request_dead': 'Current read-time summary from the existing Weak<Request> witness; false does not prove activity and true does not establish A2 Free, public-guard destruction, or external reuse.',
         'auxiliary_request': 'Auxiliary fields describe the current actual auxiliary request. On replacement, requests[1] changes; durable_ops[1], ownership_ops[1], positions[1], durable_positions[1], durable_verdicts[1], ownership_verdicts[1], proof_sources[1] and release_verdicts[1] reset. The AuxLocal, AuxOutputSubmitted, AuxDurableSuccess, AuxOwnershipSuccess, AuxOutputReleased, AuxFinishProcessed, AuxInactive, AuxFinishDisposed and AuxSyncDisposed bits and first ordinals reset; the global ordinal remains monotonic. Native/root disposition counts remain invocation-wide.',
         'disposal_masks': 'roots_disposed records concrete task/channel/result/abandon payload disposal and bookkeeping closure; native_disposed records concrete native child payload disposal. Notification Wakers and remaining context aliases are excluded. Only OriginalRequestDropped and the existing Weak<Request> summary witness final owner-alias destruction before A2Free.'
     },
     'release_order': ['OriginalRequestDropped', 'A2Free', 'PublicGuardDropped', 'BankFree']}


def observation_layout_contract():
    return {'prefix': 'FOKS_B4_OBSERVATION_LAYOUT=',
     'maximum_record_bytes': 4096,
     'maximum_stderr_bytes': 1048576,
     'record_fields': ['format', 'layouts', 'retained_bytes', 'a2_event_bytes'],
     'layout_ids': ['invocation', 'archive', 'held', 'a2_chunk', 'report', 'write_progress', 'registry_arc'],
     'layout_fields': ['bytes', 'alignment'],
     'layout_policy': {'invocation': {'ceiling_bytes': 512, 'maximum_alignment': 8},
                       'archive': {'ceiling_bytes': 32768, 'maximum_alignment': 8},
                       'held': {'ceiling_bytes': 1024, 'maximum_alignment': 16},
                       'a2_chunk': {'ceiling_bytes': 4096, 'maximum_alignment': 16},
                       'report': {'ceiling_bytes': 4096, 'maximum_alignment': 16},
                       'write_progress': {'ceiling_bytes': 64, 'maximum_alignment': 16},
                       'registry_arc': {'ceiling_bytes': 65536, 'maximum_alignment': 16}},
     'archive_entries': 64,
     'retained_bytes_maximum': 131072,
     'retained_formula': 'archive.bytes + 65536 + a2_chunk.bytes + 6*invocation.bytes + report.bytes +1024 '
                         '+a2_event_bytes',
     'a2_event_bytes': {'capacity': 512, 'ceiling_bytes': 65536, 'same_run': 'B2.fixed.a2_events.bytes'},
     'registry_arc': 'same-run B2.fixed.effect_registry.arc_bytes'}


def accepted_frontend(root):
    # Historical B3 uses its own immutable profile/series even while the current
    # outer profile identifies P1. Never infer the predecessor from that file.
    for path, checksum in OBSERVATIONS_BASE_IMAGES.values():
        value = read_json(root / path)
        if file_hash(root / path) != checksum or not isinstance(value, dict):
            fail("accepted frontend input changed")
    return validate_frontend(root, load_pins(root), read_json(root / OBSERVATIONS_PROFILE), root / OBSERVATIONS_SERIES)


def load_observations(root, base):
    accepted = accepted_frontend(root)
    profile = read_json(root / FOUNDATION_PROFILE)
    closed_fields(profile, ("format", "profile", "target", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "accepted_b2_commit", "accepted_b3_commit", "base", "pins", "node_series", "cargo_lock", "qualification"), "observations profile")
    identity = {"format": 1, "profile": OBSERVATIONS, "target": TARGET, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE, "accepted_b2_commit": FRONTEND_BASE, "accepted_b3_commit": OBSERVATIONS_BASE}
    exact_json({name: profile[name] for name in identity}, identity, "observations profile identity")
    closed_fields(profile["base"], (*FOUNDATION_BASE_IMAGES, *LIFETIMES_BASE_IMAGES, *FRONTEND_BASE_IMAGES, *OBSERVATIONS_BASE_IMAGES), "observations base")
    exact_json({name: profile["base"][name] for name in accepted["base"]}, accepted["base"], "observations historical base")
    for name, (path, checksum) in OBSERVATIONS_BASE_IMAGES.items():
        image = profile["base"][name]
        foundation_image(image, path)
        if image["sha256"] != checksum:
            fail("observations accepted base changed")
        checked_file(root / path, image)
    exact_json(profile["pins"], base, "observations inherited pins")
    exact_json(profile["cargo_lock"], accepted["cargo_lock"], "observations unchanged lock contract")
    checked_file(root / "Cargo.lock", profile["cargo_lock"]["expected"])
    foundation_image(profile["node_series"], FOUNDATION_SERIES, MAXIMUM_FOUNDATION_SERIES)
    qualification = profile["qualification"]
    closed_fields(qualification, ("claim", "source_roster", "tree", "program", "deployments", "tests", "modes", "not_qualified", "layout_evidence", "frontend", "native_execution", "frontend_layout_evidence", "observations", "observation_layout_evidence"), "observations qualification")
    exact_json({name: qualification[name] for name in ("claim", "source_roster", "not_qualified")}, {"claim": "b4-p1-observation-components-only", "source_roster": list(OBSERVATIONS_PATHS), "not_qualified": OBSERVATIONS_EXCLUSIONS}, "observations scope")
    tree = qualification["tree"]
    closed_fields(tree, ("files", "payload_bytes", "inventory_sha256"), "observations tree")
    if type(tree["files"]) is not int or tree["files"] != accepted["qualification"]["tree"]["files"] or type(tree["payload_bytes"]) is not int or not 0 < tree["payload_bytes"] <= MAXIMUM_FOUNDATION_SOURCE or not isinstance(tree["inventory_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", tree["inventory_sha256"]):
        fail("invalid observations tree envelope")
    for name in ("program", "deployments", "layout_evidence", "frontend", "native_execution", "frontend_layout_evidence"):
        exact_json(qualification[name], accepted["qualification"][name], "observations unchanged " + name)
    exact_json(qualification["tests"], {"prefix": FRONTEND_TEST_PREFIX, "names": OBSERVATIONS_TESTS, "threads": 1}, "observations test inventory")
    exact_json(qualification["modes"], OBSERVATIONS_MODES, "observations modes")
    exact_json(qualification["observations"], observations_contract(), "observations policy")
    exact_json(qualification["observation_layout_evidence"], observation_layout_contract(), "observations layout policy")
    observations_series(root, profile, accepted)
    return profile


def observations_series(root, profile, accepted=None):
    accepted = accepted_frontend(root) if accepted is None else accepted
    checked_file(root / FOUNDATION_SERIES, profile["node_series"])
    series = read_json(root / FOUNDATION_SERIES, MAXIMUM_FOUNDATION_SERIES)
    closed_fields(series, ("format", "profile", "accepted_a2_commit", "accepted_a3a_commit", "accepted_b1_commit", "accepted_b2_commit", "accepted_b3_commit", "base_series", "patches"), "observations patch series")
    identity = {"format": 1, "profile": OBSERVATIONS, "accepted_a2_commit": NODE_BASE, "accepted_a3a_commit": FOUNDATION_BASE, "accepted_b1_commit": LIFETIMES_BASE, "accepted_b2_commit": FRONTEND_BASE, "accepted_b3_commit": OBSERVATIONS_BASE, "base_series": accepted["base"]["patch_series"]}
    exact_json({name: series[name] for name in identity}, identity, "observations series identity")
    patches = series["patches"]
    if not isinstance(patches, list) or len(patches) != 5:
        fail("observations requires exactly five patches")
    prefix = validate_frontend_series(root, accepted, accepted_lifetimes(root), root / OBSERVATIONS_SERIES)
    exact_json(patches[:4], prefix["patches"], "observations immutable patch prefix")
    selected = patches[4]
    closed_fields(selected, ("path", "bytes", "sha256", "files"), "observations fifth patch")
    foundation_image({name: selected[name] for name in ("path", "bytes", "sha256")}, OBSERVATIONS_PATCH, MAXIMUM_FOUNDATION_PATCH)
    if sum(entry["bytes"] for entry in patches) > MAXIMUM_OBSERVATIONS_PATCHES:
        fail("observations combined patch payload exceeds allowance")
    files = selected["files"]
    if not isinstance(files, list) or len(files) != len(OBSERVATIONS_PATHS) or not set(OBSERVATIONS_PATHS) <= set(FRONTEND_UNION):
        fail("invalid observations patch roster")
    for path, entry in zip(OBSERVATIONS_PATHS, files, strict=True):
        closed_fields(entry, ("path", "mode", "before", "after"), "observations patch file")
        if entry["path"] != path or entry["mode"] != "100644":
            fail("observations patch path/order/mode changed")
        validate_image(entry["before"])
        validate_image(entry["after"])
    checked_file(root / OBSERVATIONS_PATCH, selected)
    return series


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
    parsed = []
    current = upstream
    for selected in patch_series(root, pins)["patches"]:
        changes = parse_one_patch(root, selected, current)
        parsed.extend(changes)
        current = postimage_inventory(current, changes)
    if pins["profile"] in NODE_STAGES:
        files = [row for row in current.values() if row["type"] == "blob"]
        tree = pins["foundation"]["qualification"]["tree"]
        if len(files) != tree["files"] or sum(row["size"] for row in files) != tree["payload_bytes"]:
            fail("foundation reconstructed source payload envelope mismatch")
    return parsed


def parse_one_patch(root, selected, upstream):
    data = checked_file(root / selected["path"], selected).read_bytes()
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
                fail("patch addition already exists at this stage")
            before_header = b"--- /dev/null\n"
        else:
            if original is None or original["type"] != "blob" or original["mode"] != entry["mode"] or original["size"] != entry["before"]["bytes"] or original["sha"] != entry["before"]["git_sha1"]:
                fail("patch preimage does not identify the immutable preceding blob")
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
            elif entry["before"] is None:
                expected[parent] = {"path": parent, "mode": "040000", "type": "tree"}
            else:
                fail("patch changes an absent parent")
            parent = parent.rpartition("/")[0]
    return expected


def source_manifest(root, pins, files, prepared):
    value = {"format": 1, "input_sha256": file_hash(root / "runtime.lock.json"), "archive_sha256": pins["upstream"]["archive_sha256"], "tree": pins["upstream"]["tree"], "patch_series_sha256": pins["patch_series_sha256"], "files": sorted(files, key=lambda item: item["path"])}
    if pins["profile"] == A2:
        value.update(format=2, profile=A2, kind="patched-source" if prepared else "upstream-archive-only")
    elif pins["profile"] in NODE_STAGES:
        value.update(format=3, profile=pins["profile"], kind="patched-source" if prepared else "upstream-archive-only", input_sha256=file_hash(root / FOUNDATION_PROFILE), base_input_sha256=file_hash(root / "runtime.lock.json"), patch_series_sha256=pins["foundation"]["node_series"]["sha256"])
        if prepared:
            actual = {"files": len(value["files"]), "payload_bytes": sum(row["bytes"] for row in value["files"]), "inventory_sha256": digest(json.dumps(value["files"], sort_keys=True, separators=(",", ":")).encode())}
            if actual != pins["foundation"]["qualification"]["tree"]:
                fail("foundation full source inventory mismatch")
    return value


def apply_patch_set(staging, patches):
    # Validate every complete preimage/postimage before writing any changed file.
    # A failed write still remains in the disposable private staging directory.
    outputs = {}
    additions = set()
    for entry, hunks in patches:
        path = staging / entry["path"]
        if entry["before"] is None:
            if path in outputs or path.exists() or path.is_symlink():
                fail("patch addition already exists while applying")
            before = b""
            additions.add(path)
        else:
            before = outputs[path] if path in outputs else path.read_bytes()
        if entry["before"] is not None and image_identity(before) != entry["before"]:
            fail("patch full preimage mismatch")
        after = transform_image(before, hunks)
        if image_identity(after) != entry["after"]:
            fail("patch full postimage mismatch")
        outputs[path] = after
    for path, after in outputs.items():
        missing = []
        parent = path.parent
        while not parent.exists():
            missing.append(parent)
            parent = parent.parent
        path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        for parent in missing:
            parent.chmod(0o755)
        with path.open("xb" if path in additions else "wb") as stream:
            stream.write(after)
        path.chmod(0o644)


def inspect_archive(archive, root=ROOT, destination=None, stage=None):
    pins = load_pins(root, stage)
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
    changed = {}
    for entry, hunks in patches:
        changed.setdefault(entry["path"], []).append((entry, hunks))
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
                if identity != changed[name][-1][0]["after"]:
                    fail("prepared patch postimage mismatch")
                after = path.read_bytes()
                if image_identity(after) != identity:
                    fail("prepared patch changed while reading")
                for change, hunks in reversed(changed[name]):
                    if image_identity(after) != change["after"]:
                        fail("prepared patch intermediate postimage mismatch")
                    before = transform_image(after, hunks, reverse=True)
                    if (change["before"] is None and before) or (change["before"] is not None and image_identity(before) != change["before"]):
                        fail("prepared patch does not reconstruct its upstream preimage")
                    if transform_image(before, hunks) != after:
                        fail("prepared patch roundtrip mismatch")
                    after = before
            files.append({"path": name, "mode": entry["mode"], **identity})
    if seen != set(expected):
        fail("prepared source roster mismatch")
    if file_hash(source / "Cargo.lock") != pins["upstream"]["cargo_lock_sha256"]:
        fail("upstream lockfile changed")
    return source_manifest(root, pins, files, prepared=True)


def verify_source(root=ROOT, stage=None):
    pins = load_pins(root, stage)
    upstream = inventory(root, pins)
    patches = parse_patch(root, pins, upstream)
    expected = postimage_inventory(upstream, patches)
    runtime = runtime_directory(root)
    recorded = read_json(runtime / "source-manifest.json")
    actual = inspect_source_tree(runtime / "source", root, pins, expected, patches)
    if recorded != actual:
        fail("prepared source manifest mismatch")
    return actual


def prepare(archive, root=ROOT, stage=None):
    runtime = runtime_directory(root)
    destination = runtime / "source"
    if destination.exists() or destination.is_symlink():
        return verify_source(root, stage)
    if (runtime / "source-manifest.json").exists():
        fail("orphan prepared manifest requires inspection")
    staging = Path(tempfile.mkdtemp(prefix="source-staging-", dir=runtime))
    try:
        original = inspect_archive(archive, root, staging, stage)
        pins = load_pins(root, stage)
        if file_hash(root / (FOUNDATION_PROFILE if stage in NODE_STAGES else "runtime.lock.json")) != original["input_sha256"]:
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
    return verify_source(root, stage)


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


def prepare_assets(archive, binding, root=ROOT, stage=None):
    if stage is not None:
        pins = load_pins(root, stage)
        base = prepare_assets(archive, binding, root)
        expected = foundation_assets(root, pins, base)
        path = runtime_directory(root) / ("assets-" + pins["profile"] + ".json")
        if not path.exists() and not path.is_symlink():
            write_json(path, expected)
        return verify_assets(root, stage)
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


def foundation_assets(root, pins, base):
    if pins["profile"] in (FRONTEND, OBSERVATIONS):
        accepted = accepted_lifetimes(root)
        foundation = accepted_foundation(root)
        historical_foundation = {"format": 1, "profile": FOUNDATION, "input_sha256": file_hash(root / LIFETIMES_PROFILE), "patch_series_sha256": foundation["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "assets": base}
        historical = {"format": 1, "profile": LIFETIMES, "input_sha256": file_hash(root / FRONTEND_PROFILE), "patch_series_sha256": accepted["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "foundation_receipt_sha256": LIFETIMES_ASSET_SHA256, "assets": base}
        for preserved, live, checksum, expected in ((LIFETIMES_ASSETS, "runtime/assets-node-foundation.json", LIFETIMES_ASSET_SHA256, historical_foundation), (FRONTEND_ASSETS, "runtime/assets-node-lifetimes.json", FRONTEND_ASSET_SHA256, historical)):
            if file_hash(root / preserved) != checksum:
                fail("accepted component asset receipt changed")
            exact_json(read_json(root / preserved), expected, "accepted component assets")
            checked_file(root / live, {"bytes": (root / preserved).stat().st_size, "sha256": checksum})
        frontend = pins["foundation"] if pins["profile"] == FRONTEND else accepted_frontend(root)
        frontend_path = FOUNDATION_PROFILE if pins["profile"] == FRONTEND else OBSERVATIONS_PROFILE
        historical_frontend = {"format": 1, "profile": FRONTEND, "input_sha256": file_hash(root / frontend_path), "patch_series_sha256": frontend["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "foundation_receipt_sha256": LIFETIMES_ASSET_SHA256, "lifetimes_receipt_sha256": FRONTEND_ASSET_SHA256, "assets": base}
        if pins["profile"] == FRONTEND:
            return historical_frontend
        if file_hash(root / OBSERVATIONS_ASSETS) != OBSERVATIONS_ASSET_SHA256:
            fail("accepted frontend asset receipt changed")
        exact_json(read_json(root / OBSERVATIONS_ASSETS), historical_frontend, "accepted frontend assets")
        checked_file(root / "runtime/assets-node-qualification.json", {"bytes": (root / OBSERVATIONS_ASSETS).stat().st_size, "sha256": OBSERVATIONS_ASSET_SHA256})
        return {"format": 1, "profile": OBSERVATIONS, "input_sha256": file_hash(root / FOUNDATION_PROFILE), "patch_series_sha256": pins["foundation"]["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "foundation_receipt_sha256": LIFETIMES_ASSET_SHA256, "lifetimes_receipt_sha256": FRONTEND_ASSET_SHA256, "frontend_receipt_sha256": OBSERVATIONS_ASSET_SHA256, "assets": base}
    if pins["profile"] == LIFETIMES:
        accepted = accepted_foundation(root)
        historical = {"format": 1, "profile": FOUNDATION, "input_sha256": file_hash(root / LIFETIMES_PROFILE), "patch_series_sha256": accepted["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "assets": base}
        preserved = read_json(root / LIFETIMES_ASSETS)
        if file_hash(root / LIFETIMES_ASSETS) != LIFETIMES_ASSET_SHA256:
            fail("accepted foundation asset receipt changed")
        exact_json(preserved, historical, "accepted foundation assets")
        # The original receipt remains at its original path too. Native
        # archives are reused without copying or relabeling historical proof.
        checked_file(root / "runtime/assets-node-foundation.json", {"bytes": (root / LIFETIMES_ASSETS).stat().st_size, "sha256": LIFETIMES_ASSET_SHA256})
        return {"format": 1, "profile": LIFETIMES, "input_sha256": file_hash(root / FOUNDATION_PROFILE), "patch_series_sha256": pins["foundation"]["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "foundation_receipt_sha256": LIFETIMES_ASSET_SHA256, "assets": base}
    return {"format": 1, "profile": FOUNDATION, "input_sha256": file_hash(root / FOUNDATION_PROFILE), "patch_series_sha256": pins["foundation"]["node_series"]["sha256"], "base_receipt_sha256": file_hash(root / "runtime/artifacts/assets.json"), "assets": base}


def verify_assets(root=ROOT, stage=None):
    if stage is not None:
        pins = load_pins(root, stage)
        expected = foundation_assets(root, pins, verify_assets(root))
        exact_json(read_json(root / ("runtime/assets-" + pins["profile"] + ".json")), expected, "foundation native asset receipt")
        return expected
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


def upstream_edges(metadata, reached, root=ROOT, mode="identity"):
    """An allowed FOKS duplicate never replaces a selected upstream dependency."""
    before = lock_inventory(root / "runtime/source/Cargo.lock")
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    allowed = set()
    if stage_for_mode(mode) in NODE_STAGES:
        pins = load_pins(root, stage_for_mode(mode))
        selected_mode(pins, mode)
        foundation_lock(root, pins)
        # The graph's independent declaration validator also checks optional
        # features/renames. This exception never applies to an unrelated local
        # edge, package, mode, registry identity or upstream dependency.
        for name in NODE_MODES[mode]["celld_foks_edges"]:
            allowed.add((name, "0.4.0", None))
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
            added = actual in allowed and key == ("celld", "0.6.2", None)
            if added:
                if package["manifest_path"] != str(root / "runtime/source/crates/celld/Cargo.toml") or target["manifest_path"] != str(root.parents[1] / "crates" / target["name"] / "Cargo.toml") or dependency["dep_kinds"] != [{"kind": None, "target": None}]:
                    fail("foundation added edge is not the exact local normal dependency")
            if actual not in expected and not added:
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


def run_owned(argv, cwd, environment, directory, name, slot, timeout, monitor=None, term_grace=3, kill_grace=3, maximum_log_bytes=128 * 1024 * 1024, expected_exit_code=0):
    """One process group, direct argv, bounded wait and durable failure evidence."""
    directory = Path(directory)
    if type(maximum_log_bytes) is not int or not 0 < maximum_log_bytes <= 128 * 1024 * 1024:
        fail("invalid command log allowance")
    if type(expected_exit_code) is not int or expected_exit_code not in (0, 1):
        fail("invalid expected command exit code")
    directory.mkdir(parents=True, exist_ok=True)
    record = {"argv": [str(arg) for arg in argv], "cwd": str(cwd), "timeout_seconds": timeout, "events": [], "cleanup_established": False}
    if expected_exit_code:
        record["expected_exit_code"] = expected_exit_code
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
                if stdout.tell() + stderr.tell() > maximum_log_bytes:
                    record["reason"] = "command log allowance exceeded"
                    break
                status = process.poll()
                if status is not None:
                    if not group_alive(process.pid):
                        record["reason"] = "completed" if status == expected_exit_code else "command failed"
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


def closed_fields(value, names, label):
    if not isinstance(value, dict) or set(value) != set(names):
        fail("invalid " + label + " fields")


def node_json(data):
    def unique(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                fail("duplicate node JSON field: " + key)
            value[key] = item
        return value
    return json.loads(data, object_pairs_hook=unique, parse_constant=lambda _: fail("nonfinite node JSON number"))


def node_hash(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        fail("invalid node input hash")


def node_path(value):
    if not isinstance(value, str) or not value.startswith("/") or len(value.encode()) > 4096 or any(ord(character) < 32 or ord(character) == 127 for character in value):
        fail("invalid absolute node input path")
    if any(part in ("", ".", "..") for part in value.split("/")[1:]):
        fail("noncanonical node input path")


def node_image(value, extra=()):
    closed_fields(value, ("bytes", "sha256", *extra), "node image")
    if type(value["bytes"]) is not int or not 0 <= value["bytes"] <= FIXTURE_LIMITS["bytes"]:
        fail("invalid node image length")
    node_hash(value["sha256"])
    if "mode" in extra and (type(value["mode"]) is not int or not 0 <= value["mode"] <= 0o7777):
        fail("invalid node image mode")
    if "path" in extra:
        node_path(value["path"])


def validate_node_profile(profile):
    """Schema checks supplement the exact reviewed whole-profile byte pin."""
    closed_fields(profile, ("format", "profile", "accepted_a2_commit", "target", "runtime_lock", "patch_series", "source_manifest", "command_profile", "program", "fixture", "launch", "network", "limits", "node_settings"), "node profile")
    if type(profile["format"]) is not int or profile["format"] != 1 or profile["profile"] != NODE or profile["accepted_a2_commit"] != NODE_BASE or profile["target"] != TARGET or profile["command_profile"] != "outer-ordinary-celld-v1":
        fail("unsupported node profile identity")
    for name in ("runtime_lock", "patch_series", "source_manifest"):
        node_image(profile[name])
    closed_fields(profile["program"], ("js", "wrangler"), "node program")
    for image in profile["program"].values():
        node_image(image)
    if profile["limits"] != NODE_LIMITS or any(type(value) is not int for value in profile["limits"].values()):
        fail("node limits differ from reviewed finite policy")
    network = profile["network"]
    if network != {"address": "127.0.0.1", "roles": list(NODE_ROLES), "sandbox_executable": "/usr/bin/sandbox-exec", "template": NODE_POLICY}:
        fail("node network policy changed")
    launch = profile["launch"]
    closed_fields(launch, ("bootstrap", "certifi", "controller", "empty_native_directory", "environment", "flags", "moto_module", "process_inspection", "python", "sys_path"), "node launch")
    if launch["bootstrap"] != "closed-sys-path-runpy-v1" or launch["controller"] != "run-node.py" or launch["empty_native_directory"] != "empty-native-config" or launch["flags"] != ["-I", "-S", "-B"] or launch["moto_module"] != "moto.server":
        fail("node launch contract changed")
    for name in ("python", "certifi"):
        node_path(launch[name])
    if not isinstance(launch["sys_path"], list) or len(launch["sys_path"]) != 3 or len(set(launch["sys_path"])) != 3:
        fail("invalid node import path inventory")
    for path in launch["sys_path"]:
        node_path(path)
    inspection = launch["process_inspection"]
    closed_fields(inspection, ("abi", "backend", "bootstrap", "headers", "library", "library_policy", "limits", "source"), "process inspection")
    if inspection["backend"] != "darwin-libproc-shortbsdinfo" or inspection["bootstrap"] != "closed-stdlib-literal-v1" or inspection["library"] != "/usr/lib/libproc.dylib" or inspection["library_policy"] != "operating-system-shared-cache":
        fail("process inspection platform contract changed")
    expected_abi = {"alignment": 4, "arg": 1, "bytes": 64, "c_int_bytes": 4, "flavor": 13, "uint32_bytes": 4, "uint64_bytes": 8,
                    "fields": [["pid", 0], ["ppid", 4], ["pgid", 8], ["status", 12], ["comm", 16], ["flags", 32], ["uid", 36], ["gid", 40], ["ruid", 44], ["rgid", 48], ["svuid", 52], ["svgid", 56], ["reserved", 60]],
                    "statuses": [1, 2, 3, 4, 5]}
    expected_limits = {"helper_timeout_ns": 3_000_000_000, "pid_capacity": 4096, "stderr_bytes": 65536, "stdout_bytes": 65536}
    # Canonical encodings distinguish bool from int even in nested ABI rows.
    if json.dumps(inspection["abi"], sort_keys=True) != json.dumps(expected_abi, sort_keys=True) or json.dumps(inspection["limits"], sort_keys=True) != json.dumps(expected_limits, sort_keys=True):
        fail("process inspection ABI or bounds changed")
    node_image(inspection["source"])
    if not 0 < inspection["source"]["bytes"] <= 65536:
        fail("process inspection source bound")
    header_root = "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk/usr/include/"
    header_names = ["libproc.h", "sys/proc_info.h", "sys/param.h", "sys/proc.h"]
    if not isinstance(inspection["headers"], list) or len(inspection["headers"]) != len(header_names):
        fail("process inspection header roster")
    for header, name in zip(inspection["headers"], header_names):
        node_image(header, ("path",))
        if header["path"] != header_root + name or not 0 < header["bytes"] <= 131072:
            fail("process inspection header identity/bound")
    for environment in (launch["environment"], profile["node_settings"]):
        if not isinstance(environment, dict) or any(not isinstance(name, str) or not isinstance(value, str) or "\0" in name + value for name, value in environment.items()):
            fail("invalid node environment")
    fixture = profile["fixture"]
    closed_fields(fixture, ("aliases", "bytecode", "external_native", "inventory_limits", "native_roster", "non_os_load_edges", "platform", "required_absence", "startup", "trees", "versions"), "node fixture")
    if fixture["inventory_limits"] != FIXTURE_LIMITS or any(type(value) is not int for value in fixture["inventory_limits"].values()):
        fail("fixture inventory bounds changed")
    for name, count in (("trees", 2), ("external_native", 8), ("aliases", 12), ("native_roster", 95), ("non_os_load_edges", 9), ("required_absence", 3), ("startup", 15)):
        if not isinstance(fixture[name], list) or len(fixture[name]) != count:
            fail("fixture inventory count changed: " + name)
    labels = set()
    for tree in fixture["trees"]:
        closed_fields(tree, ("bytes", "entries", "files", "label", "path", "sha256"), "fixture tree")
        node_path(tree["path"])
        node_hash(tree["sha256"])
        if tree["label"] not in ("venv", "stdlib") or tree["label"] in labels or any(type(tree[key]) is not int or tree[key] < 0 for key in ("bytes", "entries", "files")):
            fail("invalid fixture tree counts/label")
        labels.add(tree["label"])
    if fixture["bytecode"] != {"stdlib": 398, "venv": 448} or any(type(value) is not int for value in fixture["bytecode"].values()):
        fail("fixture bytecode inventory changed")
    if fixture["versions"] != {"boto3": "1.43.114", "botocore": "1.43.114", "moto": "5.1.17", "python": "3.14.3"}:
        fail("fixture version inventory changed")
    for native in [*fixture["external_native"], fixture["platform"]]:
        node_image(native, ("mode", "path"))
    for alias in fixture["aliases"]:
        closed_fields(alias, ("path", "resolved", "symlink_chain", "target_sha256"), "fixture alias")
        node_path(alias["path"])
        node_path(alias["resolved"])
        node_hash(alias["target_sha256"])
        if not isinstance(alias["symlink_chain"], list) or len(alias["symlink_chain"]) > 32:
            fail("invalid alias link chain")
        for link in alias["symlink_chain"]:
            closed_fields(link, ("path", "target"), "fixture alias link")
            node_path(link["path"])
            if not isinstance(link["target"], str) or not link["target"] or len(link["target"].encode()) > 4096:
                fail("invalid alias target")
    for key in ("native_roster", "required_absence"):
        if len(fixture[key]) != len(set(fixture[key])):
            fail("duplicate fixture path")
        for path in fixture[key]:
            node_path(path)
    for edge in fixture["non_os_load_edges"]:
        closed_fields(edge, ("from", "name", "resolved"), "native load edge")
        for path in edge.values():
            node_path(path)
    for candidate in fixture["startup"]:
        if not isinstance(candidate, dict):
            fail("invalid fixture startup record")
        if candidate.get("required") == "regular-file":
            node_image(candidate, ("path", "required"))
        elif candidate.get("required") == "absent-including-dangling-symlink":
            closed_fields(candidate, ("path", "required"), "startup absence")
            node_path(candidate["path"])
        else:
            fail("unknown startup input requirement")


def node_program(root):
    path = root / "run-node.py"
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 256 * 1024:
        fail("invalid node controller source")
    try:
        tree = ast.parse(path.read_bytes(), filename=str(path))
    except SyntaxError as error:
        raise ValueError("node controller source is not parseable") from error
    selected = {"WORKER_JS": "js", "WRANGLER_JSON": "wrangler"}
    values = {}
    # Only literal top-level assignments are admissible. ast.literal_eval is
    # deliberately unnecessary: no operators, calls or concatenation execute.
    for statement in tree.body:
        targets = statement.targets if isinstance(statement, ast.Assign) else [statement.target] if isinstance(statement, ast.AnnAssign) else []
        for target in targets:
            if isinstance(target, ast.Name) and target.id in selected:
                if len(targets) != 1 or target.id in values or not isinstance(statement.value, ast.Constant) or not isinstance(statement.value.value, bytes):
                    fail("node program must be a unique literal byte assignment")
                data = statement.value.value
                if len(data) > 65536:
                    fail("node program exceeds fixed allowance")
                values[target.id] = {"bytes": len(data), "sha256": digest(data)}
    if set(values) != set(selected):
        fail("missing literal node program")
    return {selected[name]: value for name, value in values.items()}


def node_process_inspector(root):
    path = root / "run-node.py"
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 256 * 1024:
        fail("invalid node controller source")
    try:
        tree = ast.parse(path.read_bytes(), filename=str(path))
    except SyntaxError as error:
        raise ValueError("node controller source is not parseable") from error
    value = None
    for statement in tree.body:
        targets = statement.targets if isinstance(statement, ast.Assign) else [statement.target] if isinstance(statement, ast.AnnAssign) else []
        for target in targets:
            if isinstance(target, ast.Name) and target.id == "PROCESS_INSPECTOR":
                if len(targets) != 1 or value is not None or not isinstance(statement.value, ast.Constant) or not isinstance(statement.value.value, str):
                    fail("process inspector must be a unique literal string assignment")
                value = statement.value.value.encode("utf-8")
    if value is None or not 0 < len(value) <= 65536:
        fail("missing or oversized literal process inspector")
    return {"bytes": len(value), "sha256": digest(value)}


def node_process_inputs(root, profile):
    inspection = profile["launch"]["process_inspection"]
    actual_source = node_process_inspector(root)
    if actual_source != inspection["source"]:
        fail("process inspector literal differs from reviewed input")
    headers = []
    for expected in inspection["headers"]:
        actual, _ = stable_fixture_file(expected["path"], expected["bytes"], expected)
        headers.append({"path": expected["path"], "bytes": actual["bytes"], "sha256": actual["sha256"]})
    return {"source": actual_source, "headers": headers, "library": inspection["library"], "library_policy": inspection["library_policy"]}


def load_node_profile(root=ROOT):
    path = root / "node.lock.json"
    profile = read_json(path, 128 * 1024)
    validate_node_profile(profile)
    # Paths, aliases, launch whitelist and all data rows are a closed immutable
    # qualification input, not user-supplied configuration. Changing them needs
    # a new source-reviewed pin as well as passing the structural checks above.
    if file_hash(path) != NODE_PROFILE_SHA256:
        fail("node profile differs from reviewed immutable bytes")
    for name, relative in (("runtime_lock", "runtime.lock.json"), ("patch_series", "patches/series.json"), ("source_manifest", "runtime/source-manifest.json")):
        checked_file(root / relative, profile[name])
    if node_program(root) != profile["program"]:
        fail("node program bytes differ from reviewed input")
    if node_process_inspector(root) != profile["launch"]["process_inspection"]["source"]:
        fail("process inspector literal differs from reviewed input")
    return profile


def resolve_fixture_path(path, maximum_hops=32):
    """Finite component-wise resolution records opt/venv aliases as well."""
    path = Path(path)
    if not path.is_absolute():
        fail("fixture resolution requires an absolute path")
    pending = list(path.parts[1:])
    current = Path("/")
    chain, seen = [], set()
    while pending:
        part = pending.pop(0)
        if part in ("", "."):
            continue
        if part == "..":
            current = current.parent
            continue
        candidate = current / part
        try:
            status = candidate.lstat()
        except FileNotFoundError:
            # No traversal of an absent subtree. Canonicalize the remaining
            # lexical suffix, matching resolve(strict=False) for these pins.
            return str(Path(os.path.normpath(str(candidate.joinpath(*pending))))), chain, False
        if stat.S_ISLNK(status.st_mode):
            marker = (str(candidate), tuple(pending))
            if marker in seen or len(chain) >= maximum_hops:
                fail("fixture symlink cycle or hop limit")
            seen.add(marker)
            target = os.readlink(candidate)
            target.encode("utf-8", "strict")
            if not target or len(target.encode()) > 4096:
                fail("invalid fixture symlink target")
            chain.append({"path": str(candidate), "target": target})
            parts = list(PurePosixPath(target).parts)
            if target.startswith("/"):
                current = Path("/")
                parts = parts[1:]
            pending = parts + pending
        else:
            current = candidate
    return str(current), chain, True


def fixture_stat_identity(status):
    return (status.st_dev, status.st_ino, status.st_mode, status.st_size, status.st_mtime_ns, status.st_ctime_ns)


def stable_fixture_file(path, maximum=1024**3, expected=None):
    """Hash actual regular bytes and detect replacement/change around the read."""
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        fail("fixture input is not a regular nonsymlink file: " + str(path))
    if type(maximum) is not int or not 0 <= before.st_size <= maximum:
        fail("fixture file exceeds admitted length before read")
    if expected is not None and (before.st_size != expected["bytes"] or ("mode" in expected and stat.S_IMODE(before.st_mode) != expected["mode"])):
        fail("fixture input size/mode mismatch before read")
    checksum = hashlib.sha256()
    count, first = 0, b""
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        if fixture_stat_identity(os.fstat(stream.fileno())) != fixture_stat_identity(before):
            fail("fixture file replaced before read")
        while chunk := stream.read(CHUNK):
            if not first:
                first = chunk[:4]
            count += len(chunk)
            if count > before.st_size:
                fail("fixture file grew during read")
            checksum.update(chunk)
        if fixture_stat_identity(os.fstat(stream.fileno())) != fixture_stat_identity(before):
            fail("fixture file changed during read")
    if count != before.st_size or fixture_stat_identity(path.lstat()) != fixture_stat_identity(before):
        fail("fixture file replaced or resized during read")
    result = {"bytes": count, "mode": stat.S_IMODE(before.st_mode), "sha256": checksum.hexdigest()}
    if expected is not None and result["sha256"] != expected["sha256"]:
        fail("fixture input hash mismatch")
    return result, first in MACHO_MAGICS


def fixture_tree(root, allowance):
    root = Path(root)
    resolved, chain, exists = resolve_fixture_path(root, allowance["symlink_hops"])
    if not exists or chain or resolved != str(root) or not stat.S_ISDIR(root.lstat().st_mode):
        fail("fixture tree root changed or is aliased")

    def roster():
        result = {}
        pending = [root]
        while pending:
            directory = pending.pop()
            with os.scandir(directory) as entries:
                for entry in entries:
                    path = Path(entry.path)
                    relative = relative_name(path.relative_to(root).as_posix())
                    if relative in result or len(result) >= allowance["entries"]:
                        fail("fixture entry count exceeds allowance")
                    status = path.lstat()
                    if not any(check(status.st_mode) for check in (stat.S_ISDIR, stat.S_ISREG, stat.S_ISLNK)):
                        fail("special file in fixture tree")
                    result[relative] = fixture_stat_identity(status)
                    if stat.S_ISDIR(status.st_mode):
                        pending.append(path)
        return result

    root_before = fixture_stat_identity(root.lstat())
    before = roster()
    rows, native = [], []
    total, files, bytecode = 0, 0, 0
    for relative, identity in sorted(before.items()):
        path = root / relative
        status = path.lstat()
        if fixture_stat_identity(status) != identity:
            fail("fixture roster changed during scan")
        if stat.S_ISLNK(status.st_mode):
            target = os.readlink(path)
            target.encode("utf-8", "strict")
            resolved, _, exists = resolve_fixture_path(path, allowance["symlink_hops"])
            rows.append({"path": relative, "kind": "symlink", "target": target, "resolved": resolved, "exists": exists})
        elif stat.S_ISDIR(status.st_mode):
            rows.append({"path": relative, "kind": "directory", "mode": stat.S_IMODE(status.st_mode)})
        else:
            if total + status.st_size > allowance["bytes"]:
                fail("fixture logical bytes exceed allowance")
            image, is_native = stable_fixture_file(path, allowance["bytes"] - total)
            rows.append({"path": relative, "kind": "file", **image})
            total += image["bytes"]
            files += 1
            bytecode += relative.endswith(".pyc")
            if is_native:
                native.append(str(path))
                if len(native) > allowance["native_files"]:
                    fail("fixture native count exceeds allowance")
    if roster() != before or fixture_stat_identity(root.lstat()) != root_before:
        fail("fixture roster changed during scan")
    encoded = json.dumps(rows, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    if len(encoded) > allowance["canonical_json_bytes"]:
        fail("fixture inventory JSON exceeds allowance")
    return {"entries": len(rows), "files": files, "bytes": total, "sha256": digest(encoded)}, encoded, native, bytecode


def fixture_absent(path):
    try:
        Path(path).lstat()
    except FileNotFoundError:
        return
    fail("unexpected fixture startup/import input: " + str(path))


def verify_node_fixture(profile, evidence_directory=None):
    fixture = profile["fixture"]
    allowance = dict(fixture["inventory_limits"])
    trees, native = [], []
    for expected in fixture["trees"]:
        actual, encoded, found, bytecode = fixture_tree(expected["path"], allowance)
        if actual != {key: expected[key] for key in ("entries", "files", "bytes", "sha256")} or bytecode != fixture["bytecode"][expected["label"]]:
            fail("complete fixture tree identity changed: " + expected["label"])
        if evidence_directory is not None:
            with (Path(evidence_directory) / (expected["label"] + "-inventory.json")).open("xb") as output:
                output.write(encoded)
        trees.append({**actual, "label": expected["label"], "path": expected["path"]})
        native.extend(found)
        for key, amount in (("entries", actual["entries"]), ("bytes", actual["bytes"]), ("canonical_json_bytes", len(encoded)), ("native_files", len(found))):
            allowance[key] -= amount
    for expected in [*fixture["external_native"], fixture["platform"]]:
        if expected is not fixture["platform"]:
            allowance["bytes"] -= expected["bytes"]
            if allowance["bytes"] < 0:
                fail("combined fixture bytes exceed allowance")
        actual, is_native = stable_fixture_file(expected["path"], expected["bytes"], expected)
        if actual != {key: expected[key] for key in ("bytes", "mode", "sha256")}:
            fail("native/platform fixture input changed")
        if expected is not fixture["platform"]:
            if not is_native:
                fail("expected native fixture is not Mach-O")
            native.append(expected["path"])
    if len(native) > fixture["inventory_limits"]["native_files"] or sorted(native) != fixture["native_roster"]:
        fail("native fixture roster changed")
    for alias in fixture["aliases"]:
        resolved, chain, exists = resolve_fixture_path(alias["path"], allowance["symlink_hops"])
        expected = next((item for item in fixture["external_native"] if item["path"] == resolved), None)
        if not exists or resolved != alias["resolved"] or chain != alias["symlink_chain"] or expected is None or stable_fixture_file(resolved, expected["bytes"], expected)[0]["sha256"] != alias["target_sha256"]:
            fail("fixture executable/load alias changed")
    aliases = {item["path"]: item["resolved"] for item in fixture["aliases"]}
    for edge in fixture["non_os_load_edges"]:
        if edge["from"] not in native or edge["resolved"] not in native or aliases.get(edge["name"]) != edge["resolved"]:
            fail("unbound non-OS native load edge")
    for path in fixture["required_absence"]:
        fixture_absent(path)
    for entry in fixture["startup"]:
        if entry["required"] == "absent-including-dangling-symlink":
            fixture_absent(entry["path"])
        else:
            actual, _ = stable_fixture_file(entry["path"], entry["bytes"], entry)
            if any(actual[name] != entry[name] for name in ("bytes", "sha256")):
                fail("fixture startup input changed")
    verify_platform_null()
    # Return immutable evidence, not thousands of retained per-file rows.
    return {"trees": trees, "supplements_sha256": digest(json.dumps({key: value for key, value in fixture.items() if key != "trees"}, sort_keys=True, separators=(",", ":")).encode()), "native_files": len(native)}


def node_inputs(root=ROOT, profile=None, evidence_directory=None):
    current = load_node_profile(root)
    if profile is not None and profile != current:
        fail("node profile changed during qualification")
    return {"source": {name: stable_fixture_file(root / name, 256 * 1024)[0]["sha256"] for name in NODE_SOURCE_PATHS}, "node_lock_sha256": file_hash(root / "node.lock.json"), "fixture": verify_node_fixture(current, evidence_directory), "process_inspection": node_process_inputs(root, current)}


def verify_platform_null():
    status = Path("/dev/null").lstat()
    if not stat.S_ISCHR(status.st_mode) or (os.major(status.st_rdev), os.minor(status.st_rdev)) != (3, 2):
        fail("platform /dev/null is not the pinned Darwin character device 3:2")


def validate_node_ports(ports):
    if not isinstance(ports, dict) or set(ports) != set(NODE_ROLES) or any(type(port) is not int or not 1024 <= port <= 65535 for port in ports.values()) or len(set(ports.values())) != 5:
        fail("invalid closed node port allocation")


def choose_node_ports():
    with contextlib.ExitStack() as held:
        ports = {}
        for role in NODE_ROLES:
            listener = held.enter_context(socket.socket(socket.AF_INET, socket.SOCK_STREAM))
            listener.bind(("127.0.0.1", 0))
            ports[role] = listener.getsockname()[1]
        validate_node_ports(ports)
        return ports


def node_network_command(argv, ports):
    validate_node_ports(ports)
    sandbox = Path("/usr/bin/sandbox-exec")
    if not sandbox.is_file():
        fail("reviewed node sandbox launcher unavailable")
    return [str(sandbox), "-p", NODE_POLICY.format(**ports), *map(str, argv)]


def verify_empty_native_directory(directory):
    path = Path(directory) / "empty-native-config"
    if path.is_symlink() or not path.is_dir() or any(path.iterdir()):
        fail("isolated native configuration directory changed")


def node_environment(root, profile, directory, ports):
    validate_node_ports(ports)
    directory = Path(directory)
    empty = directory / "empty-native-config"
    temporary = directory / "tmp"
    empty.mkdir()
    temporary.mkdir()
    verify_empty_native_directory(directory)
    verify_platform_null()
    return {**profile["launch"]["environment"], "TMPDIR": str(temporary), "OPENSSL_CONF_INCLUDE": str(empty), "OPENSSL_MODULES": str(empty), "SSL_CERT_DIR": str(empty), "SSL_CERT_FILE": profile["launch"]["certifi"]}


def node_controller_command(root, profile, binary, directory, ports, deadline_ns):
    validate_node_ports(ports)
    if type(deadline_ns) is not int or deadline_ns <= time.monotonic_ns():
        fail("node scenario deadline expired before launch")
    # sys is built in. Replace the import path before loading even runpy.
    bootstrap = "import sys; sys.path[:] = " + repr(profile["launch"]["sys_path"]) + "; import runpy; sys.argv = sys.argv[1:]; runpy.run_path(sys.argv[0], run_name='__main__')"
    args = [profile["launch"]["python"], *profile["launch"]["flags"], "-c", bootstrap, str(root / "run-node.py"), "--profile", str(root / "node.lock.json"), "--node-lock-sha256", file_hash(root / "node.lock.json"), "--binary", binary["path"], "--binary-sha256", binary["sha256"], "--directory", str(directory), "--deadline-ns", str(deadline_ns)]
    for role in NODE_ROLES:
        args.extend(("--" + role.replace("_", "-") + "-port", str(ports[role])))
    return node_network_command(args, ports)


def verify_node_artifact(stdout_path, graph, root=ROOT):
    target = graph.get("node_target")
    if not isinstance(target, dict):
        fail("missing actual node target graph")
    expected_path = root / "runtime/target" / TARGET / "debug/celld"
    artifacts, finished = [], []
    consumed = 0
    with Path(stdout_path).open("rb") as stream:
        while line := stream.readline(4 * 1024 * 1024 + 1):
            consumed += len(line)
            if consumed > 128 * 1024 * 1024 or len(line) > 4 * 1024 * 1024:
                fail("compiler message allowance exceeded")
            try:
                message = node_json(line)
            except (ValueError, UnicodeError) as error:
                raise ValueError("unclassified compiler message") from error
            if not isinstance(message, dict):
                fail("unclassified compiler message")
            if message.get("reason") == "compiler-artifact" and message.get("executable") is not None:
                actual = message.get("target", {})
                if message.get("package_id") != target["package_id"] or message.get("features") != target["features"] or message.get("executable") != str(expected_path) or any(actual.get(key) != target[key] for key in ("name", "kind", "crate_types", "src_path")) or message.get("profile", {}).get("test") is not False:
                    fail("unexpected executable compiler artifact")
                artifacts.append(message)
            elif message.get("reason") == "build-finished":
                if type(message.get("success")) is not bool:
                    fail("compiler completion is not a boolean")
                finished.append(message["success"])
            elif message.get("reason") not in ("compiler-artifact", "compiler-message", "build-script-executed"):
                fail("unclassified compiler message reason")
    if len(artifacts) != 1 or finished != [True]:
        fail("missing unique successful celld compiler artifact")
    # The executable is inside the already admitted four-GiB target allowance.
    image, _ = stable_fixture_file(expected_path, 4 * 1024**3)
    if not image["bytes"] or image["mode"] & 0o111 == 0:
        fail("celld artifact is not executable")
    return {"path": str(expected_path), "bytes": image["bytes"], "sha256": image["sha256"], "artifact": target.copy()}


def verify_node_report(path, profile, binary, ports, node_lock_hash):
    report = read_json(path, profile["limits"]["report_bytes"])
    closed_fields(report, ("format", "profile", "claim", "status", "node_lock_sha256", "binary", "program", "ports", "run_id", "startup", "sequence", "cleanup", "observation_limits"), "node report")
    if type(report["format"]) is not int or report["format"] != 1 or report["profile"] != NODE or report["claim"] != NODE_CLAIM or report["status"] != "passed":
        fail("node controller did not report the closed successful claim")
    if report["node_lock_sha256"] != node_lock_hash or report["binary"] != {key: binary[key] for key in ("path", "sha256")} or report["program"] != profile["program"] or report["ports"] != ports:
        fail("node controller input binding mismatch")
    validate_node_ports(report["ports"])
    if not isinstance(report["run_id"], str) or not re.fullmatch(r"[0-9a-f]{32}", report["run_id"]):
        fail("invalid node run identity")
    if not isinstance(report["startup"], dict) or not report["startup"]:
        fail("missing actual node startup/sequence observations")
    sequence = report["sequence"]
    closed_fields(sequence, ("completed", "steps", "incarnations", "error"), "node sequence")
    if sequence["completed"] is not True or sequence["error"] is not None or not isinstance(sequence["steps"], list) or not 1 <= len(sequence["steps"]) <= 64 or not isinstance(sequence["incarnations"], list) or len(sequence["incarnations"]) != 4:
        fail("node sequence incomplete")
    cleanup = report["cleanup"]
    closed_fields(cleanup, ("completed", "children", "storage_tasks_joined", "active_responses", "outer_group_disappearance_observed", "error"), "node cleanup")
    if cleanup["completed"] is not True or cleanup["storage_tasks_joined"] is not True or type(cleanup["active_responses"]) is not int or cleanup["active_responses"] != 0 or cleanup["outer_group_disappearance_observed"] is not False or cleanup["error"] is not None or not isinstance(cleanup["children"], list) or not cleanup["children"]:
        fail("node controller child cleanup incomplete")
    children = {}
    for child in cleanup["children"]:
        closed_fields(child, ("label", "pid", "argv", "node", "directory", "ports", "generation", "started_ns", "exit_ns", "returncode", "attributed_pids", "retired", "log"), "node child")
        label = child["label"]
        if not isinstance(label, str) or label in children or child["retired"] is not True or type(child["returncode"]) is not int or (child["returncode"] == 0) != (label != "invalid-option"):
            fail("node child retirement/result not established")
        if any(type(child[key]) is not int or child[key] <= 0 for key in ("pid", "started_ns", "exit_ns")) or child["exit_ns"] < child["started_ns"]:
            fail("node child physical lifetime missing")
        if not isinstance(child["argv"], list) or not child["argv"] or any(not isinstance(value, str) for value in child["argv"]) or not isinstance(child["log"], str):
            fail("node child command/log identity missing")
        if not isinstance(child["attributed_pids"], list) or child["pid"] not in child["attributed_pids"] or any(type(pid) is not int or pid <= 0 for pid in child["attributed_pids"]):
            fail("node child descendant inventory missing")
        if child["node"] is not None and not isinstance(child["node"], str) or child["directory"] is not None and not isinstance(child["directory"], str) or child["ports"] is not None and not isinstance(child["ports"], dict):
            fail("node child metadata has invalid types")
        children[label] = child
    if set(children) != {"version", "help", "invalid-option", "moto", "deploy", "a", "b", "b-restart", "c"}:
        fail("node child inventory incomplete or unexpected")
    expected_incarnations = [children[label] for label in ("a", "b", "b-restart", "c")]
    if sequence["incarnations"] != expected_incarnations or any(not isinstance(child["generation"], str) or not child["generation"] for child in expected_incarnations):
        fail("node incarnation evidence differs from retired children")
    for child in expected_incarnations:
        suffix = "b" if child["label"] in ("b", "b-restart") else "ac"
        expected_ports = {"public": ports["public_" + suffix], "internal": ports["internal_" + suffix]}
        if child["ports"] != expected_ports or not child["node"] or not child["directory"] or child["argv"][0] != binary["path"]:
            fail("node incarnation address/command binding mismatch")
    if report["observation_limits"] != {"final_transport_grace_observed": False, "internal_task_drain_observed": False} or any(type(value) is not bool for value in report["observation_limits"].values()):
        fail("unsupported node drain observation claim")
    return report


def node_monitor(root, directory, profile):
    refusal, resources = resource_refusal(root)
    resources["evidence_bytes"] = directory_bytes(directory)
    if resources["evidence_bytes"] > profile["limits"]["evidence_bytes"]:
        refusal = "node evidence allowance exceeded"
    return refusal, resources


def build_node(graph_directory, root=ROOT):
    import check_graph
    pins, profile = load_pins(root), load_node_profile(root)
    selected_mode(pins, "node-baseline")
    source, assets = verify_source(root), verify_assets(root)
    graph = check_graph.verify(graph_directory, root, "node-baseline")
    refusal, resources = resource_refusal(root, initial=True)
    if refusal:
        fail(refusal + ": " + json.dumps(resources))
    toolchain, environment = controlled_environment(root)
    evidence_root = runtime_directory(root) / "evidence"
    evidence_root.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="node-", dir=evidence_root))
    before_directory = directory / "fixture-before"
    before_directory.mkdir()
    receipt = {"format": 1, "profile": NODE, "mode": "node-baseline", "claim": NODE_CLAIM, "status": "incomplete", "source": source, "assets": assets, "graph_directory": str(Path(graph_directory).resolve()), "graph_sha256": digest(json.dumps(graph, sort_keys=True).encode()), "initial_resources": resources}
    write_json(directory / "receipt.json", receipt)
    runtime_started = False
    with BuildSlot(root) as slot:
        try:
            receipt["node_inputs_before"] = node_inputs(root, profile, before_directory)
            receipt["tools"] = tool_identity(root, directory, slot, environment, toolchain)
            for name, argv in build_commands(root, toolchain, pins, "node-baseline").items():
                receipt[name] = run_owned(argv, root, environment, directory, name, slot, pins["build"]["timeout_seconds"], lambda: node_monitor(root, directory, profile))
            binary = verify_node_artifact(directory / "build.stdout", graph, root)
            receipt["binary"] = binary
            # No selected interpreter starts until all actual input bytes have
            # been checked again after build, including the before-bound source.
            if node_inputs(root, profile) != receipt["node_inputs_before"]:
                fail("node inputs changed before runtime launch")
            run_directory = directory / "run"
            run_directory.mkdir()
            ports = choose_node_ports()
            runtime_environment = node_environment(root, profile, run_directory, ports)
            deadline_ns = time.monotonic_ns() + profile["limits"]["scenario_seconds"] * 1_000_000_000
            argv = node_controller_command(root, profile, binary, run_directory, ports, deadline_ns)
            receipt.update(ports=ports, runtime_environment=runtime_environment, network_policy=NODE_POLICY.format(**ports), deadline_ns=deadline_ns)
            runtime_started = True
            receipt["controller"] = run_owned(argv, root, runtime_environment, directory, "controller", slot, max(0, (deadline_ns - time.monotonic_ns()) / 1_000_000_000), lambda: node_monitor(root, directory, profile), maximum_log_bytes=profile["limits"]["log_bytes"])
            # run_owned has now established actual whole-process-group cleanup.
            verify_empty_native_directory(run_directory)
            stable_fixture_file(binary["path"], binary["bytes"], binary)
            report_path = run_directory / "report.json"
            receipt["report"] = verify_node_report(report_path, profile, binary, ports, receipt["node_inputs_before"]["node_lock_sha256"])
            receipt["report_sha256"] = file_hash(report_path)
            after_directory = directory / "fixture-after"
            after_directory.mkdir()
            receipt["node_inputs_after"] = node_inputs(root, profile, after_directory)
            if receipt["node_inputs_before"] != receipt["node_inputs_after"]:
                fail("node inputs changed during qualification")
            if verify_source(root) != source or verify_assets(root) != assets or check_graph.verify(graph_directory, root, "node-baseline") != graph:
                fail("node build inputs changed during qualification")
            if config_inputs(root) != receipt["tools"]["cargo_configs"] or any(file_hash(path) != expected for path, expected in receipt["tools"]["native_tools"].items()):
                fail("native tools/Cargo configuration changed during qualification")
            controlled_environment(root)
            refusal, _ = node_monitor(root, directory, profile)
            if refusal:
                fail(refusal)
            receipt["status"] = "passed"
        except BaseException as error:
            receipt["error"] = str(error)
            receipt["status"] = "failed" if slot.clean else "cleanup uncertain"
            raise
        finally:
            if runtime_started and slot.clean and "node_inputs_after" not in receipt:
                # Failed commands still get post-run identity evidence once
                # physical cleanup is established. Preserve the original error;
                # a second failure cannot manufacture a passed receipt.
                try:
                    verify_empty_native_directory(run_directory)
                    after_directory = directory / "fixture-after-failure"
                    after_directory.mkdir()
                    receipt["node_inputs_after"] = node_inputs(root, profile, after_directory)
                    if receipt["node_inputs_after"] != receipt["node_inputs_before"]:
                        fail("node inputs changed during failed qualification")
                except BaseException as error:
                    receipt["post_cleanup_input_error"] = str(error)
                    receipt["status"] = "failed"
            receipt["final_resources"] = disk_status(root)
            write_json(directory / "receipt.json", receipt)
    return {"directory": str(directory), "receipt_sha256": file_hash(directory / "receipt.json"), "status": receipt["status"]}


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
    if mode == "node-baseline" or mode in NODE_BINARY_MODES:
        shared = ["--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", "--package", "foks-celld-guarded", "--package", "celld@0.6.2", *features, "--bin", "celld", "-j", "2", "--target", TARGET]
        cargo = toolchain / "bin/cargo"
        return {
            "build": network_command([cargo, "build", *shared, "--message-format=json-render-diagnostics"]),
            "clippy": network_command([cargo, "clippy", *shared, "--", "-D", "warnings"]),
        }
    shared = ["--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", *features, "-j", "2", "--target", TARGET]
    binary = ["--bin", "foks-celld-guarded"]
    cargo = toolchain / "bin/cargo"
    commands = {
        "build": network_command([cargo, "build", *shared, *binary]),
        "clippy": network_command([cargo, "clippy", *shared, *(["--all-targets"] if mode == "lifecycle" or mode in NODE_LIFECYCLE_MODES else binary), "--", "-D", "warnings"]),
    }
    if mode == "lifecycle" or mode in NODE_LIFECYCLE_MODES:
        commands["lifecycle"] = network_command([cargo, "test", *shared, "--test", "lifecycle", "--", "--test-threads=1", "--nocapture"])
    return commands


def foundation_test_commands(root, toolchain, pins):
    if pins["profile"] in (FRONTEND, OBSERVATIONS):
        fail("frontend tests require the bound direct executable and two-port policy")
    selected_mode(pins, pins["profile"])
    prefix = LIFETIMES_TEST_PREFIX if pins["profile"] == LIFETIMES else FOUNDATION_TEST_PREFIX
    shared = [toolchain / "bin/cargo", "test", "--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", "--package", "foks-celld-guarded", "--package", "celld@0.6.2", "--features", "foks-celld-guarded/node-qualification", "--lib", "-j", "2", "--target", TARGET, prefix, "--"]
    return {"list": network_command([*shared, "--list"]), "run": network_command([*shared, "--test-threads=1"])}


def frontend_test_compile_command(root, toolchain, pins):
    if pins["profile"] not in (FRONTEND, OBSERVATIONS):
        fail("frontend compilation requires its exact stage")
    selected_mode(pins, pins["profile"])
    return network_command([toolchain / "bin/cargo", "test", "--locked", "--offline", "--manifest-path", str(root / "Cargo.toml"), "--no-default-features", "--package", "foks-celld-guarded", "--package", "celld@0.6.2", "--features", "foks-celld-guarded/node-qualification", "--lib", "-j", "2", "--target", TARGET, "--no-run", "--message-format=json"])


def frontend_transport_commands(root, toolchain, name):
    if name not in FRONTEND_TRANSPORT_TESTS:
        fail("unknown frontend transport test group")
    prefix = "deadline::tests::" if name == "deadline" else "memory::tests::qualification_"
    # Cargo must test this dev-dependency-bearing package in its own workspace.
    # These are root-workspace A0 regressions, not celld dependency-graph tests.
    # Keep its different lock/profile artifacts separate while retaining the
    # same aggregate target-directory resource ceiling and owned cleanup.
    shared = [toolchain / "bin/cargo", "test", "--locked", "--offline", "--manifest-path", str(root.parents[1] / "Cargo.toml"), "--no-default-features", "--package", "foks-hosted-transport", "--features", "qualification", "--lib", "-j", "2", "--target", TARGET, "--target-dir", str(root / "runtime/target/transport-regression"), prefix, "--"]
    return {"list": network_command([*shared, "--list"]), "run": network_command([*shared, "--test-threads=1"])}


def validate_frontend_ports(ports):
    if not isinstance(ports, dict) or set(ports) != set(FRONTEND_ROLES) or any(type(value) is not int or not 1024 <= value <= 65535 for value in ports.values()) or len(set(ports.values())) != 2:
        fail("invalid closed frontend port allocation")


def choose_frontend_ports():
    # Reserve both distinct ports together. Release immediately before child
    # execution; a collision at the actual bind fails the run, without retry.
    with contextlib.ExitStack() as held:
        ports = {}
        for role in FRONTEND_ROLES:
            listener = held.enter_context(socket.socket(socket.AF_INET, socket.SOCK_STREAM))
            listener.bind(("127.0.0.1", 0))
            ports[role] = listener.getsockname()[1]
        validate_frontend_ports(ports)
        return ports


def frontend_test_command(binary, listing, ports=None):
    if type(listing) is not bool:
        fail("invalid frontend test command kind")
    if listing:
        if ports is not None:
            fail("frontend test listing cannot use runtime ports")
        return network_command([binary, FRONTEND_TEST_PREFIX, "--list"])
    validate_frontend_ports(ports)
    return ["/usr/bin/sandbox-exec", "-p", FRONTEND_POLICY.format(**ports), str(binary), FRONTEND_TEST_PREFIX, "--test-threads=1"]


def frontend_test_environment(environment, ports):
    validate_frontend_ports(ports)
    if any(key.startswith("FOKS_B3_") for key in environment):
        fail("ambient frontend test configuration is forbidden")
    return {**environment, **{FRONTEND_PORT_ENV[role]: str(ports[role]) for role in FRONTEND_ROLES}}


def verify_frontend_test_artifact(stdout_path, graph, root=ROOT, stage=FRONTEND):
    target = graph.get("node_test_target")
    if stage not in (FRONTEND, OBSERVATIONS) or not isinstance(target, dict) or graph.get("profile") != stage or graph.get("mode") != stage:
        fail("missing frontend library test target graph")
    artifacts, finished, consumed = [], [], 0
    path = Path(stdout_path)
    if path.is_symlink() or not path.is_file():
        fail("invalid frontend compiler output")
    with path.open("rb") as stream:
        while line := stream.readline(4 * 1024 * 1024 + 1):
            consumed += len(line)
            if consumed > 128 * 1024 * 1024 or len(line) > 4 * 1024 * 1024:
                fail("frontend compiler message allowance exceeded")
            message = node_json(line)
            if not isinstance(message, dict):
                fail("unclassified frontend compiler message")
            if message.get("reason") == "compiler-artifact" and message.get("executable") is not None:
                actual = message.get("target", {})
                if message.get("package_id") != target["package_id"] or message.get("features") != target["features"] or any(actual.get(key) != target[key] for key in ("name", "kind", "crate_types", "src_path")) or message.get("profile", {}).get("test") is not True or not isinstance(message["executable"], str):
                    fail("unexpected frontend test executable artifact")
                artifacts.append(message["executable"])
            elif message.get("reason") == "build-finished":
                if type(message.get("success")) is not bool:
                    fail("frontend compiler completion is not a boolean")
                finished.append(message["success"])
            elif message.get("reason") not in ("compiler-artifact", "compiler-message", "build-script-executed"):
                fail("unclassified frontend compiler message reason")
    if len(artifacts) != 1 or finished != [True]:
        fail("missing unique successful frontend test artifact")
    binary = Path(artifacts[0])
    directory = root / "runtime/target" / TARGET / "debug/deps"
    if binary.parent != directory or not re.fullmatch(r"celld-[0-9a-f]{16}", binary.name):
        fail("frontend test executable escapes the exact target directory")
    for parent in (root / "runtime", root / "runtime/target", root / "runtime/target" / TARGET, directory.parent, directory):
        if parent.is_symlink() or not parent.is_dir():
            fail("frontend test target directory is not regular")
    if binary.resolve() != binary:
        fail("frontend test artifact is not canonical")
    image, _ = stable_fixture_file(binary, 4 * 1024**3)
    if not image["bytes"] or not image["mode"] & 0o111:
        fail("frontend test artifact is not executable")
    return {"path": str(binary), "bytes": image["bytes"], "sha256": image["sha256"], "artifact": target.copy()}


def foundation_test_output(path, listing, stage=FOUNDATION):
    if stage not in NODE_STAGES:
        fail("unsupported component test stage")
    expected = {FOUNDATION: FOUNDATION_TESTS, LIFETIMES: LIFETIMES_TESTS, FRONTEND: FRONTEND_TESTS, OBSERVATIONS: OBSERVATIONS_TESTS}[stage]
    return component_test_output(path, listing, expected)


def component_test_output(path, listing, expected):
    # Fixed caller inventories only. The output is never used to choose tests.
    count = len(expected)
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1024 * 1024:
        fail("invalid foundation test output")
    lines = [line for line in path.read_text().splitlines() if line]
    names, totals = [], []
    for line in lines:
        if listing and line.endswith(": test"):
            names.append(line[:-6])
        elif listing and line == str(count) + " tests, 0 benchmarks":
            totals.append(line)
        elif not listing and line.startswith("test ") and line.endswith(" ... ok"):
            names.append(line[5:-7])
        elif not listing and line == "running " + str(count) + " tests":
            totals.append(line)
        elif not listing and re.fullmatch(r"test result: ok\. " + str(count) + r" passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9]+\.[0-9]+s", line):
            totals.append(line)
        else:
            fail("unexpected foundation test output")
    if sorted(names) != sorted(expected) or len(names) != count or len(totals) != (1 if listing else 2):
        fail("foundation tests missing, duplicated, ignored or incomplete")
    return names


def compiled_layout_record(path, expected_stderr_sha256, prefix, maximum_record):
    """Read one fixed-tag record from the unchanged owned-command stderr."""
    if not isinstance(expected_stderr_sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_stderr_sha256):
        fail("invalid foundation layout stderr hash")
    path = Path(path)
    maximum = 1024 * 1024
    try:
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode) or not 0 <= before.st_size <= maximum:
            fail("invalid or oversized foundation layout stderr")
        identity = fixture_stat_identity(before)
        # NONBLOCK prevents a replaced special file from blocking before the
        # descriptor check; regular-file reads retain their ordinary semantics.
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as stream:
            if fixture_stat_identity(os.fstat(stream.fileno())) != identity:
                fail("foundation layout stderr replaced before read")
            raw = stream.read(maximum + 1)
            if fixture_stat_identity(os.fstat(stream.fileno())) != identity:
                fail("foundation layout stderr changed during read")
        if len(raw) != before.st_size or len(raw) > maximum or fixture_stat_identity(path.lstat()) != identity:
            fail("foundation layout stderr replaced or resized")
    except OSError as error:
        fail("cannot read foundation layout stderr: " + str(error))
    if digest(raw) != expected_stderr_sha256:
        fail("foundation layout stderr differs from owned command")

    lines = raw.split(b"\n")
    records = []
    for index, line in enumerate(lines):
        if prefix.rstrip(b" ") not in line:
            continue
        if index == len(lines) - 1 or not line.startswith(prefix) or b"\r" in line or len(line) + 1 > maximum_record:
            fail("invalid foundation layout record framing")
        records.append(line + b"\n")
    if len(records) != 1:
        fail("foundation layout record missing or duplicated")
    raw_record = records[0]

    def unique(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                fail("duplicate foundation layout JSON field: " + key)
            value[key] = item
        return value

    def refuse_number(_):
        fail("foundation layout numbers must be finite integers")

    record = json.loads(raw_record[len(prefix):-1].decode("utf-8"), object_pairs_hook=unique, parse_float=refuse_number, parse_constant=refuse_number)
    return record, raw_record


def foundation_layout_output(path, expected_stderr_sha256, stage=FOUNDATION):
    """Keep the accepted B1 schema while recording this build's actual sizes."""
    if type(stage) is not str or stage not in NODE_STAGES:
        fail("unsupported foundation layout stage")
    cleanup_reserve = 204800 if stage == FOUNDATION else LIFETIMES_LIMITS["cleanup_reserve"]
    record, raw_record = compiled_layout_record(path, expected_stderr_sha256, b"FOKS_B1_LAYOUT ", 4096)
    fixed = {"format": 1, "claim": "b1-native-layouts-only", "arc_convention": "repr-c-two-atomic-usize", "role_caps": [1024 * 1024, 8 * 1024 * 1024, 64 * 1024], "role_counts": [1, 2, 1], "cleanup_reserve": cleanup_reserve, "weak_limit": 66}
    closed_fields(record, (*fixed, "layouts", "initial_process_debt"), "foundation layout record")
    exact_json({name: record[name] for name in fixed}, fixed, "foundation layout contract")
    layout_names = ("request_arc", "control_arc", "registry_arc", "owner_factory", "original_owner", "header_cell", "cleanup_credits", "control_observation", "node_arc")
    layouts = record["layouts"]
    closed_fields(layouts, layout_names, "foundation native layouts")
    for name in layout_names:
        value = layouts[name]
        closed_fields(value, ("bytes", "alignment"), "foundation native layout")
        size, alignment = value["bytes"], value["alignment"]
        if type(size) is not int or not 0 < size <= 64 * 1024 or type(alignment) is not int or not 0 < alignment <= 16 or alignment & (alignment - 1) or size % alignment:
            fail("invalid foundation native layout size or alignment")
    if layouts["request_arc"]["bytes"] > 4096:
        fail("foundation Request Arc layout exceeds allowance")
    debt = 67 * layouts["request_arc"]["bytes"] + layouts["control_arc"]["bytes"] + layouts["registry_arc"]["bytes"]
    if type(record["initial_process_debt"]) is not int or record["initial_process_debt"] != debt:
        fail("foundation initial process debt differs from measured layouts")
    return {"record": record, "raw_record_bytes": len(raw_record), "raw_record_sha256": digest(raw_record)}


def lifetimes_layout_output(path, expected_stderr_sha256):
    """Validate real compiled layouts against the closed B2 admission table."""
    record, raw_record = compiled_layout_record(path, expected_stderr_sha256, b"FOKS_B2_LAYOUT ", 65536)
    closed_fields(record, LIFETIMES_LAYOUT_FIELDS, "lifetimes layout record")
    fixed = {"format": 1, "claim": "b2-constructor-layouts-only", "arc_convention": "repr-c-two-atomic-usize", "limits": LIFETIMES_LIMITS}
    exact_json({name: record[name] for name in fixed}, fixed, "lifetimes layout contract")

    def layout(size, alignment, ceiling, positive=False):
        # All operands are bounded before addition. On the pinned 64-bit
        # target the largest permitted total is far below usize overflow.
        if type(size) is not int or not int(positive) <= size <= ceiling or type(alignment) is not int or not 0 < alignment <= 16 or alignment & (alignment - 1) or size % alignment or (not size and alignment != 1):
            fail("invalid lifetimes layout size or alignment")

    def round_up(value, alignment):
        return (value + alignment - 1) // alignment * alignment

    closed_fields(record["fixed"], LIFETIMES_FIXED_POLICY, "lifetimes fixed layouts")
    for name, (bucket, arc, ceiling) in LIFETIMES_FIXED_POLICY.items():
        entry = record["fixed"][name]
        closed_fields(entry, LIFETIMES_FIXED_FIELDS, "lifetimes fixed layout")
        size, alignment = entry["bytes"], entry["alignment"]
        layout(size, alignment, ceiling, positive=True)
        arc_bytes = round_up(round_up(16, alignment) + size, max(8, alignment)) if arc else 0
        if arc_bytes > ceiling or (name == "a2_events" and size % 512):
            fail("lifetimes fixed allocation exceeds its policy")
        exact_json({key: entry[key] for key in ("arc_bytes", "bucket")}, {"arc_bytes": arc_bytes, "bucket": bucket}, "lifetimes fixed allocation policy")

    closed_fields(record["constructors"], LIFETIMES_CONSTRUCTOR_POLICY, "lifetimes constructors")
    for name, (rule, lanes) in LIFETIMES_CONSTRUCTOR_POLICY.items():
        entry = record["constructors"][name]
        closed_fields(entry, LIFETIMES_CONSTRUCTOR_FIELDS, "lifetimes constructor")
        component_ceiling = {"R": 49152, "I": 8192, "N": 512}[rule]
        for field in ("capture", "future", "result", "channel_payload"):
            layout(entry[field + "_bytes"], entry[field + "_alignment"], component_ceiling)
        capture, future, result, payload = (entry[field + "_bytes"] for field in ("capture", "future", "result", "channel_payload"))
        typed = capture + future + result
        policy = lifetimes_constructor_policy(rule, lanes)
        if rule == "I":
            if payload or typed > 8192:
                fail("lifetimes inline constructor has a channel or exceeds layout allowance")
            channel, owned, overlap = 0, 0, 0
        else:
            if not payload:
                fail("lifetimes allocated result channel has no payload layout")
            channel = 80 + round_up(payload, 16)
            if rule == "R":
                if typed > 49152 or channel > 8192:
                    fail("lifetimes root exceeds typed or owned partition")
                owned = 8192 - channel
                overlap = typed + channel + owned
            else:
                if future:
                    fail("lifetimes native child cannot own an async future")
                owned = 0
                overlap = capture + result + channel
            if overlap > policy["ceiling_bytes"]:
                fail("lifetimes constructor overlap exceeds admitted ceiling")
        expected = {key: value for key, value in policy.items() if key != "rule"}
        expected.update(channel_bound_bytes=channel, owned_capacity_bound_bytes=owned, admitted_overlap_bytes=overlap)
        exact_json({key: entry[key] for key in expected}, expected, "lifetimes constructor policy")

    closed_fields(record["controls"], LIFETIMES_CONTROL_NAMES, "lifetimes controls")
    for entry in record["controls"].values():
        closed_fields(entry, LIFETIMES_CONTROL_FIELDS, "lifetimes control")
        layout(entry["bytes"], entry["alignment"], 2048, positive=True)
        exact_json(entry["ceiling_bytes"], 2048, "lifetimes control ceiling")
    return {"record": record, "raw_record_bytes": len(raw_record), "raw_record_sha256": digest(raw_record)}


def frontend_layout_output(path, expected_stderr_sha256):
    record, raw_record = compiled_layout_record(path, expected_stderr_sha256, b"FOKS_B3_LAYOUT ", 16384)
    closed_fields(record, FRONTEND_LAYOUT_FIELDS, "frontend layout record")
    fixed = {"format": 1, "claim": "b3-frontend-layouts-only", "watch_allocation_bound": 4096, "frame_bytes": [32, 4194304], "process_precharge": FRONTEND_PROCESS_PRECHARGE, "original_invocation_limit": 8388608}
    exact_json({name: record[name] for name in fixed}, fixed, "frontend layout contract")

    def layout(size, alignment, ceiling, positive=False):
        if type(size) is not int or not int(positive) <= size <= ceiling or type(alignment) is not int or not 0 < alignment <= 16 or alignment & (alignment - 1) or size % alignment or (size == 0 and alignment != 1):
            fail("invalid frontend layout size or alignment")

    closed_fields(record["constructors"], FRONTEND_LAYOUT_POLICY, "frontend constructors")
    for name, policy in FRONTEND_LAYOUT_POLICY.items():
        entry = record["constructors"][name]
        closed_fields(entry, FRONTEND_CONSTRUCTOR_FIELDS, "frontend constructor")
        for field in ("capture", "future", "result"):
            layout(entry[field + "_bytes"], entry[field + "_align"], policy["ceiling_bytes"])
        typed = entry["capture_bytes"] + entry["future_bytes"] + entry["result_bytes"]
        if type(entry["typed_bytes"]) is not int or entry["typed_bytes"] != typed or typed > policy["ceiling_bytes"]:
            fail("frontend constructor exceeds additive typed allowance")
    for field, names in (("fixed", FRONTEND_FIXED_LAYOUTS), ("watch_components", FRONTEND_WATCH_COMPONENTS)):
        closed_fields(record[field], names, "frontend " + field)
        for entry in record[field].values():
            closed_fields(entry, ("bytes", "align"), "frontend fixed layout")
            layout(entry["bytes"], entry["align"], 4096, positive=True)
    components = record["watch_components"]
    # Public selected-type measurements bound the private Tokio Shared<bool>:
    # nine Notify, the RwLock<bool>, three atomics, one optional index, and
    # two Arc counters. Rounding each constituent to 16 covers all padding.
    rounded = {name: (entry["bytes"] + 15) // 16 * 16 for name, entry in components.items()}
    derived = 9 * rounded["notify"] + rounded["rwlock_bool"] + 6 * rounded["atomic_usize"]
    if components["atomic_usize"] != {"bytes": 8, "align": 8} or type(record["watch_derived_bytes"]) is not int or record["watch_derived_bytes"] != derived or derived > 4096:
        fail("frontend watch allocation bound is not supported by its constituents")
    return {"record": record, "raw_record_bytes": len(raw_record), "raw_record_sha256": digest(raw_record)}


def observation_layout_output(path, expected_stderr_sha256):
    """P1 measurements are additive, source-bound, and share the old log hash."""
    policy = observation_layout_contract()
    record, raw_record = compiled_layout_record(path, expected_stderr_sha256, policy["prefix"].encode("ascii"), policy["maximum_record_bytes"])
    closed_fields(record, policy["record_fields"], "observation layout record")
    exact_json(record["format"], 1, "observation layout format")
    layouts = record["layouts"]
    closed_fields(layouts, policy["layout_ids"], "observation layouts")
    for name, limits in policy["layout_policy"].items():
        entry = layouts[name]
        closed_fields(entry, policy["layout_fields"], "observation layout")
        size, alignment = entry["bytes"], entry["alignment"]
        if type(size) is not int or not 0 < size <= limits["ceiling_bytes"] or type(alignment) is not int or not 0 < alignment <= limits["maximum_alignment"] or alignment & (alignment - 1) or size % alignment:
            fail("invalid observation layout size or alignment")
    if layouts["archive"]["bytes"] != 64 * layouts["invocation"]["bytes"] or layouts["archive"]["alignment"] != layouts["invocation"]["alignment"]:
        fail("observation archive differs from its actual fixed array")
    events = record["a2_event_bytes"]
    if type(events) is not int or not 0 < events <= 65536 or events % 512:
        fail("invalid observation A2 event backing size")
    # These two measurements name the same actual allocations as B2. A second
    # smaller row cannot substitute for the already measured backing/Arc.
    b2 = lifetimes_layout_output(path, expected_stderr_sha256)["record"]["fixed"]
    if events != b2["a2_events"]["bytes"] or layouts["registry_arc"]["bytes"] != b2["effect_registry"]["arc_bytes"] or layouts["registry_arc"]["alignment"] != max(8, b2["effect_registry"]["alignment"]) or layouts["registry_arc"]["bytes"] < layouts["archive"]["bytes"]:
        fail("observation and B2 allocation measurements disagree")
    retained = layouts["archive"]["bytes"] + 65536 + layouts["a2_chunk"]["bytes"] + 6 * layouts["invocation"]["bytes"] + layouts["report"]["bytes"] + 1024 + events
    if type(record["retained_bytes"]) is not int or record["retained_bytes"] != retained or retained > policy["retained_bytes_maximum"]:
        fail("observation retained storage exceeds the measured overlap")
    return {"record": record, "raw_record_bytes": len(raw_record), "raw_record_sha256": digest(raw_record)}


def foundation_probe_specs(mode):
    if mode not in NODE_BINARY_MODES:
        fail("unknown node binary probe mode")
    probes = {"version": (["--version"], 0), "help": (["--help"], 0), "invalid-option": (["--foks-guarded-node-qualification"], 1)}
    if mode in (FRONTEND, OBSERVATIONS):
        probes.update({name: (FRONTEND_REFUSALS[name][0], 1) for name in ("frontend-missing-scenario", "frontend-unsupported-scenario")})
    elif mode in (FRONTEND + "-default", OBSERVATIONS + "-default"):
        probes["frontend-absent"] = (FRONTEND_REFUSALS["frontend-absent"][0], 1)
    return probes


def foundation_probe_output(directory, name, mode=None):
    if mode is not None and mode not in NODE_BINARY_MODES:
        fail("unknown node binary probe mode")
    allowed = foundation_probe_specs(mode) if mode is not None else ("version", "help", "invalid-option")
    if name not in allowed:
        fail("unknown foundation binary probe")
    outputs = {}
    for stream in ("stdout", "stderr"):
        path = directory / (name + "." + stream)
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 65536:
            fail("foundation probe output allowance exceeded")
        outputs[stream] = path.read_bytes()
        if len(outputs[stream]) > 65536:
            fail("foundation probe output allowance exceeded")
    out, err = outputs["stdout"], outputs["stderr"]
    warnings = []
    # Jemalloc's background_thread_boot0 can emit this exact stderr line at
    # allocator initialization, before even the invalid-option parse error.
    if err.startswith(FOUNDATION_JEMALLOC_WARNING):
        warnings.append({"id": "jemalloc-background-thread-unsupported", "stream": "stderr", "line": 1, "text": FOUNDATION_JEMALLOC_WARNING.decode("ascii").removesuffix("\n")})
        err = err[len(FOUNDATION_JEMALLOC_WARNING):]
    if name in ("version", "help"):
        # main parses Action before installing tracing and calling
        # memory::tune_allocator. Its single warning is therefore impossible
        # for a parse refusal. For help/version, the lossy asynchronous writer
        # may omit or reorder this complete line relative to the CLI answer.
        # Recognize only the pinned message and default microsecond timestamp;
        # never strip arbitrary log lines, whitespace, or partial messages.
        lines, memory_warnings = [], 0
        for number, line in enumerate(out.splitlines(keepends=True), 1):
            if FOUNDATION_MEMORY_WARNING.fullmatch(line):
                memory_warnings += 1
                warnings.append({"id": "celld-allocator-background-thread-unavailable", "stream": "stdout", "line": number, "text": line.decode("ascii").removesuffix("\n")})
            else:
                lines.append(line)
        if memory_warnings > 1:
            fail("duplicate foundation allocator warning")
        out = b"".join(lines)
    if name == "version":
        accepted = out == b"celld 0.6.2 (debug)\n" and not err
    elif name == "help":
        accepted = (len(out), digest(out)) == (FRONTEND_HELP_IMAGE if mode in (FRONTEND, OBSERVATIONS) else FOUNDATION_HELP_IMAGE) and not err
    elif name == "invalid-option":
        accepted = not out and err == b"Error: unknown command or option: --foks-guarded-node-qualification; run `celld --help` for usage\n"
    else:
        accepted = not out and err == FRONTEND_REFUSALS[name][1]
    if not accepted:
        fail("foundation binary probe outcome mismatch: " + name)
    # Raw files are untouched; these hashes/lengths refer to them, not to the
    # temporary exact-warning projection used by the outcome comparison.
    return {"stdout_bytes": len(outputs["stdout"]), "stdout_sha256": digest(outputs["stdout"]), "stderr_bytes": len(outputs["stderr"]), "stderr_sha256": digest(outputs["stderr"]), "recognized_warnings": warnings}


def run_frontend_components(focused, root, toolchain, pins, graph, environment, directory, slot):
    stage = pins["profile"]
    if stage not in (FRONTEND, OBSERVATIONS):
        fail("unsupported direct component stage")
    timeout = pins["build"]["timeout_seconds"]
    focused["compile"] = run_owned(frontend_test_compile_command(root, toolchain, pins), root, environment, directory, "tests-compile", slot, timeout, lambda: resource_refusal(root))
    if file_hash(directory / "tests-compile.stdout") != focused["compile"]["stdout_sha256"]:
        fail("frontend compiler output changed after owned command")
    focused["binary"] = verify_frontend_test_artifact(directory / "tests-compile.stdout", graph, root, stage)
    binary = Path(focused["binary"]["path"])
    checked_file(binary, focused["binary"])
    focused["list"] = run_owned(frontend_test_command(binary, True), root, environment, directory, "tests-list", slot, timeout, lambda: resource_refusal(root))
    if file_hash(directory / "tests-list.stdout") != focused["list"]["stdout_sha256"]:
        fail("frontend test listing changed after owned command")
    foundation_test_output(directory / "tests-list.stdout", True, stage)
    checked_file(binary, focused["binary"])
    ports = choose_frontend_ports()
    focused["ports"] = ports
    focused["port_environment"] = {FRONTEND_PORT_ENV[role]: str(ports[role]) for role in FRONTEND_ROLES}
    focused["network_policy_sha256"] = digest(FRONTEND_POLICY.format(**ports).encode())
    # Save the chosen policy/ports before launching. Ownership stays with the
    # same slot and supervisor; only this direct test child gets loopback.
    write_json(directory / "frontend-test-inputs.json", {"binary": focused["binary"], "ports": ports, "environment": focused["port_environment"], "policy": FRONTEND_POLICY.format(**ports)})
    focused["run"] = run_owned(frontend_test_command(binary, False, ports), root, frontend_test_environment(environment, ports), directory, "tests-run", slot, timeout, lambda: resource_refusal(root))
    checked_file(binary, focused["binary"])
    if file_hash(directory / "tests-run.stdout") != focused["run"]["stdout_sha256"]:
        fail("frontend test output changed after owned command")
    foundation_test_output(directory / "tests-run.stdout", False, stage)
    stderr, checksum = directory / "tests-run.stderr", focused["run"]["stderr_sha256"]
    focused["layouts"] = foundation_layout_output(stderr, checksum, stage)
    focused["lifetimes_layouts"] = lifetimes_layout_output(stderr, checksum)
    focused["frontend_layouts"] = frontend_layout_output(stderr, checksum)
    if stage == OBSERVATIONS:
        focused["observation_layouts"] = observation_layout_output(stderr, checksum)
    focused["transport_tests"] = {}
    for group, names in FRONTEND_TRANSPORT_TESTS.items():
        result = {"names": names.copy(), "threads": 1, "network": "deny-all", "scope": "root-workspace-a0-regression", "manifest": str(root.parents[1] / "Cargo.toml")}
        focused["transport_tests"][group] = result
        for action, argv in frontend_transport_commands(root, toolchain, group).items():
            name = "transport-" + group + "-" + action
            result[action] = run_owned(argv, root, environment, directory, name, slot, timeout, lambda: resource_refusal(root))
            path = directory / (name + ".stdout")
            if file_hash(path) != result[action]["stdout_sha256"]:
                fail("frontend transport test output changed after owned command")
            component_test_output(path, action == "list", names)
    checked_file(binary, focused["binary"])


def build_foundation(graph_directory, root, mode):
    import check_graph
    stage = stage_for_mode(mode)
    if stage not in NODE_STAGES:
        fail("unsupported node component build stage")
    pins = load_pins(root, stage)
    selected_mode(pins, mode)
    foundation_lock(root, pins)
    source, assets = verify_source(root, stage), verify_assets(root, stage)
    graph = check_graph.verify(graph_directory, root, mode)
    refusal, measured = resource_refusal(root, initial=True)
    if refusal:
        fail(refusal + ": " + json.dumps(measured))
    toolchain, environment = controlled_environment(root)
    evidence_root = runtime_directory(root) / "evidence"
    evidence_root.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix={FOUNDATION: "foundation-", LIFETIMES: "lifetimes-", FRONTEND: "frontend-", OBSERVATIONS: "observations-"}[stage], dir=evidence_root))
    claims = {"node-foundation-default": "b1-default-bin-regression", "node-foundation": "b1-foundation-component-only", "node-foundation-lifecycle": "b1-a2-lifecycle-regression", "node-lifetimes-default": "b2-default-bin-regression", "node-lifetimes": "b2-lifetimes-components-only", "node-lifetimes-lifecycle": "b2-a2-lifecycle-regression"}
    claims.update({FRONTEND + "-default": "b3-default-bin-regression", FRONTEND: "b3-frontend-components-only", FRONTEND + "-lifecycle": "b3-a2-lifecycle-regression"})
    claims.update({OBSERVATIONS + "-default": "b4-p1-default-bin-regression", OBSERVATIONS: "b4-p1-observation-components-only", OBSERVATIONS + "-lifecycle": "b4-p1-a2-lifecycle-regression"})
    receipt = {"format": 1, "profile": stage, "mode": mode, "claim": claims[mode], "status": "incomplete", "source": source, "assets": assets, "profile_sha256": file_hash(root / FOUNDATION_PROFILE), "series_sha256": pins["foundation"]["node_series"]["sha256"], "cargo_lock_sha256": file_hash(root / "Cargo.lock"), "graph_directory": str(Path(graph_directory).resolve()), "graph_sha256": digest(json.dumps(graph, sort_keys=True).encode()), "initial_resources": measured, "not_qualified": OBSERVATIONS_EXCLUSIONS if stage == OBSERVATIONS else FRONTEND_EXCLUSIONS if stage == FRONTEND else FOUNDATION_EXCLUSIONS}
    write_json(directory / "receipt.json", receipt)
    with BuildSlot(root) as slot:
        try:
            receipt["tools"] = tool_identity(root, directory, slot, environment, toolchain)
            commands = build_commands(root, toolchain, pins, mode)
            for name in ("build", "clippy"):
                receipt[name] = run_owned(commands[name], root, environment, directory, name, slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
            if mode in NODE_LIFECYCLE_MODES:
                binary = root / "runtime/target" / TARGET / "debug/foks-celld-guarded"
                image, _ = stable_fixture_file(binary, 4 * 1024**3)
                if not image["bytes"] or not image["mode"] & 0o111:
                    fail("foundation lifecycle artifact is not executable")
                receipt["binary"] = {"path": str(binary), "bytes": image["bytes"], "sha256": image["sha256"]}
                receipt["identity"] = run_owned(network_command([binary, "identity"]), root, environment, directory, "identity", slot, pins["build"]["identity_timeout_seconds"], lambda: resource_refusal(root))
                identity = read_json(directory / "identity.stdout", 65536)
                if identity.get("claim") != "a2-library-identity-only" or identity.get("input_sha256") != file_hash(root / "runtime.lock.json") or identity.get("actual_shared_request_exercised") is not True or any(not identity.get(field) for field in ("v8_engine", "sqlite_source_id", "sqlite_version")):
                    fail("foundation lifecycle identity does not match unchanged A2 recipe")
                receipt["identity_output"] = identity
                receipt["lifecycle"] = run_owned(commands["lifecycle"], root, environment, directory, "lifecycle", slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
            else:
                receipt["binary"] = verify_node_artifact(directory / "build.stdout", graph, root)
                binary = Path(receipt["binary"]["path"])
                receipt["probes"] = {}
                for name, (arguments, expected_exit) in foundation_probe_specs(mode).items():
                    checked_file(binary, receipt["binary"])
                    command = run_owned(network_command([binary, *arguments]), root, environment, directory, name, slot, 15, lambda: resource_refusal(root), maximum_log_bytes=65536, expected_exit_code=expected_exit)
                    receipt["probes"][name] = {"command": command, "output": foundation_probe_output(directory, name, mode)}
                    checked_file(binary, receipt["binary"])
                if mode == stage:
                    focused = {"prefix": FOUNDATION_TEST_PREFIX if stage == FOUNDATION else LIFETIMES_TEST_PREFIX, "names": {FOUNDATION: FOUNDATION_TESTS, LIFETIMES: LIFETIMES_TESTS, FRONTEND: FRONTEND_TESTS, OBSERVATIONS: OBSERVATIONS_TESTS}[stage], "threads": 1}
                    receipt["focused_tests"] = focused
                    if stage in (FRONTEND, OBSERVATIONS):
                        run_frontend_components(focused, root, toolchain, pins, graph, environment, directory, slot)
                    for name, argv in ({} if stage in (FRONTEND, OBSERVATIONS) else foundation_test_commands(root, toolchain, pins)).items():
                        focused[name] = run_owned(argv, root, environment, directory, "tests-" + name, slot, pins["build"]["timeout_seconds"], lambda: resource_refusal(root))
                        foundation_test_output(directory / ("tests-" + name + ".stdout"), name == "list", stage)
                        if name == "run":
                            focused["layouts"] = foundation_layout_output(directory / "tests-run.stderr", focused[name]["stderr_sha256"], stage)
                            if stage == LIFETIMES:
                                focused["lifetimes_layouts"] = lifetimes_layout_output(directory / "tests-run.stderr", focused[name]["stderr_sha256"])
            checked_file(binary, receipt["binary"])
            if verify_source(root, stage) != source or verify_assets(root, stage) != assets or check_graph.verify(graph_directory, root, mode) != graph:
                fail("foundation qualification inputs changed")
            if config_inputs(root) != receipt["tools"]["cargo_configs"] or any(file_hash(path) != expected for path, expected in receipt["tools"]["native_tools"].items()):
                fail("foundation native tools/configuration changed")
            controlled_environment(root)
            refusal, _ = resource_refusal(root)
            if refusal:
                fail(refusal)
            checked_file(binary, receipt["binary"])
            receipt["status"] = "passed"
        except BaseException as error:
            receipt["error"] = str(error)
            receipt["status"] = "failed" if slot.clean else "cleanup uncertain"
            raise
        finally:
            receipt["final_resources"] = disk_status(root)
            write_json(directory / "receipt.json", receipt)
    return {"directory": str(directory), "receipt_sha256": file_hash(directory / "receipt.json"), "status": receipt["status"]}


def build(graph_directory, root=ROOT, mode="identity"):
    if stage_for_mode(mode) in NODE_STAGES:
        return build_foundation(graph_directory, root, mode)
    if mode == "node-baseline":
        return build_node(graph_directory, root)
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
        operation = sub.add_parser(name, help=help_text)
        operation.add_argument("archive", type=Path)
        operation.add_argument("--stage", choices=NODE_STAGES)
    assets = sub.add_parser("assets")
    assets.add_argument("archive", type=Path)
    assets.add_argument("binding", type=Path)
    assets.add_argument("--stage", choices=NODE_STAGES)
    sub.add_parser("verify-source").add_argument("--stage", choices=NODE_STAGES)
    sub.add_parser("verify-assets").add_argument("--stage", choices=NODE_STAGES)
    build_parser = sub.add_parser("build")
    build_parser.add_argument("graph_directory", type=Path)
    build_parser.add_argument("--mode", choices=("identity", "lifecycle", "node-baseline", *NODE_MODES), default="identity")
    args = parser.parse_args()
    try:
        if args.operation == "verify":
            report = inspect_archive(args.archive, stage=args.stage)
        elif args.operation == "prepare":
            report = prepare(args.archive, stage=args.stage)
        elif args.operation == "assets":
            report = prepare_assets(args.archive, args.binding, stage=args.stage)
        elif args.operation == "verify-source":
            report = verify_source(stage=args.stage)
        elif args.operation == "verify-assets":
            report = verify_assets(stage=args.stage)
        else:
            report = build(args.graph_directory, mode=args.mode)
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, TypeError, KeyError, EOFError, tarfile.TarError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

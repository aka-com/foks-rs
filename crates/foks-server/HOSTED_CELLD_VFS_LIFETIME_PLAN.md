# Candidate: make paged VFS retirement safe for ordinary hydration

Status: source-backed candidate for a separate plan/review/change. No implementation, execution or crash reproduction. This candidate is not yet part of a reviewed implementation phase. Sources are the reconstructed B1 tree at `tools/foks-celld-guarded/runtime/source` in the validation worktree.

## Findings

`crates/celld/ltx_repl.rs:2800–2866` starts an asynchronous hydration root, opens its own SQLite connection in a blocking job, and checks `cancelled` only between blocking steps. `close_removed:3601–3630` sets that flag, closes the managed database, unregisters the VFS and removes/renames local files without joining the hydration root or current step. Cancellation therefore does not exclude an in-flight step at unregistration.

`crates/ltx/src/paged_vfs.rs:600–668` resolves the registration repeatedly by name. The loop at 636–642 dereferences `sqlite3_vfs_find` without checking for null. A source-permitted interleaving is: step passes its initial lookup/count; removal unregisters the name; step reaches the next unchecked lookup. An open file can retain the registration allocation while the name lookup still returns null. This is a concrete unsafe path; its timing has not been reproduced.

There is a distinct two-variable reclamation race. `x_close:241–258` decrements `open_files` and then reads `retired`. `unregister_paged_vfs:671–688` stores `retired`, reads `open_files` and can free the same allocation. A possible order is close decrements 1→0; unregister stores true, sees zero and frees; close then reads freed `app.retired`. Depending on timing, duplicate reclamation is also possible. This interleaving is source reasoning, not an observed runtime failure.

## Minimum ownership change to review

1. Make `Hydrator::step` use the actual main-file state retained by its own live connection, not repeated registration-name lookups. Obtain `SQLITE_FCNTL_FILE_POINTER`, verify the paged I/O-method identity and a main-file reader before casting, then use that file's reader/shared hydration set. A private helper can calculate progress from these pinned inputs. Preserve the existing retirement error using a safely pinned retired state; do not claim that checking it cancels an I/O already underway. A `Hydrator` for an unrelated VFS/file must refuse, not cast unchecked.
2. Replace the independent count/retired reclamation decision with one ownership transition. A concrete small design is one reference count initialized with the registration's reference, plus one reference per open file; unregister releases the registration reference exactly once, and each close releases its own reference. Only the 1→0 winner frees, and every nonwinning releaser performs no further access to that allocation. A separate retired flag may express admission policy but cannot decide reclamation. Keep the connection/file reference until all relevant close callbacks have finished.
3. Before implementation, freeze the lookup/open/retire admission contract as well. A registration reference is not enough if `sqlite3_vfs_find` can hand a raw pointer to a new opener or `hydration(name)` observer while retirement frees it. Audit the existing call sites and protect these borrows with a concrete registration lease/serialized registry transition, or explicitly require and enforce the caller's no-new-open boundary. Also make duplicate unregister release the registration reference at most once. Do not present the two changes above as a complete global lookup/open safety proof without this gate.

The production change should remain centered on `crates/ltx/src/paged_vfs.rs`; `ltx_repl.rs` is the source of the concurrent ordinary caller and any required caller-side admission adjustment must be separately frozen. Preserve SQL, page selection, FOKS wire behavior and ordinary successful hydration. Do not add process-lifetime leaks to avoid freeing.

## Deterministic validation to freeze with the change

- Use a bounded PageReader and barriers to pause a genuine Hydrator after acquiring its live main-file state but before a later page decision. Unregister concurrently, resume, and require the defined retirement error or consistent pinned read, with no null lookup/cast. Check the original bytes and no access to a replacement registration.
- Pause a final-close release and unregister at the ownership boundary in both orders. Track the actual registration allocation's destructor and require exactly one destruction after both references are released, never before a retained connection closes. Keep a separate valid WAL/main-file close case.
- Exercise unregister before step, duplicate unregister, last close before unregister, unregister before last close, and the explicitly selected lookup/open-admission race. Failure and panic paths must dispose of each reference exactly once.
- Verify the tests actually run: the current `celld-ltx` manifest has `[lib] test = false`. Choose and freeze a runnable unit/integration target and any necessary narrowly scoped test hook; do not assume inline tests are executed. Add sanitizer checking when available, but do not substitute a timing stress loop for the barrier cases.

This repair only concerns local memory safety and ordinary hydration retirement. Guarded B2 still must close admission and join the actual hydration root/native step **before** VFS unregistration and local file retirement. Neither this fix nor a local join proves remote PUT termination or permits S3 reclamation. Lower LTX boxed closures/results also remain separate from the new wrapper-byte cap unless explicitly precharged by a size-aware hook.

## Source identities

| Path | Bytes | SHA-256 |
|---|---:|---|
| `crates/celld/ltx_repl.rs` | 240194 | `55b02a3845bc295c9994053ff32c172907c02c971eecc8afe44d0378c005928e` |
| `crates/ltx/src/paged_vfs.rs` | 28013 | `9c2b916c4d54fcc6e30730c2329c79a426869325cea58184b2c6bdccde6c0c57` |
| `crates/ltx/Cargo.toml` | 1599 | `ff9bd92d83532c8fd19b1681331f8a774f713c11b8e9cea9d32eaf836fd6f381` |

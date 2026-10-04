# Build, CI, tooling, tests and docs

Area key `build`. 14 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The repository has five GitHub workflows. Two of them run on pull_request, and both are path-filtered: foks-desktop.yml and foks-fuzz.yml. foks-standalone.yml runs only on push, nightly schedule and manual dispatch. It covers fmt, clippy -D warnings and tests for the 24 non-Tauri crates through tools/foks-{client,server}/check.sh, plus the generated-protocol check and the Go descriptor and fixture guards. The nightly full suite runs the Go v0.1.9 live-interop scripts. A weekly job checks upstream protocol drift. A nightly hosted canary publishes signed, attested leases. Tags trigger a desktop-only release that is signed, notarized and attested. Most actions are SHA-pinned, the release path fails closed, and the canary isolates its tooling. Several gaps remain. Pull-request gating depends on the push event and on path filters, so there is no single required check, and the wire-compatibility metadata check does not run on pull_request events. The desktop workflow rebuilds the whole workspace in release mode on macOS with no cache, no concurrency group and no timeouts. Nothing audits dependencies (no cargo-deny, cargo-audit, npm audit or Dependabot). Prettier is not enforced, and five files currently drift. The two flaky-test workarounds in AGENTS.md have identifiable root causes that can be fixed. The first is flock locks released only on close while other tests spawn processes concurrently; it also forces 468 tests in foks-desktop-app and foks-client-app to run serially. The second is the CLI tests finding foks-agent by its sibling path, which nothing keeps up to date. The book is accurate where I spot-checked it: the agent bounds in ch. 19, the chat-limits table in ch. 24, the crate sizes in Appendix D, and all 219 file paths cited in chapters resolve. The ISSUES.md numeric limits also match the code. The older material has drifted: docs/GUIDE.html and docs/INTRO.html still contain crypto misstatements that the book already calls out, four code and README references point to design docs that were never brought over, the src-tauri README is stale, and monorepo leftovers remain (a Bazel reference, path-boundary checks that cannot fail in practice, Chromium notes). Only the desktop app is released. foks-server, foks-agent and the foks-rs CLI have systemd units under packaging/ but no release pipeline.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [No dependency advisory, license or source auditing for 749 crates and 280 npm packages](#build-deps-supply-chain-audit) | security | high | M |
| [Desktop PR validation rebuilds the full workspace on macOS in release mode, uncached, untimed and uncancelled](#build-ci-desktop-cost) | performance | medium | M |
| [No single PR-triggered required check; clippy, fmt and protocol-metadata guards for 23 crates run only on push](#build-ci-pr-required-gate) | tooling | medium | M |
| [No SECURITY.md, CONTRIBUTING.md or prerequisite check; onboarding knowledge lives in an agent-oriented AGENTS.md](#build-contributor-security-onboarding) | docs | medium | S |
| [Replace vacuous monorepo path-boundary checks with an enforced crate-layer allowlist; document the desktop's direct client-app use](#build-crate-layer-boundaries) | maintainability | medium | M |
| [docs/GUIDE.html and docs/INTRO.html still misdescribe the KDF, the hybrid combiner and recovery-kit stretching](#build-docs-guide-crypto-misstatements) | docs | medium | S |
| [Prettier is never checked in CI, and ESLint and tsc cover only apps/desktop](#build-frontend-format-lint-gate) | tooling | medium | S |
| [No release pipeline for foks-server, foks-agent and foks-rs CLI despite shipped systemd units](#build-release-standalone-binaries) | missing-feature | medium | L |
| [CLI integration tests resolve foks-agent by sibling path, which nothing keeps current; six copies of the agent harness](#build-test-agent-binary-resolution) | testing | medium | M |
| [Root cause of the first-run receipt-lock flake: flock locks outlive drop while sibling tests spawn processes](#build-test-flock-release-race) | correctness-risk | medium | M |
| [Inconsistent action pinning, persisted checkout credentials in secret-bearing jobs, no workflow linting](#build-actions-hardening) | security | low | S |
| [README and book benchmark numbers have no committed result file or revision](#build-benchmark-provenance) | docs | low | S |
| [Broken design-doc references and stale READMEs; no automated link or path check](#build-docs-link-and-drift-check) | docs | low | M |
| [Weekly fuzz campaigns discard their corpus and run on a floating nightly](#build-fuzz-corpus-persistence) | testing | low | S |

### build-deps-supply-chain-audit

**No dependency advisory, license or source auditing for 749 crates and 280 npm packages**

- Type: security
- Priority: high
- Effort: M
- Layers: ci, tooling
- Verification: adjusted

No workflow runs cargo-deny, cargo-audit, npm audit or a license check, and there is no Dependabot or Renovate config. The session hook installs with `npm ci --no-audit`. This project ships signed desktop binaries and handles keys. The lockfile has 749 packages and 58 crates in multiple versions, including curve25519-dalek 4 and 5, sha2 0.10 and 0.11, and three syn versions. There are exact pins without rationale (`lru = "=0.18.2"`, `rusqlite = "=0.40.2"`) and an unmaintained fs2 0.4.3. The release attests provenance, but shipped binaries carry no dependency manifest that later advisories could be checked against.

**Evidence**

- [`Cargo.toml:86`](../../../Cargo.toml#L86): Exact pins for lru and rusqlite (line 90) carry no comment; fs2 = "0.4" at line 83.
- [`crates/foks-crypto/Cargo.toml:22`](../../../crates/foks-crypto/Cargo.toml#L22): `sha2 = "0.10"` beside the workspace sha2 0.11 (Cargo.toml:99). The digest-0.10 comment at line 16 sits above `getrandom` (line 17), not the sha2 line.
- `claude/hooks/session-start.sh:80`: npm ci --no-audit --no-fund.
- `github/workflows/foks-desktop.yml:154`: The release build (154-159) and attestation (214-238) run no advisory scan and produce no auditable dependency manifest.
- [`package-lock.json:1`](../../../package-lock.json#L1): 279 packages, only 5 non-dev; build tooling such as vite and @tauri-apps/cli runs in the release pipeline.

**Recommendation**

(1) Add deny.toml. [advisories] denies vulnerabilities and unsound crates and warns on unmaintained ones, with a reviewed ignore list. [licenses] allows MIT, Apache-2.0, BSD-2/3-Clause, ISC, Unicode-3.0, Zlib and MPL-2.0 (adjust after the first run) and fails on others. [bans] sets multiple-versions = "warn" and documents skips for the RustCrypto digest 0.10/0.11 split required by FOKS v0.1.9 compatibility and ed25519-dalek 2. [sources] restricts to crates.io. Run `cargo deny --locked check` on Cargo.lock and deny.toml changes and on a weekly schedule, for both Cargo.lock and fuzz/Cargo.lock. (2) Run `npm audit --audit-level=high` on the full tree whenever package-lock.json changes. Dev dependencies such as vite and @tauri-apps/cli run in the release build, so do not limit the audit to `--omit=dev`; downgrade dev-only findings to warnings if they are too noisy. (3) Add .github/dependabot.yml for cargo, npm, gomod (tools/foks-v019-oracle, tools/foks-protocol-sync) and github-actions with grouped weekly updates. (4) Build the release foks-agent (stage-foks-agent.sh) and foks-desktop (tauri build with `--runner cargo-auditable`, or an equivalent wrapper) with cargo-auditable so `cargo audit bin` can scan shipped binaries. (5) Add a one-line rationale comment beside each `=` pin and move the digest-0.10 comment onto the sha2 line.

<details><summary>Verifier note</summary>

The core claim holds. There is no deny.toml, no dependabot or renovate config, and no audit step in any workflow, and the session hook runs `npm ci --no-audit --no-fund` (session-start.sh:80). Cargo.lock has 749 packages, of which 58 crates appear in multiple versions, including curve25519-dalek 4.1.3/5.0.0, sha2 0.10.9/0.11.0 and syn 1/2/3. Exact pins lru =0.18.2 (Cargo.toml:86) and rusqlite =0.40.2 (:90) carry no rationale, and fs2 = "0.4" is at :83. package-lock.json has 279 packages. Corrections: sha2 = "0.10" is at foks-crypto/Cargo.toml:22, not 21. The comment sits at :16, directly above getrandom at :17. foks-desktop.yml:146 is a keychain step; the release build is at 154-159 and attestation at 214-238. The supply-chain split comes from the FOKS v0.1.9 digest-0.10 API requirement as well as ed25519-dalek 2. On npm, only 5 of the 279 packages are runtime (react, react-dom, @tauri-apps/api, lucide, scheduler). `npm audit --omit=dev` therefore covers almost nothing, while vite and @tauri-apps/cli execute during release builds, so build tooling should not be limited to a weekly non-blocking report.

</details>

### build-ci-desktop-cost

**Desktop PR validation rebuilds the full workspace on macOS in release mode, uncached, untimed and uncancelled**

- Type: performance
- Priority: medium
- Effort: M
- Layers: ci, tooling
- Verification: adjusted

Any PR touching apps/desktop/** triggers validate-macos, even a TypeScript or CSS-only change. That job runs `npm run test:rust:full -- --release` (the whole workspace in release mode), builds a DMG and runs the real-agent transcript. It has no Cargo cache, no concurrency group to cancel superseded pushes, and no timeout-minutes, so a hung real-agent or Xvfb step runs for 6 h on macOS minutes. For same-repo branches, the push-triggered standalone `client` job also runs the client graph on macos-14. The scripts hard-code `-j 2` for constrained containers, which underuses 4-vCPU Linux runners. tauri-driver and cargo-fuzz are compiled from source on every run.

**Evidence**

- `github/workflows/foks-desktop.yml:46`: Full workspace tests in release mode on macos-15 for every matching PR; the workflow has no cache step, concurrency group or timeout-minutes.
- `github/workflows/foks-desktop.yml:6`: 'apps/desktop/**' covers TS-only changes and triggers both native jobs.
- `github/workflows/foks-standalone.yml:46`: A second macOS job (macos-14) runs the client graph on branch pushes.
- [`scripts/test-rust-full.sh:7`](../../../scripts/test-rust-full.sh#L7): Hard-coded -j 2; also in prepare-desktop-tests.sh:13, test-rust-scale.sh:5 and foks-desktop.yml:89-92.
- `github/workflows/foks-desktop.yml:98`: `cargo install tauri-driver` compiles from source on each run; foks-fuzz.yml:20 does the same for cargo-fuzz in five matrix legs.
- `github/workflows/foks-standalone.yml:61`: Caches all of target/ under an exact key with no restore-keys, so any Cargo.lock change is a fully cold build.

**Recommendation**

(1) Split desktop PR validation by path group, using the `changes` job from the CI-gate finding. TS-only changes (apps/desktop/{src,kit,tests}, package*.json) run one ubuntu job: typecheck, eslint, prettier, test:foks-ui. src-tauri, crates, scripts and packaging changes run the Linux native and WebView jobs. The macOS bundle, packaged-startup and real-agent steps run only when src-tauri, packaging/foks-desktop or scripts/*macos* change, on tags, and on a new nightly schedule. (2) Together with (1), so the macOS PR job no longer builds a release bundle: replace `test:rust:full -- --release` with debug tests of the macOS-specific crates (foks-keystore native, foks-desktop-app, foks-client-app) and move the release-mode full run to the nightly schedule. (3) Add Swatinem/rust-cache, pinned by SHA, with `save-if: github.ref == 'refs/heads/main'`. Add restore-keys to the standalone actions/cache entries, or replace them. (4) Add `concurrency: {group: desktop-${{ github.ref }}, cancel-in-progress: ${{ github.event_name == 'pull_request' }}}` and `timeout-minutes` (Linux 60, macOS 90, fuzz 20). (5) Remove `-j 2` from the scripts and the workflow. Have AGENTS.md and the session hook export CARGO_BUILD_JOBS=2 for constrained containers; foks-fuzz.yml already uses that variable. (6) Install tauri-driver and cargo-fuzz from prebuilt binaries (a SHA-pinned taiki-e/install-action or cargo-binstall), or cache ~/.cargo/bin keyed on the pinned versions.

<details><summary>Verifier note</summary>

The core claims hold. The PR path filter 'apps/desktop/**' (foks-desktop.yml:6) triggers both native jobs for TS- or CSS-only changes. validate-macos runs `npm run test:rust:full -- --release` (line 46), the DMG bundle, the packaged startup check and the real-agent transcript. foks-desktop.yml has no actions/cache, no concurrency group and no timeout-minutes, and neither do foks-fuzz.yml or foks-standalone.yml jobs. The standalone `client` job runs on macos-14 for branch pushes (line 46). `-j 2` is hard-coded in test-rust-full.sh:7, prepare-desktop-tests.sh:13, test-rust-scale.sh:5 and foks-desktop.yml:89-92. tauri-driver is built from source at line 98 (the finding cites 97), and cargo-fuzz at foks-fuzz.yml:20 in five legs. The standalone cache uses an exact key with no restore-keys (line 61). Corrections: the Xvfb step is in the Linux job, not macOS, so it consumes Linux minutes. Running the macOS tests with --release likely reuses release dependency artifacts for the following release bundle build. Switching those tests to debug adds a second profile compile unless the bundle and real-agent steps are gated off at the same time, so (1) and (2) must land together. taiki-e/install-action may not ship a prebuilt tauri-driver (it falls back to cargo-binstall), so caching ~/.cargo/bin keyed on the pinned version is an acceptable alternative.

</details>

### build-ci-pr-required-gate

**No single PR-triggered required check; clippy, fmt and protocol-metadata guards for 23 crates run only on push**

- Type: tooling
- Priority: medium
- Effort: M
- Layers: ci
- Verification: adjusted

foks-standalone.yml has no pull_request or merge_group trigger. On a per-change basis it is the only workflow that runs `generate-protocol.sh --check --offline`, the offline Go descriptor and fixture guards, and fmt plus clippy -D warnings for the server and client crate graphs. foks-protocol-drift.yml repeats the generated-contract check weekly. Same-repo branch pushes do run these checks against the PR head, but fork PRs and merge-queue runs never execute them. The PR-triggered desktop workflow lints only foks-desktop and foks-desktop-app (2 of 25 members). Every PR-triggered workflow is path-filtered, so none can be a required status check, because a filtered-out run blocks merging while it stays pending. Book ch. 23:80 says `generate-protocol.sh --check` 'fails CI', which is true only after a push or on the weekly schedule.

**Evidence**

- `github/workflows/foks-standalone.yml:3`: Triggers are push (path-filtered), schedule and workflow_dispatch only; no pull_request or merge_group.
- `github/workflows/foks-standalone.yml:115`: The offline generated-metadata check and the Go guard tests run in server-fast (push) and server-full (schedule) only.
- `github/workflows/foks-protocol-drift.yml:29`: The only other place `generate-protocol.sh --check` runs: weekly schedule or manual dispatch.
- `github/workflows/foks-desktop.yml:47`: The PR workflow runs clippy and fmt only for foks-desktop and foks-desktop-app (also lines 48 and 93).
- `github/workflows/foks-desktop.yml:4`: pull_request is path-filtered, as is foks-fuzz.yml:3-4, so neither can be a required check.
- [`book/23-compatibility.qmd:80`](../../../book/23-compatibility.qmd#L80): States that `generate-protocol.sh --check` fails CI when generated files are stale.

**Recommendation**

Add `.github/workflows/ci.yml` triggered on pull_request, merge_group and push to main. (1) A `changes` job computes path groups with a SHA-pinned paths-filter action or a `git diff --name-only` script: rust-core, server, client, desktop-native, desktop-ui, protocol, fuzz, docs. (2) A `protocol` job runs `tools/foks-server/generate-protocol.sh --check --offline` and `go test -C tools/foks-v019-oracle ./...` when crates/foks-proto, foks-rpc, foks-server/src/rpc, foks-protocol-metadata or tools/foks-* change. (3) A `rust-lint` job runs `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings` on ubuntu. First install the desktop apt list and run `bash scripts/prepare-desktop-tests.sh` so the Tauri sidecar exists, or pass `--exclude foks-desktop-app` and leave that crate to foks-desktop.yml. (4) Call tools/foks-client/check.sh and tools/foks-server/check.sh from that workflow, or reuse them with `workflow_call`. (5) A final `ci-ok` job runs with `if: always()` and `needs:` on every job. It fails if any needed job ended as failure or cancelled and treats skipped as success. Make `ci-ok` the only required check. Keep foks-standalone.yml for the scheduled full suite and drop its push trigger once ci.yml exists. Change book ch. 23:80 to name the job that enforces the check.

<details><summary>Verifier note</summary>

The core claim holds. foks-standalone.yml triggers only on push, schedule and workflow_dispatch (lines 3-18). Its server-fast job (push) is the only per-change run of `generate-protocol.sh --check --offline` and the offline Go guards (lines 115-120). The PR-triggered foks-desktop.yml runs clippy and fmt only for foks-desktop and foks-desktop-app (lines 47-48, 93), which is 2 of 25 workspace members. Both PR-triggered workflows (foks-desktop, foks-fuzz) are path-filtered, and no workflow has merge_group. Corrections: (a) the finding says standalone is the only workflow that runs `generate-protocol.sh --check`, but foks-protocol-drift.yml:29 also runs it weekly (online mode). (b) Same-repo branch pushes already run all of these checks on the PR head SHA, so the real gap is that nothing is enforceable as a required check and fork and merge-queue runs get no coverage. That is medium, not high. (c) The recommended clippy command lacks `--` before `-D warnings`. Workspace-wide clippy also needs the staged agent sidecar before foks-desktop-app compiles (AGENTS.md:11-13; tauri externalBin binaries/foks-agent), so the lint job must run scripts/prepare-desktop-tests.sh first, or exclude foks-desktop-app, which foks-desktop.yml already lints.

</details>

### build-contributor-security-onboarding

**No SECURITY.md, CONTRIBUTING.md or prerequisite check; onboarding knowledge lives in an agent-oriented AGENTS.md**

- Type: docs
- Priority: medium
- Effort: S
- Layers: docs, tooling
- Verification: adjusted

The repository has no root SECURITY.md disclosure policy. crates/foks-client/SECURITY.md describes the client security model, but it gives no private reporting channel and no statement of supported versions. There is no CONTRIBUTING.md mapping CI jobs to local commands, and no prerequisite check. Prerequisites are spread across several files. The README's Development section names only Rust, Node and npm, and links to apps/desktop/src-tauri/README.md for the Linux native packages. jq is required by both check.sh scripts and is documented nowhere, and python3 is required by prepare-desktop-tests.sh. Go 1.25 or newer is required by generate-protocol.sh and documented only in tools/foks-protocol-sync/README.md. The requirement to stage a foks-agent binary before tests appears only in AGENTS.md. The session-start hook probes the pkg-config modules and the npm version, but only in Claude containers, and it does not check jq, python3 or Go.

**Evidence**

- [`README.md:38`](../../../README.md#L38): The Development section names only the Rust toolchain, Node and npm. Lines 70-71 delegate platform prerequisites to apps/desktop/src-tauri/README.md.
- [`apps/desktop/src-tauri/README.md:54`](../../../apps/desktop/src-tauri/README.md#L54): Lists the Linux native packages, so they are not documented only in AGENTS.md.
- [`crates/foks-client/SECURITY.md:1`](../../../crates/foks-client/SECURITY.md#L1): A crate-level security model; it has no reporting channel or supported-version policy.
- [`tools/foks-server/check.sh:7`](../../../tools/foks-server/check.sh#L7): Hard requirement on jq (also tools/foks-client/check.sh:24); jq is not listed as a prerequisite anywhere.
- [`scripts/prepare-desktop-tests.sh:14`](../../../scripts/prepare-desktop-tests.sh#L14): Requires python3 for artifact resolution.
- [`tools/foks-protocol-sync/README.md:14`](../../../tools/foks-protocol-sync/README.md#L14): The only place that documents Go 1.25 or newer; CI uses 1.26.x.
- `claude/hooks/session-start.sh:20`: Exits unless CLAUDE_CODE_REMOTE=true. The pkg-config probe at lines 29-37 is not reusable by human contributors and does not check jq, python3 or Go.
- [`AGENTS.md:9`](../../../AGENTS.md#L9): The only place that documents the test-time foks-agent staging requirement and the pkg-config failure mode.

**Recommendation**

(1) Add a root (or .github/) SECURITY.md giving a private reporting route (an address or GitHub private vulnerability reporting) and a pre-v1 statement that only the latest release is supported. Cover the protocol, agent, desktop and server, and link crates/foks-client/SECURITY.md as the threat model. (2) Move the hook's pkg-config probes into scripts/doctor.sh and add checks for the rustup toolchain, .node-version, the packageManager npm version, jq, python3, Go 1.25 or newer, and on macOS the Command Line Tools SDK path that .cargo/config.toml assumes. Print an install command for anything missing, call the script from the hook, and expose it as `npm run doctor`. (3) Add CONTRIBUTING.md with a table from each CI job to its local command, the commit-message convention and agent-staging requirement from AGENTS.md, and the expectations for Go interop. Once the underlying lock and agent-binary flakiness is fixed, remove the flaky-test workarounds from AGENTS.md.

<details><summary>Verifier note</summary>

The core gaps are real. There is no root, .github/ or docs/ SECURITY.md disclosure policy, no CONTRIBUTING.md and no prerequisite-check script. jq is hard-required at tools/foks-server/check.sh:7 and tools/foks-client/check.sh:24, and python3 at scripts/prepare-desktop-tests.sh:14. The session-start hook exits unless CLAUDE_CODE_REMOTE=true (line 20). Several details are overstated. (1) crates/foks-client/SECURITY.md exists (484 lines). It is a security model, not a reporting policy, and the new root policy should link to it. (2) The native libraries are not documented only in AGENTS.md: apps/desktop/src-tauri/README.md:54-56 lists libwebkit2gtk-4.1-dev, libgtk-3-dev, libpcsclite-dev and librsvg2-bin, and README.md:70-71 links that file as the platform prerequisites. AGENTS.md is unique only for the test-time agent-staging requirement and the failure modes. (3) The documented Go minimum is 1.25 (tools/foks-protocol-sync/README.md:14-15, tools/foks-v019-oracle/go.mod); CI uses 1.26.x. (4) The hook probes pkg-config modules and the npm version and stages the agent, but does not probe jq, python3 or Go, so 'most of this' overstates it. Medium priority is reasonable for a key-management project that ships signed binaries and has no private disclosure channel.

</details>

### build-crate-layer-boundaries

**Replace vacuous monorepo path-boundary checks with an enforced crate-layer allowlist; document the desktop's direct client-app use**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: tooling, ci, docs, desktop-native, cli
- Verification: adjusted

The path loops in tools/foks-client/check.sh and tools/foks-server/check.sh only reject path dependencies outside crates/foks-* or apps/desktop/src-tauri, which every one of the 25 members already satisfies. They guard the old monorepo split. .clippy.toml (which cites a Bazel linters.bzl) and the 'AKA-free client graph' step name are leftovers from the same split.

The crate layering the project relies on is not checked. foks-desktop-app and foks-cli both link foks-client-app, foks-keystore and foks-yubi (hardware) directly. During maintenance, both open ClientCredentials and ProfileSession in-process and run verify_imported_profile. The desktop additionally uses client-app for state leases, reset and readiness checks, and input validators. Appendix D calls agent-proto and agent-client the seam for these front ends, and ch. 20 says the host 'holds no long-lived secrets'. Both statements should document and fence the maintenance exception.

**Evidence**

- [`tools/foks-server/check.sh:32`](../../../tools/foks-server/check.sh#L32): The comment at 32-42 describes building 'two products out of one workspace'; the path loop at 58-71 can fail only on a path dependency outside crates/foks-* or apps/desktop/src-tauri.
- `clippy.toml:2`: References lint_clippy_aspect() in tools/lint/linters.bzl; the repository has no tools/lint and no .bzl files.
- `github/workflows/foks-standalone.yml:67`: Step name 'Test the isolated AKA-free client graph'.
- [`apps/desktop/src-tauri/Cargo.toml:23`](../../../apps/desktop/src-tauri/Cargo.toml#L23): Lines 23-31 declare direct dependencies on foks-client-app, foks-keystore, foks-yubi (hardware), foks-crypto and foks-verify.
- [`apps/desktop/src-tauri/src/commands/portability.rs:297`](../../../apps/desktop/src-tauri/src/commands/portability.rs#L297): The host opens ProfileRegistry, ClientCredentials and ProfileSession and runs verify_imported_profile in-process after stop_owned_agent.
- [`apps/desktop/src-tauri/src/agent/maintenance.rs:134`](../../../apps/desktop/src-tauri/src/agent/maintenance.rs#L134): Also uses client-app portability (maintenance_readiness, reset_unreadable_state at :543, ClientStateLease at :383) and matches foks_keystore errors (:138).
- [`crates/foks-cli/src/state.rs:113`](../../../crates/foks-cli/src/state.rs#L113): The CLI opens ClientCredentials and ProfileSession and uses HardwareYubiProvider in-process; foks-cli/Cargo.toml:14-21 has the same direct edges as the desktop.
- [`book/appendix-d-crate-map.qmd:57`](../../../book/appendix-d-crate-map.qmd#L57): Calls agent-proto and agent-client the seam for foks-cli, foks-mcp, foks-desktop and the Tauri app.
- [`book/20-desktop.qmd:45`](../../../book/20-desktop.qmd#L45): 'The host is privileged but holds no long-lived secrets'.

**Recommendation**

(1) Add tools/crate-layers.toml and scripts/check-crate-layers.py, which reads `cargo metadata --locked --format-version 1`. Call it from both check.sh scripts in place of the path loops, and keep the foks-server-testkit production-dependency guard. Seed the allowlist from the current graph. Annotate the client-app, keystore and yubi edges from both foks-desktop-app and foks-cli as maintenance and portability exceptions, naming portability.rs, agent/maintenance.rs, agent/process.rs, first_run.rs, chat_migration.rs and foks-cli/src/state.rs. Add rules such as 'server crates never depend on client crates' and 'foks-mcp and foks-desktop depend only on the agent seam, crypto and proto'. (2) Document the offline-maintenance exception in ch. 20 and Appendix D: during maintenance, with the agent stopped, the host and the CLI open credentials directly. Removing the desktop edge would require more than an agent maintenance operation for import verification. The state leases, readiness and reset functions, and the validation helpers (normalize_profile_label, fix_device_name, validate_local_alias, Capability) would also have to move to a lighter shared crate, so treat that as optional follow-up. (3) Delete .clippy.toml or fix its comment, and rename the CI step.

<details><summary>Verifier note</summary>

Confirmed:
- Both check.sh path loops accept only crates/foks-* and apps/desktop/src-tauri, and all 25 workspace members are inside those paths. The loop still blocks a future non-FOKS path dependency, so it is not entirely vacuous, but it passes trivially.
- .clippy.toml cites tools/lint/linters.bzl, which does not exist (no .bzl files at all).
- foks-standalone.yml:67 is named 'Test the isolated AKA-free client graph'.
- foks-desktop-app's Cargo.toml lines 23-31 list the stated direct dependencies.
- portability.rs:297-347 opens ProfileRegistry, ClientCredentials and ProfileSession and calls verify_imported_profile after stop_owned_agent (line 392).
- Appendix D:57-59 and ch. 20:45-47 say what the finding quotes.
- No document describes the exception.

The finding is incomplete in two ways.

First, foks-cli has the same direct edges (client-app, keystore, yubi with `hardware`) and does the same in-process import verification (crates/foks-cli/src/state.rs:113-143). The Appendix D seam statement is therefore inaccurate for the CLI too, and the proposed rule 'foks-cli does not depend on foks-client' only passes because the CLI reaches foks-client through client-app.

Second, the desktop uses foks-client-app well beyond portability.rs and chat_migration.rs:
- agent/maintenance.rs: maintenance_readiness, reset_unreadable_state, ClientStateLease, keystore error matching.
- agent/process.rs: state leases.
- first_run.rs: state_identity.
- Validation helpers in servers.rs, validation.rs, accounts.rs and account_conveniences.rs.

Moving import verification into the agent alone would not remove the edge.

</details>

### build-docs-guide-crypto-misstatements

**docs/GUIDE.html and docs/INTRO.html still misdescribe the KDF, the hybrid combiner and recovery-kit stretching**

- Type: docs
- Priority: medium
- Effort: S
- Layers: docs
- Verification: adjusted

docs/GUIDE.html and docs/INTRO.html are unchanged since the 2026-09-22 import and still state several things the code contradicts:
- derive_key is described as HKDF-SHA256; the code uses a typed HMAC-SHA-512/256.
- The hybrid combiner is described as 'SHA-512/256 (or HKDF)'; the code uses SHA3-256 (hybrid.rs).
- The KV MAC is described as HMAC-SHA256; the code uses HMAC-SHA-512/256.
- The 17-token recovery kit is described as Argon2id-stretched; the code applies Argon2id only to passphrases.

The research notes (research-crypto.md §17, research-desktop-overview.md §304) and book ch. 3 and 4 already list these errors, and the notes add a few more, but the HTML was never corrected. Book ch. 1 still directs readers to GUIDE.html for the threat model.

**Evidence**

- [`docs/GUIDE.html:546`](../../../docs/GUIDE.html#L546): 'derive_key(seed, app_index, context) using HKDF-SHA256'; also :290 ('HKDF hierarchy') and :781.
- [`docs/GUIDE.html:520`](../../../docs/GUIDE.html#L520): 'passed into SHA-512/256 (or HKDF)'.
- [`docs/GUIDE.html:548`](../../../docs/GUIDE.html#L548): KV MAC described as HMAC-SHA256 (also :296); code uses typed HMAC-SHA-512/256 (foks-crypto/src/primitives.rs:14-20).
- [`docs/INTRO.html:288`](../../../docs/INTRO.html#L288): 'stretched via Argon2id (64 MiB memory, 3 iterations)'; also :267 and GUIDE.html:308.
- [`crates/foks-crypto/src/hybrid.rs:274`](../../../crates/foks-crypto/src/hybrid.rs#L274): Combiner is Sha3_256; derive_key at :1650 is HMAC<Sha512_256>.
- [`book/01-problem.qmd:151`](../../../book/01-problem.qmd#L151): Points readers to docs/GUIDE.html for the threat model; README.md links docs/ only for screenshots.
- [`book/docs/research/research-desktop-overview.md:304`](../../../book/docs/research/research-desktop-overview.md#L304): Additional known GUIDE error: fennec_* MCP tool names and toolset placement.

**Recommendation**

Correct GUIDE.html and INTRO.html in place using the wording from book ch. 3 and 4: the KDF, the combiner, the KV MAC, recovery-kit stretching, the MCP tool names and toolsets, and the app-index description. Alternatively, retire both files in favor of the rendered book and leave a short redirect page. In that case, update book/01-problem.qmd:151 and reword the 'repository's own guide' sentences in book/03-key-hierarchy.qmd:43-46 and book/04-hybrid-encryption.qmd:118-120. Record the decision in book/README.md.

**Already tracked:** Identified in book/docs/research/research-crypto.md §17 and book ch. 3/4, but no change was ever made to docs/*.html.

<details><summary>Verifier note</summary>

The quoted statements are confirmed at the cited lines:
- GUIDE.html:290, 546 and 781 say HKDF.
- GUIDE.html:520 says 'SHA-512/256 (or HKDF)'.
- GUIDE.html:296 and 548 say the KV MAC is HMAC-SHA256.
- INTRO.html:267 and 288, and GUIDE.html:308, say Argon2id is applied to the recovery kit.

The code contradicts each of them:
- `derive_key` (hybrid.rs:1650) is HMAC<Sha512_256> with a type ID.
- `typed_hmac` is HMAC-SHA-512/256.
- The combiner (hybrid.rs:274) is Sha3_256.
- Argon2id appears only in passphrase.rs.

The docs have not changed since aa6a887 (2026-09-22).

The claim that the README points readers to these files is wrong. README.md references docs/ only for the screenshots. The links come from book/01-problem.qmd:151 and the research notes.

The retirement option is incomplete. book/03-key-hierarchy.qmd:43-44 and book/04-hybrid-encryption.qmd:118-120 refer to 'the repository's own guide', and those sentences would need rewording too.

Other GUIDE errors are already catalogued and should be fixed in the same change:
- MCP tool names `fennec_*` and the placement of `usage` in the Team toolset (research-desktop-overview.md:231, 304).
- The 'app index 5 (kv_app)' description (research-crypto.md:269).

</details>

### build-frontend-format-lint-gate

**Prettier is never checked in CI, and ESLint and tsc cover only apps/desktop**

- Type: tooling
- Priority: medium
- Effort: S
- Layers: ci, tooling, desktop-ui
- Verification: adjusted

No workflow runs `prettier --check`, and it currently flags five files. Three are hand-written: chat-mock.ts, chat/send-service.ts and screens/device-sheets.tsx. The other two are generated: wire-contract.json, produced by assemble.mjs, and a recorded benchmark JSON. CI runs only `eslint apps/desktop`, and only on macOS. It never runs `npm run lint`, so scripts/*.mjs (including build-macos-release.mjs) and scripts/benchmarks/*.ts are never linted. The benchmark harness imports production types and decodeChatScope, but its tsconfig is never run, and test_summary.py is never run. All of these pass today, so this is a prevention gap, apart from the Prettier drift.

**Evidence**

- `github/workflows/foks-desktop.yml:42`: `npm exec -- eslint apps/desktop` runs in the macOS job only; the Linux job's 'Check frontend and desktop' step (lines 83-93) runs no ESLint.
- [`package.json:7`](../../../package.json#L7): `lint` (eslint . plus cargo fmt --all -- --check) is not called by any workflow; line 6 is `format` (write-only); there is no check variant.
- [`package.json:20`](../../../package.json#L20): typecheck covers only apps/desktop/tsconfig.json.
- [`scripts/benchmarks/tsconfig.json:1`](../../../scripts/benchmarks/tsconfig.json#L1): Separate harness tsconfig (extends the desktop config, include *.ts); it passes today but nothing runs it.
- `prettierignore:1`: Excludes neither scripts/benchmarks/results/ nor the generated apps/desktop/src-tauri/wire-contract.json.
- [`apps/desktop/src-tauri/wire-contract/assemble.mjs:80`](../../../apps/desktop/src-tauri/wire-contract/assemble.mjs#L80): `--write` emits wire-contract.json with JSON.stringify(…, null, 2), so Prettier formatting of that file would be undone on the next reassembly.

**Recommendation**

Add `"format:check": "prettier --check '**/*.{js,mjs,ts,tsx,json,css}'"` and a `check` script: `npm run typecheck && tsc --noEmit -p scripts/benchmarks/tsconfig.json && eslint . && npm run format:check && python3 scripts/benchmarks/test_summary.py`. Run `npm run check` in the ubuntu UI job and remove the separate eslint line from the macOS job. Add both `scripts/benchmarks/results/` and the generated `apps/desktop/src-tauri/wire-contract.json` to .prettierignore, since the latter is owned by assemble.mjs. Then run Prettier once on the three hand-written files: chat-mock.ts, chat/send-service.ts and screens/device-sheets.tsx.

<details><summary>Verifier note</summary>

Most of the finding holds. `npx prettier --check` flags exactly the five files named. The only ESLint step in any workflow is `npm exec -- eslint apps/desktop` (foks-desktop.yml:42, macOS job). The Linux job (83-93) runs typecheck, UI tests and cargo, but no ESLint. No workflow runs `npm run lint`, scripts/benchmarks/tsconfig.json, or test_summary.py. `tsc -p scripts/benchmarks/tsconfig.json`, `eslint .` and test_summary.py all pass today. ISSUES.md does not track any of this.

One part of the recommendation is wrong. wire-contract.json is a generated artifact: apps/desktop/src-tauri/wire-contract/assemble.mjs `--write` writes it with `JSON.stringify(aggregate, null, 2)`. Its `--check` mode and tests/wire-contract-domains.test.ts compare parsed values, not bytes. If Prettier reformats the file, the next `--write` reverts that formatting and the new format gate fails again. The file belongs in .prettierignore next to the benchmark results, so only three hand-written files need formatting.

Smaller corrections: the `lint` script is at package.json:7 (line 6 is `format`). The benchmark harness imports chat types and `decodeChatScope` from production modules, not the chat services themselves.

</details>

### build-release-standalone-binaries

**No release pipeline for foks-server, foks-agent and foks-rs CLI despite shipped systemd units**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: ci, server, cli, docs
- Verification: adjusted

The only release workflow publishes the desktop DMG and .deb. No workflow builds, checksums or attests the standalone foks-server, foks-agent or foks-rs binaries. packaging/ ships hardened systemd units that expect /usr/local/bin/foks-server and ~/.local/bin/foks-agent, but nothing validates them. crates/foks-server/README.md points operators to packaging/ for units and 'deployment notes', but there are no notes, and the server unit's config path (/etc/foks/server.toml) does not match the documented `init` output (/var/lib/foks/server.toml). Operators must build from source with no provenance, unlike desktop users. There is no CHANGELOG, and desktop releases rely on generate_release_notes.

**Evidence**

- `github/workflows/foks-desktop.yml:18`: Only foks-desktop-v* tags trigger a release; the publish job (245-265) uploads only *.dmg, *.deb and SHA256SUMS.
- [`packaging/foks-server/foks-server.service:10`](../../../packaging/foks-server/foks-server.service#L10): ExecStart=/usr/local/bin/foks-server serve-config --config /etc/foks/server.toml; no release produces this binary.
- [`crates/foks-server/README.md:127`](../../../crates/foks-server/README.md#L127): Says example systemd units and deployment notes are in ../../packaging. packaging/ has no server deployment notes, and the init flow at :111-118 writes /var/lib/foks/server.toml, not /etc/foks/server.toml.
- [`packaging/foks-agent/foks-agent.service:7`](../../../packaging/foks-agent/foks-agent.service#L7): User unit expecting %h/.local/bin/foks-agent, which no release produces.
- `github/workflows/foks-desktop.yml:131`: The tag-to-workspace-version binding exists only for desktop tags.

**Recommendation**

Add .github/workflows/foks-standalone-release.yml on `foks-v*` tags, and bind the tag to the workspace version with the same sed check. Build foks-server, foks-agent and foks-rs with `cargo auditable build --release --locked`: start with x86_64-unknown-linux-gnu, then add aarch64-unknown-linux-gnu (ubuntu-24.04-arm) and aarch64-apple-darwin for agent and CLI. Package each target as a tarball with LICENSE, the systemd units and a sample server.toml. Produce SHA256SUMS, attest with actions/attest-build-provenance, and publish with the pinned gh-release action. Run `systemd-analyze verify` on packaging/**/*.service in CI. Reconcile the server unit's config path (/etc/foks/server.toml) with the documented init flow (/var/lib/foks/server.toml). Add the deployment notes that crates/foks-server/README.md:127 promises, including the libpcsclite1 runtime requirement for agent and CLI. Document installation in README.

<details><summary>Verifier note</summary>

Confirmed:
- The only release workflow is foks-desktop.yml, triggered on `foks-desktop-v*` tags, and it publishes only the DMG, the .deb and SHA256SUMS.
- No workflow builds, checksums or attests foks-server, foks-agent or foks-rs.
- No CHANGELOG exists, and generate_release_notes is used.
- Nothing validates the systemd units.
- The tag-version binding is at foks-desktop.yml:131-134.

One claim is false: the units are referenced. crates/foks-server/README.md:127 says 'Example systemd units and deployment notes are in ../../packaging', although packaging/ contains no server deployment notes.

There is also an inconsistency the finding missed. The server unit runs `serve-config --config /etc/foks/server.toml`, but the documented `foks-server init --directory /var/lib/foks` flow produces /var/lib/foks/server.toml (README:111-118).

The recommendation is feasible with one addition. foks-agent and foks-rs enable foks-yubi `hardware` (the yubikey crate, which uses PC/SC), so the Linux tarballs need libpcsclite1 at runtime and should document it. foks-server does not depend on foks-yubi.

</details>

### build-test-agent-binary-resolution

**CLI integration tests resolve foks-agent by sibling path, which nothing keeps current; six copies of the agent harness**

- Type: testing
- Priority: medium
- Effort: M
- Layers: cli, agent, tooling, docs
- Verification: confirmed

Six foks-cli integration test files each define their own Agent struct and readiness loop. Each one finds the agent at `CARGO_BIN_EXE_foks-rs`.with_file_name("foks-agent"). crates/foks-agent has no tests/ directory, so `cargo test --workspace` never builds or uplifts the plain foks-agent binary, and the CLI tests run whatever the last `cargo build` produced. The readiness loop retries every error, including a protocol version mismatch, for 10 s before reporting. The 'Cargo says fresh but the binary is stale' case in AGENTS.md is consistent with mtime-based freshness after a tree sync that preserves older mtimes. The benchmark guide documents the same symptom with shared targets, and the `-C metadata=` workaround works only because it changes the unit hash. The staging scripts hard-code target/{debug,release} and ignore CARGO_TARGET_DIR, so a stale agent can also be copied into apps/desktop/src-tauri/binaries for local bundles.

**Evidence**

- [`crates/foks-cli/tests/admin_ipc.rs:73`](../../../crates/foks-cli/tests/admin_ipc.rs#L73): Sibling-path lookup; the readiness loop at lines 85-89 retries any error until the deadline.
- [`crates/foks-cli/tests/support/mod.rs:6`](../../../crates/foks-cli/tests/support/mod.rs#L6): The shared support module has only `cli()`. The Agent struct is duplicated in admin_ipc, bot_ipc, invitations_ipc, rename_ipc, sso_ipc and mcp_stdio.
- [`crates/foks-agent/src/main.rs:146`](../../../crates/foks-agent/src/main.rs#L146): The clap command has no `version` attribute and no way to print PROTOCOL_VERSION.
- [`crates/foks-agent-proto/src/message.rs:4`](../../../crates/foks-agent-proto/src/message.rs#L4): PROTOCOL_VERSION = 33; a mismatch only shows up as a connection-time error.
- [`scripts/stage-foks-agent.sh:15`](../../../scripts/stage-foks-agent.sh#L15): Copies target/release/foks-agent by hard-coded path, unlike prepare-desktop-tests.sh:13-32, which reads the artifact path from cargo JSON.
- `claude/hooks/session-start.sh:95`: Also hard-codes target/debug/foks-agent.
- [`scripts/test-foks-desktop-real-agent.sh:7`](../../../scripts/test-foks-desktop-real-agent.sh#L7): Builds without --locked and hard-codes target/debug paths.
- [`scripts/benchmarks/README.md:200`](../../../scripts/benchmarks/README.md#L200): Documents that shared targets can reuse a binary from another source tree.
- [`AGENTS.md:34`](../../../AGENTS.md#L34): The manual relink workaround.

**Recommendation**

(1) Add `support::spawn_agent(state)` to crates/foks-cli/tests/support/mod.rs. It resolves the binary from `FOKS_AGENT_TEST_BINARY`, which the testkit already uses in desktop_command_layer.rs:227. Otherwise it runs, once per process through a OnceLock, `$CARGO build --locked -p foks-agent --bin foks-agent --message-format=json` with the same profile and takes `executable` from the compiler-artifact message. Delete the six Agent copies. (2) Add `#[command(version)]` and a hidden `--print-protocol-version` flag to foks-agent. spawn_agent checks its output against `foks_agent_proto::PROTOCOL_VERSION` before polling and fails immediately with the binary path and mtime on a mismatch. (3) Add `crates/foks-agent/tests/binary.rs` that uses `env!("CARGO_BIN_EXE_foks-agent")` to assert that check, so `cargo test --workspace` always rebuilds the bin. (4) Rewrite stage-foks-agent.sh and the hook's copy step to reuse prepare-desktop-tests.sh's JSON artifact resolution and run the same protocol-version check before copying into binaries/. (5) Then delete AGENTS.md:22-25 and 34-38.

**Already tracked:** AGENTS.md:22-26 and 34-38 describe manual workarounds only.

<details><summary>Verifier note</summary>

I verified the core claim with the Cargo unit graph (`RUSTC_BOOTSTRAP=1 cargo test --workspace --no-run --unit-graph -Z unstable-options`). For foks-agent it contains only the test-mode bin unit and no plain `build` unit. foks-cli does get a plain build unit for foks-rs because it has integration tests. So `cargo test --workspace` never refreshes target/debug/foks-agent. All six files (admin_ipc, bot_ipc, invitations_ipc, mcp_stdio, rename_ipc, sso_ipc) define `struct Agent` and use `CARGO_BIN_EXE_foks-rs`.with_file_name("foks-agent"); bot_ipc.rs:201 does so a second time. support/mod.rs has only `cli()`. The readiness loop at admin_ipc.rs:86-90 retries every error for 10 s. crates/foks-agent has no tests/ directory, its clap command (main.rs:146-150) has no version attribute, and PROTOCOL_VERSION = 33. stage-foks-agent.sh:15 and session-start.sh:95 copy from hard-coded target/ paths. test-foks-desktop-real-agent.sh:7 builds without --locked. FOKS_AGENT_TEST_BINARY is already used at desktop_command_layer.rs:227, and prepare-desktop-tests.sh already resolves the artifact from cargo JSON. The recommendation is feasible: Cargo releases the build-directory lock before running tests, so a nested `cargo build` from a test works (the escargot pattern). The explanation of why 'Cargo reports fresh' is speculative but presented only as consistent with the evidence.

</details>

### build-test-flock-release-race

**Root cause of the first-run receipt-lock flake: flock locks outlive drop while sibling tests spawn processes**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-native, client-lib, agent, ci, tooling
- Verification: adjusted

A flock lock belongs to the open file description. It is released only by LOCK_UN or when every duplicate descriptor is closed. Command::spawn on another thread gives the child a copy of the fd table until execve closes O_CLOEXEC descriptors, so a lock released only by drop/close can outlive the drop. I measured this directly: 1,895 failures in 271,710 drop-then-relock iterations under concurrent spawns, and 0 with an explicit unlock. This explains the documented first-run receipt-lock flake (first_run.rs:475-492) and foks-desktop-app's serial test run. The same race affects production locks that rely on drop: the first_run attempt lock and its status probe, the implicit ProfileLock scheduler drops, and shared reader locks. The result can be a transient false 'Running' or 'mutation-in-flight'. Lock handling is inconsistent today. AgentLock (agent main.rs:6252) and registry.rs:1526 unlock in Drop, ProfileLock unlocks only through an explicit release(), and first_run never unlocks.

**Evidence**

- [`apps/desktop/src-tauri/src/commands/first_run.rs:475`](../../../apps/desktop/src-tauri/src/commands/first_run.rs#L475): Test drops `first` (line 491), then immediately unwraps `second.try_lock_exclusive()` (line 492); it tests fs2 and OS behaviour, not a production guard.
- [`apps/desktop/src-tauri/src/commands/first_run.rs:243`](../../../apps/desktop/src-tauri/src/commands/first_run.rs#L243): The production attempt lock is released only by implicit drop; the status probe at line 204 takes and drops the same lock.
- [`apps/desktop/src-tauri/src/agent/tests/maintenance.rs:662`](../../../apps/desktop/src-tauri/src/agent/tests/maintenance.rs#L662): Lib tests in the same process spawn fixture binaries (also 723, 920, 970 and tests/process.rs:59, 66, 405).
- [`crates/foks-client-app/src/runtime.rs:1125`](../../../crates/foks-client-app/src/runtime.rs#L1125): Scheduler ProfileLock released by implicit drop (also 1138 and federation.rs:823 onward); release() at 115-121 unlocks only exclusive holders when called explicitly; tests at 4296/4401/4432 re-exec current_exe.
- [`crates/foks-client-app/src/registry.rs:1526`](../../../crates/foks-client-app/src/registry.rs#L1526): An existing guard already unlocks in Drop, a pattern to reuse.
- [`crates/foks-agent/src/main.rs:6252`](../../../crates/foks-agent/src/main.rs#L6252): AgentLock Drop calls fs2::FileExt::unlock explicitly.
- [`crates/foks-server/src/writer.rs:398`](../../../crates/foks-server/src/writer.rs#L398): The server already uses std File::try_lock, a precedent for dropping fs2.
- [`scripts/test-rust-full.sh:9`](../../../scripts/test-rust-full.sh#L9): foks-desktop-app suite serialized; tools/foks-client/check.sh:72-73 serializes foks-client-app but cites native credential state.
- [`AGENTS.md:28`](../../../AGENTS.md#L28): The documented workaround is to rerun serially.

**Recommendation**

(1) Add one `LockGuard(File)` type whose Drop calls `unlock()` before close. For shared readers, put it inside the Arc so the last drop unlocks. Use it for every flock holder: first_run attempt locks, ProfileLock (operation, scheduler and shared), DatabaseLock, NativeManifestLock, chat_intent, pending_chat, portability lease and selection, the desktop agent/process.rs lock, and agent connectivity. Fold the registry.rs:1526 guard and AgentLock into it. (2) Replace fs2 with std `File::lock`, `try_lock`, `lock_shared`, `try_lock_shared` and `unlock` (stable since 1.89; toolchain is 1.95; foks-server already uses them). Map `TryLockError::WouldBlock` to the existing WouldBlock arms across all 10 files, then remove `fs2` from workspace dependencies. (3) Add a deterministic regression test. Acquire the guard, then on a second thread spawn `Command::new("true")` with a `pre_exec` closure that sleeps 200 ms. Spawn blocks until exec, so it must run on another thread. Have the child signal through a pipe, or wait about 50 ms, then drop the guard on the test thread and assert that a second descriptor can lock immediately. (4) Run foks-desktop-app and foks-client-app under cargo-nextest in CI. Once the guard lands, drop `--test-threads=1` for foks-desktop-app (test-rust-full.sh:9, foks-desktop.yml:92, AGENTS.md:28-32). Remove it for foks-client-app only after separately confirming that its native-credential tests (check.sh:72) are isolated.

**Already tracked:** AGENTS.md:28-32 documents the workaround (serial run and rerun) but not the cause or a fix.

<details><summary>Verifier note</summary>

I confirmed the mechanism empirically. A standalone program (rustc 1.97, std File::try_lock, which also uses flock) runs four threads spawning `true` in a loop. Under that load, lock / open second / drop(first) / second.try_lock failed 1,895 times in 271,710 iterations. With an explicit unlock() before drop it failed 0 times in 1,111,099 iterations. The cited code holds: first_run.rs:475-492 (test drop then unwrap), 204 and 243 (probe and production lock released only by drop), and the fixture spawns in agent/tests/maintenance.rs:662/723/920/970 and tests/process.rs:59/66/405. Also confirmed: re-exec of current_exe at runtime.rs:4296/4401/4432, AgentLock Drop unlock at main.rs:6252-6255, the desktop agent spawn at process.rs:330, toolchain 1.95.0, and 242 + 226 = 468 tests. Corrections: (a) fs2 is used in 10 files, not 5. The finding misses desktop agent/process.rs, client-app pending_chat.rs, portability/lease.rs, portability/selection.rs and registry.rs. (b) The lock code is mixed rather than uniformly drop-only. registry.rs:1526 already has an unlock-on-Drop guard, and ProfileLock has an explicit release() that unlocks exclusive holders (runtime.rs:115-121). Implicit drops remain at runtime.rs:1125/1138 and the federation.rs scheduler locks, and shared readers deliberately rely on close of the last Arc. (c) foks-server already uses std File::try_lock (writer.rs:398, keys/directory.rs:55/94), so the std migration has a precedent in-tree. (d) The proposed regression test must spawn on another thread, because Command::spawn blocks until the child execs; a pre_exec sleep on the test thread would pass even without the fix. (e) check.sh:72 attributes foks-client-app's --test-threads=1 to native credential state, not this race. Do not drop it solely on the strength of the lock guard. (f) Production impact is a microsecond-wide window that yields a transient false 'Running' or 'mutation-in-flight', so the priority is medium.

</details>

### build-actions-hardening

**Inconsistent action pinning, persisted checkout credentials in secret-bearing jobs, no workflow linting**

- Type: security
- Priority: low
- Effort: S
- Layers: ci
- Verification: adjusted

setup-node uses the mutable @v6 tag in four places (foks-desktop.yml:32, 74, 124, 175). setup-go uses @v6 in foks-standalone.yml:104 and 140, while foks-protocol-drift.yml:18 pins it by SHA. Three SHA pins have no version comment (foks-fuzz.yml:18 and 30, foks-desktop.yml:109). checkout and upload-artifact are each pinned to two different SHAs, labeled v7.0.1 and v4, across workflows. Every checkout except the hosted canary keeps the default persist-credentials, which writes the job's GITHUB_TOKEN into git config. In the macOS release job, npm ci lifecycle scripts and the steps that receive Apple signing and notarization secrets can read that token. In the drift workflow the persisted token carries issues: write. The tokens are otherwise contents: read, which limits the exposure. Nothing lints workflows (no actionlint or zizmor), and there is no .github/dependabot.yml to keep pins current.

**Evidence**

- `github/workflows/foks-desktop.yml:124`: actions/setup-node@v6 tag pin in the macOS signing job; also lines 32, 74 and 175.
- `github/workflows/foks-standalone.yml:104`: actions/setup-go@v6 tag pin (also line 140).
- `github/workflows/foks-protocol-drift.yml:18`: setup-go pinned by SHA (# v6). checkout and upload-artifact here use the '# v4' SHAs (lines 16 and 40), and the workflow grants issues: write (line 10) while the checkout persists credentials.
- `github/workflows/foks-fuzz.yml:18`: checkout SHA pin without a version comment; upload-artifact at line 30 likewise. Nightly is unpinned at line 19.
- `github/workflows/foks-desktop.yml:109`: upload-artifact SHA pin without a version comment.
- `github/workflows/foks-desktop.yml:121`: macOS release checkout without persist-credentials: false. Apple secrets are injected at lines 137-138 and 156-158.
- `github/workflows/foks-hosted-compat.yml:24`: The only checkout that sets persist-credentials: false.

**Recommendation**

Pin every action by full SHA with a `# vX.Y.Z` comment. Use one version each of checkout, upload-artifact and download-artifact. Set `persist-credentials: false` on every checkout, since no job pushes or fetches this repository after checkout. Add a `workflow-lint` job that runs pinned actionlint and zizmor when .github/** changes. Create .github/dependabot.yml (none exists) with the github-actions ecosystem so SHA pins and their version comments stay current.

<details><summary>Verifier note</summary>

The core claims hold. setup-node@v6 appears at foks-desktop.yml:32, 74, 124 and 175. setup-go@v6 appears at foks-standalone.yml:104 and 140, while the drift workflow SHA-pins setup-go at line 18, not 19. Three SHA pins have no version comment: foks-fuzz.yml:18 (checkout), foks-fuzz.yml:30 and foks-desktop.yml:109 (upload-artifact). checkout and upload-artifact are each pinned to two different SHAs, labeled '# v7.0.1' and '# v4'. Only foks-hosted-compat.yml:24 sets persist-credentials: false. No actionlint or zizmor config or job exists. Three details need correcting. (1) Apple secrets go only to the macOS release job, not the Linux one. (2) The workflow-level token is contents: read, so the exposure is limited; the drift workflow's persisted token also carries issues: write. (3) There is no .github/dependabot.yml at all (.github holds only workflows/), so the recommendation has to create one, not extend it. No job pushes or authenticates a git fetch of this repo after checkout; the only remote git call is diff-upstream-protocol.sh:30 against public go-foks. So persist-credentials: false is feasible everywhere. ISSUES.md does not track this.

</details>

### build-benchmark-provenance

**README and book benchmark numbers have no committed result file or revision**

- Type: docs
- Priority: low
- Effort: S
- Layers: docs, tooling
- Verification: adjusted

The README and book ch. 24 publish p95 figures (43.7, 38.4 and 45.1 ms). Nothing records which source revision, binaries or machine produced them, and scripts/benchmarks/results/ holds only the server-expiry run, which does carry hashes. The README column is headed 'After' with no 'Before' column, which suggests it was copied from a comparison. The harness records worker, agent and harness hashes per trial, and summarize-chat-notifications.py exists, so the evidence was produced but not committed.

**Evidence**

- [`README.md:104`](../../../README.md#L104): Table header '| Workload | After |' with no baseline column.
- [`book/24-performance.qmd:130`](../../../book/24-performance.qmd#L130): '### The published results' repeats the numbers at lines 132-136, with no link to a results file.
- [`scripts/benchmarks/results/server-expiry-2026-09-20.json:14`](../../../scripts/benchmarks/results/server-expiry-2026-09-20.json#L14): Precedent: a recorded result with revision, platform, rustc and source hashes. No chat-notification equivalent exists.
- [`scripts/benchmarks/run-chat-notifications.py:79`](../../../scripts/benchmarks/run-chat-notifications.py#L79): The harness records artifactSha256, sourceRevision and sourcePatchSha256 per trial, but not CPU or kernel.
- [`scripts/benchmarks/summarize-chat-notifications.py:45`](../../../scripts/benchmarks/summarize-chat-notifications.py#L45): Summary rows contain p95, deltas and discovery counts only; no provenance fields.

**Recommendation**

Commit the chat-notification results under scripts/benchmarks/results/: either the raw JSONL, which already carries artifactSha256, sourceRevision and sourcePatchSha256 per trial, or a summary file. If only a summary is committed, extend summarize-chat-notifications.py to copy those provenance fields into its output, and extend run-chat-notifications.py to record platform, CPU model, kernel and rustc, as the server-expiry result does. Link the file from README.md and book ch. 24. In the README, rename the 'After' column to 'p95, median of 3 trials'. Optionally add the summarizer's offP95Ms as a notifications-off column. Each future table update then cites a new results file.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' notes that the larger before/after foreground-latency benchmark has not been run; provenance of the published numbers is not tracked.

<details><summary>Verifier note</summary>

The core claim holds. README.md:104 heads the column '| Workload | After |' with no baseline column. The figures (43.7, 38.4, 45.1 ms) came in with the initial import commit 8cb0d1a, with no recorded provenance. scripts/benchmarks/results/ holds only server-expiry-2026-09-20.json, which does record revision, platform, rustc and source hashes. run-chat-notifications.py:79 records artifactSha256, sourceRevision and sourcePatchSha256 per trial. Two corrections. (1) The book heading '### The published results' is at line 130, with the table at 132-136; the book's column is already labeled 'Median of three trial p95s', so only the README header is misleading. (2) summarize-chat-notifications.py prints only per-case off/on p95, deltas and discovery counts; it carries no sourceRevision or hashes, and the harness records no CPU model or kernel. Committing the summary alone would therefore not provide the stated provenance. ISSUES.md:189-190 notes only that the before/after benchmark has not been run, so this finding adds something concrete.

</details>

### build-docs-link-and-drift-check

**Broken design-doc references and stale READMEs; no automated link or path check**

- Type: docs
- Priority: low
- Effort: M
- Layers: docs, ci, tooling
- Verification: adjusted

Four references point to design documents that are not in the repository: docs/state-consistency.md, merkle-checkpoints.md, realtime-inbox-reconciliation-plan.md and default-refresh-job-registration-plan.md. Two of them appear in `#[ignore]` reasons. The src-tauri README has several stale sections:
- Its command-domain table lists 11 of the 22 modules.
- Its Checks section omits the required `--test-threads=1`.
- Its WebView section still says the test touches no credential store or account and lists only smoke checks; 45b4b46 added a fixture-seeded authenticated flow, and the section does not mention building the fixture.

The root README still lists 'browser acceptance tests', which a6d4cd8 removed, and the stub script and hook still mention Chromium. book/README.md links to `../foks-rs`. The research notes contain /home/bnoland absolute paths. research-crypto.md has 58 lines of `lib.rs:N` references, which went stale when e19fe86 split foks-crypto/src/lib.rs down to 96 lines. No automated link or path check exists.

**Evidence**

- [`crates/foks-server-testkit/tests/realtime_reconciliation_benchmark.rs:45`](../../../crates/foks-server-testkit/tests/realtime_reconciliation_benchmark.rs#L45): Ignore reason cites the missing docs/realtime-inbox-reconciliation-plan.md.
- [`crates/foks-client-app/src/runtime/registration_benchmark.rs:8`](../../../crates/foks-client-app/src/runtime/registration_benchmark.rs#L8): Ignore reason cites the missing docs/default-refresh-job-registration-plan.md.
- [`apps/desktop/README.md:1127`](../../../apps/desktop/README.md#L1127): Links to ../../docs/state-consistency.md, which does not exist; research-desktop-overview.md:309 already notes this.
- [`crates/foks-client-db/README.md:29`](../../../crates/foks-client-db/README.md#L29): Links to ../../docs/merkle-checkpoints.md, which does not exist.
- [`apps/desktop/src-tauri/README.md:85`](../../../apps/desktop/src-tauri/README.md#L85): The table (lines 85-97) lists 11 of the 22 modules in commands/mod.rs; it omits account_conveniences, bot, chat, chat_local, chat_migration, first_run, invitations, portability, preparation, sso and web_admin.
- [`apps/desktop/src-tauri/README.md:62`](../../../apps/desktop/src-tauri/README.md#L62): `cargo test --locked -p foks-desktop-app` lacks `-- --test-threads=1` (AGENTS.md:28-29 and CI foks-desktop.yml:92 use it).
- [`apps/desktop/src-tauri/README.md:108`](../../../apps/desktop/src-tauri/README.md#L108): 'It does not access a credential store or remote account' is stale after 45b4b46: the fixture seeds private-file credentials and a local server and drives authenticated editing. The local command at :112 relies on the default --fixture path but does not say to build the example.
- [`README.md:120`](../../../README.md#L120): Mentions browser acceptance tests that a6d4cd8 removed.
- [`scripts/container-native-stubs.sh:18`](../../../scripts/container-native-stubs.sh#L18): Still refers to the removed acceptance harness's Chromium.
- [`book/README.md:4`](../../../book/README.md#L4): Relative link ../foks-rs does not resolve; book/ is inside the repository.
- [`book/docs/research/research-client.md:3`](../../../book/docs/research/research-client.md#L3): Says paths are relative to /home/bnoland/projects/foks-rs; all six research notes contain such paths.

**Recommendation**

(1) Restore the four design docs under book/ or docs/design/, or remove the references and reword the two ignore reasons. (2) Add scripts/check-docs.py and run it in CI on *.md, *.qmd and Rust changes. It should resolve relative Markdown and Quarto links and `docs/...md` strings in Rust sources and ignore reasons, and check that every `mod` in apps/desktop/src-tauri/src/commands/mod.rs appears in the src-tauri README table. (3) In the src-tauri README, add --test-threads=1 to the Checks section. Rewrite the WebView section to describe the fixture-seeded authenticated flow, the lost-reply fault and the private-file credentials, and add the `cargo build -p foks-client-app --example webview_fixture` step. Fix the root README layout line and the Chromium comments in the stub script and hook. (4) Mark book/docs/research/* as snapshots pinned to a commit and remove the absolute paths. (5) Optionally run `quarto render` on changes under book/**.

<details><summary>Verifier note</summary>

Confirmed:
- All four design docs are missing.
- Both `#[ignore]` reasons cite missing files (realtime_reconciliation_benchmark.rs:45, registration_benchmark.rs:8).
- apps/desktop/README.md:1127 and foks-client-db/README.md:29 link to missing files.
- commands/mod.rs declares 22 modules and the README table lists 11; the missing names are exactly those given.
- README Checks line 62 lacks --test-threads=1 (AGENTS.md:28-29).
- Root README:120 still says 'browser acceptance tests', which a6d4cd8 removed.
- container-native-stubs.sh:18-19 and session-start.sh:100 still mention Chromium.
- book/README.md:4 links to ../foks-rs.
- All six research notes contain /home/bnoland paths.
- No link checker exists.

The WebView section claim needs correcting. 'Does not yet automate account enrollment' is still accurate, because webview_fixture seeds a synthetic account and does not drive enrollment. The stale text is at lines 105-108: it says the test 'does not access a credential store or remote account', but it now seeds private-file credentials and a local testkit server. It also lists only startup, agent-status, app-lock and disconnect checks. `--fixture` has a default (target/debug/examples/webview_fixture), so the real gap is the missing `cargo build --example webview_fixture` step.

research-crypto.md has 58 lines with `lib.rs:N` references, 87 occurrences in total. research-desktop-overview.md:309 already notes the missing state-consistency.md.

</details>

### build-fuzz-corpus-persistence

**Weekly fuzz campaigns discard their corpus and run on a floating nightly**

- Type: testing
- Priority: low
- Effort: S
- Layers: ci, tooling
- Verification: adjusted

The weekly schedule fuzzes five targets for 600 s each but keeps nothing except crash artifacts on failure. Coverage found in one run is lost, and each campaign restarts from the small committed seeds (for example, one `empty-frame` seed for rpc). The nightly toolchain floats, so a weekly failure may come from a compiler change rather than a code change. cargo-fuzz is compiled from source in each matrix leg. fuzz/Cargo.lock is maintained separately; I verified it matches the root lockfile today, but nothing enforces that.

**Evidence**

- `github/workflows/foks-fuzz.yml:26`: FUZZ_SECONDS=600 on schedule (weekly cron at line 6). No corpus cache or upload.
- `github/workflows/foks-fuzz.yml:19`: `rustup toolchain install nightly` with no date pin; later steps use `cargo +nightly` from the repository root, where rust-toolchain.toml selects 1.95.0.
- `github/workflows/foks-fuzz.yml:20`: cargo-fuzz 0.13.2 is compiled from source in each of the five matrix legs, with no cache.
- `github/workflows/foks-fuzz.yml:31`: Artifacts are uploaded only on failure().
- [`fuzz/Cargo.toml:10`](../../../fuzz/Cargo.toml#L10): Separate [workspace]. fuzz/Cargo.lock is maintained independently, and nothing checks it against the root lockfile.
- [`fuzz/corpus/rpc/empty-frame`](../../../fuzz/corpus/rpc/empty-frame): The only committed rpc seed; snowpack also has one seed.

**Recommendation**

Add actions/cache on `fuzz/corpus/${{ matrix.target }}` with key `fuzz-corpus-<target>-${{ github.run_id }}` and restore-keys `fuzz-corpus-<target>-`. On scheduled runs, run `cargo fuzz cmin <target>` before the save. PR runs will then also start from the accumulated corpus. Pin nightly by its dated name in the workflow: for example, set `FUZZ_TOOLCHAIN: nightly-2026-09-15` and use `rustup toolchain install "$FUZZ_TOOLCHAIN"` and `cargo +"$FUZZ_TOOLCHAIN" fuzz ...`. Mirror the pin in fuzz/README.md and bump it by hand. A fuzz/rust-toolchain.toml would not apply, because the commands run from the repository root with an explicit +toolchain. Cache the cargo-fuzz binary keyed on its version and the toolchain, or install a pinned prebuilt release. Add a check to the fuzz PR job that every package shared by fuzz/Cargo.lock and Cargo.lock has the same version.

<details><summary>Verifier note</summary>

The core claims hold. The schedule is weekly (cron '17 7 * * 2') with FUZZ_SECONDS=600 (line 26) over five targets. There is no corpus cache or upload; artifacts are uploaded only on failure() (line 31). fuzz/corpus is not gitignored, so corpus entries found in CI are discarded with the runner. The committed seeds are small: rpc has the single 'empty-frame' file and snowpack has one. Nightly floats (line 19). cargo-fuzz is built from source in every leg (line 20) with no cargo cache. fuzz/Cargo.toml:10 declares a separate [workspace]. I confirmed all 140 shared packages have matching versions in the two lockfiles today, and no script checks this. The recommendation needs a correction: a fuzz/rust-toolchain.toml would have no effect. The workflow runs `cargo +nightly ...` from the repository root, where rust-toolchain.toml selects 1.95.0, and an explicit +toolchain overrides any toolchain file anyway. Dependabot is not a dependable way to bump a dated nightly, and the repository has no Dependabot config.

</details>

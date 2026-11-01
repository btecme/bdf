# BUILD-008 — independent correction retest

Status: **original R1–R4 corrections pass; one additional P1, B8-R5, remains.**

Retested 2026-11-01 on `633cfd2c4567dc603307ab2f30ff35c73445fc38` (PR #14),
whose base `c1c38b07ec72cf321793082e8ed576a2c3858cee` contains merged #11/#12.
The BUILD-008 service/grant/policy source is unchanged by the adapter PR. This is
not an isolated rebuild of either pre-merge PR head. The [original failed review](BUILD-008-codex-review.md)
and its raw evidence are preserved.

## Original corrections

- **R1 closed:** the original public-API leaf-link read and directory-link write
  probes now refuse both operations. No outside output appears. This is the stated
  boundary inside the granted root; the trusted root/ancestors and hard-link limits
  are not broadened or claimed solved.
- **R2 closed:** after adapting the probe to `DocumentScope::Only(Vec<DocumentKey>)`,
  the first session's limited grant cannot query or rotate the other session's
  document with the same local number. Its pages remain `[0, 0, 0]`.
- **R3 closed:** with an unavailable audit sink, the ordinary dispatcher, direct
  document view and direct save planning all refuse the client. The original probe
  passes; the new queued-plan and human-save regressions pass in the scoped suite.
- **R4's successful-writer race closed:** a real revoking child stays running while
  an issuer is paused inside its transaction. After release, both commands succeed,
  the new grant remains and the revoked secret is refused by a fresh store. The
  interprocess lock covers the whole cooperating-writer transaction.

## B8-R5 — P1 — a failed revoke is undone by the next grant transaction

Location: `crates/app-service/src/grants.rs:409–418` (`transact`), with the
failed-write behavior in `revoke` at `:455`.

The store explicitly promises that a revoke which cannot be persisted still holds
in memory for that run. The new transaction wrapper unconditionally reloads the
file before every mutation. That file still says the grant is live after a failed
revoke, and reloading it drops the store's in-memory denial.

Independent public-API reproduction on Windows:

1. Create a private grant store and authenticate a synthetic client.
2. Make the grant file read-only so its atomic replacement fails.
3. Revoke the grant: the call returns a storage error, and `check(actor)` correctly
   refuses it in memory.
4. Issue an unrelated grant while the file is still read-only. This call also fails.
5. `check(actor)` now **succeeds** for the previously refused actor.

The safety assertion exits **101**. No JSON file is edited or secret forged; the
read-only bit is restored after observing the result. This is distinct from the
fixed successful-concurrency case: two failed writes must not reactivate an actor
that this same running store already denied.

Preserve local revocations across reloads/failed transactions, or keep the store
fail-closed until a verified durable recovery. Do not let an unrelated issue/prune
restore authorization from an older disk snapshot. Add failure/retry regressions
that inspect the already-authenticated actor, not just a newly opened store, and
retain the successful interprocess-race test.

## Evidence and scope

The [PR #14 review](PR-014-codex-review.md) records the 386 passing scoped tests,
exact-head hosted results and shared raw-evidence manifest. Five independent original
counterexample cases are now green (R1 has separate read/write cases); B8-R5's new
failure case is red. Source identities and Cargo-selected libraries are recorded
under ignored `target/bdf-evidence/PR-014-codex-review/`.

The BDF state namespace and model-free core package are present; native ARM64
packaging/install checks pass on the tested head. No legacy credentials are imported.
Adapter migration remains BUILD-009 and overall BUILD-008 acceptance stays open.
No production fix, commit, push or merge was made by Codex.

## Revision history

| Date | Change |
|---|---|
| 2026-11-01 | Original R1–R4 corrected cases pass; failed-revocation reload regression B8-R5 requires correction. |

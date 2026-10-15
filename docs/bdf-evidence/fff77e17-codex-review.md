# Codex review — rustls 0.23.46

Reviewed 2026-10-14. Commit: `fff77e1746a365fb807862d07bf29d8cab859fc3`; parent: `f5b9480a9d5447655101ce453dcdab58c1dbca82`. Branch: `chore/build-001-baseline`. Scope is this commit, not the earlier asynchronous-open changes or the whole BUILD-001 acceptance gate.

## Verdict

**No blocking findings in the dependency bump.** The only package change in the parsed lockfile is rustls 0.23.45 → 0.23.46 with its checksum. Package count and every other package record are unchanged, including the tempfile/getrandom edge. No application source, Cargo manifest, enabled feature or advisory-ignore policy changes in this commit.

The upstream [0.23.46 release](https://github.com/rustls/rustls/releases/tag/v%2F0.23.46) lists the four security fixes. Each published advisory names 0.23.46 as a patched release: [session Debug secrets](https://github.com/rustls/rustls/security/advisories/GHSA-c8m5-c952-6cm4), [ECH rejection](https://github.com/rustls/rustls/security/advisories/GHSA-vp88-798v-h67f), [Brotli allocation](https://github.com/rustls/rustls/security/advisories/GHSA-h2v8-394m-p7vv), [key-exchange hints](https://github.com/rustls/rustls/security/advisories/GHSA-w44r-93j7-qv28). The cached RustSec records map these to RUSTSEC-2026-0346 through 0349 and also specify the patched bound. RustSec web pages and the crates.io API were unavailable through the browser tool; the upstream advisory/release pages were read successfully.

## Verification

- Verified all **2,859** entries in the builder's `target/bdf-evidence/BUILD-001-rustls/MANIFEST.sha256`: no missing files or SHA-256 mismatches.
- Independently hashed the cached `.crate`: `48e13bd8c0e9365c43cfa5c9e8f9ad49d3c8444926c9aac819e0e4dc503c8fdf`, matching the lockfile. Archive metadata names upstream source commit `f04804fa13341a9b1dc02b5d1dd8a746f2c4b7fb`, also named by the public release. Package metadata declares Apache-2.0 OR ISC OR MIT and Rust 1.71. That dependency's MSRV does not prove the workspace's 1.95 MSRV.
- Verified current `Cargo.lock` SHA-256: `d2ef027890a784d6792b54413a20bd280675780eebb53e2a6495d25bc3e4b75c`; unchanged after the independent checks.
- Parsed both recorded scoped-suite logs: each totals **1,220 passed, 0 failed, 6 ignored** across 69 summaries. These are verified builder logs, not new full-suite runs by Codex.
- Independently ran `cargo tree --locked --offline -p pdfcraft -i rustls -e features`: resolves 0.23.46 through ureq and rustls-cng; enabled rustls features are ring, logging/log, std and tls12.
- Independently ran the full `cargo deny` checks offline. The first attempt could not take the lock in the read-only user advisory cache. Repeated with a workspace copy of that same database and a temporary config whose only difference is `advisories.db-path`: **advisories, bans, licenses and sources all pass**, exit 0. No fetch, ignored-advisory additions or lockfile modifications.
- Attempted the live-update test with the builder's post-bump test binary. It failed before contacting GitHub because the sandbox account has no visible trusted OS certificates. This does not independently reproduce the builder's successful HTTPS run, and it does not show a regression caused by this lockfile change. The production code returns an error; the ignored test unwraps it.
- `git diff --check fff77e17^ fff77e17` passed.

Independent logs, commands and hashes: ignored `target/bdf-evidence/fff77e17-codex-review/results.json`, `tree.log`, `deny-workspace-db.log` and `tls.log`. No fresh build, full test sweep, hosted CI run, optional-font build or workspace MSRV check was performed in this review.

## Nonblocking evidence clarification

Section 14.4 of BUILD-001 evidence says the post-quantum downgrade could apply to the update check in principle; section 15 refers back to that applicability assessment. The actual graph enables ring, whose built-in groups are X25519, P-256 and P-384. Ureq selects that provider unless explicitly overridden; first-party code installs no alternative rustls provider. There is no hybrid X25519MLKEM768 group here.

Therefore the [advisory's specific hybrid-to-classical loss](https://github.com/rustls/rustls/security/advisories/GHSA-w44r-93j7-qv28) is not applicable to the current update-check provider configuration. Clarify that historical evidence sentence in an addendum; keep the upgrade for the published fixes and dependency hygiene. This documentation correction does not block the bump and should not broaden this review into an application rewrite.

## Remaining baseline gates

This result does not mark BUILD-001 verified or approve merging/releasing the branch. Its earlier source changes require their own review; hosted Windows/Linux CI, optional fonts and MSRV remain open as reported. Keep `baseline_green = false` until the applicable baseline gate is satisfied. No push, merge, publication or MCP registration was performed.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-14 | 0.1 | Reviewed the narrow rustls bump, verified evidence, reran offline dependency checks and recorded TLS sandbox limits. |

# Claude Code — BDF delivery focus and current handoff

Status: proposed owner handoff, prepared 2026-10-18. Copy the prompt below into Claude Code
with access to the same local BDF folder. It does not authorize weakening AGENTS.md or
merging/releasing work without the owner's separate instruction.

## Prompt

Deliver a dependable, fast, intuitive Windows 11 PDF desktop app, local-first, for viewing,
annotation, forms, page organization and trustworthy saving. Windows desktop comes first;
web/cloud and a reusable support platform come later. Claude implements; Codex independently
reviews. Prioritize working, reviewable increments over more planning or framework expansion.

Act as the implementation lead. Own routine choices of APIs, compliant dependencies, tests,
error handling, branch synchronization, PR retargeting and CI repairs. Use supported authorized
tools; a missing browser control is not a product decision. Preserve other contributors' work
and never bypass access/approval restrictions. If access truly needs the owner, state the one
action required. Do not ask the owner to choose Rust crates, filesystem primitives or test
commands.

Keep the agreed safety outcomes: no lost edits or original bytes, no temporary disclosure of
confidential document bytes, and no misleading save success. Follow AGENTS.md, including
safe Rust, asset provenance, confinement and opt-in MCP/control. Never weaken a gate, broaden
permissions, use an unsafe/copy fallback or accept data exposure to declare progress. Follow
explicit stop conditions, but investigate compliant alternatives and bring a recommendation;
reporting a limitation does not satisfy the acceptance criterion.

Ask the owner only about a material change to product behavior, safety/privacy guarantees,
scope, cost, external commitments, releases or an explicit owner-only exception. Explain the
user impact, your recommended choice and the meaningful alternative. Ask one product-level
question and continue unblocked work in the same item. Installer coexistence versus replacing
the upstream app is a product decision; selecting a file API is engineering work.

Current assignment: read `docs/bdf-evidence/BUILD-004-codex-review.md` and finish its bounded
corrections. Fix R1's staging disclosure and R2's save-completion state loss, including the
focused regression/UI tests. Do not introduce background jobs or a new service framework to
fix this boundary. Verify the existing `67693735` fixture/mode corrections, and bring Codex a
scoped disposition for inherited dependency-policy failures without blanket advisory ignores.
Handle #4's ordinary base retargeting after #3's merge; keep its independent-review dependency
honest. Do not self-certify BUILD-003. Preserve localization fallback and record native-platform
coverage without expanding this into translating the entire product or adding macOS release
scope. Do not claim unperformed power-loss or macOS tests.

Use scoped, bounded verification with two build jobs; hosted CI owns workspace sweeps. Prove
the targeted defects, fix their causes and retest affected paths. Avoid unrelated refactors,
blanket mutation campaigns and duplicate evidence. Do not create watchers or reminders unless
asked. Distinguish implemented, locally tested, hosted-green and independently accepted.

Report briefly: user-visible outcome, changes, verification/limits, any genuine blocker with
your recommendation, and the next bounded action. Link technical evidence instead of narrating
every discovery. Push the focused, tested PR for Codex re-review when ready. Do not merge or
release without separate owner authorization.

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | Product-focused operating agreement and bounded BUILD-004 correction prompt. |

# BDF-0002: Run open and render of untrusted documents in a supervised worker process

> **Status:** Proposed for owner approval · **Date:** 2026-10-15 · **Work item:** BUILD-002 · **Evidence:** [BUILD-002](../bdf-evidence/BUILD-002.md) section 6 · **Prototype:** `apps/pdfcraft-worker`

## Context

The foundation review (F10) found no total memory budget and no isolation between a hostile
document and the application's accepted state. Panics are caught, but an out-of-memory abort, a stack
overflow, an infinite loop or a runaway allocation in the parser or rasterizer takes the whole
application, and the user's open documents, with it. The specification (section 7) asks for a
private, bounded worker, and for a report, not unsafe code, if no compliant design exists.

## Decision

Move **open, render and (later) text extraction** into a private worker process, with these rules.
The prototype demonstrates each one with a test that runs the real worker binary.

1. **Protocol.** Finite, versioned frames over the child's stdin and stdout. Every size is checked on
   the 20-byte header before any payload is allocated. Documents and pixels travel as numbered
   chunks with a declared total and SHA-256, checked at both ends. stdout carries frames only.
2. **The parent owns state.** The accepted document bytes, hash and page count live in the parent. A
   worker is a disposable view; when one dies the next call starts another and re-sends the
   document. A read-only render is retried once; a memory-limit hit, a refusal and a cancel are not.
3. **Containment, failing closed.** The worker runs in a job object that ends it when the parent
   ends, however the parent ends, including when the worker is wedged. If that job cannot be created
   or the worker cannot be assigned to it, the call **fails with an error and the worker is killed**;
   running uncontained is an explicit opt-in (`allow_uncontained`) that the caller can see
   (`Worker::contained()`). A watcher thread enforces a per-call deadline and polls a memory cap.
   Replies are validated (request id, sizes, order, hash, and the checked product of width, height
   and bytes per pixel against the declared length) and a malformed one ends the worker. `Limits`
   and `Config` are validated before use, so a nonsensical limit is an error, not a wrap-around.
4. **Cancellation.** The caller is answered at once, including while the parent is blocked uploading
   the document, hashing it or waiting for a reply; the worker gets a grace period (300 ms in the
   prototype) to stop and is then terminated. Measured: about 315 ms from cancel to return.
5. **Mutations stay serialized in the parent** (spec section 7); the worker is read-only.
6. **Safe Rust only.** The only third-party code is `win32job`, a maintained safe wrapper, reviewed
   before use (BUILD-002 section 2).

## What the evaluation found

- **Feasible with no `unsafe` code.** All the containment properties above are tested; six of them
  were also mutation-checked (the guarantee removed, the test seen to fail), and the corrections
  after review (BUILD-002 follow-up, R1 to R3) each have a test seen to fail before the fix: the
  size arithmetic, cancellation during upload and hashing, and fail-closed containment.
- **The worker stays a prototype.** The corrections make its guarantees true as stated; they do not
  make it a product component. Adoption is conditional on those corrected guarantees being re-reviewed
  and on the open items below.
- **Cost.** About 10 ms more for a small document; 17 to 41 ms more for a large render; about
  110 ms more to open a 95 MiB scan, because the document crosses the pipe and is hashed. Memory:
  the worker holds its own copy of the input, so a document costs about twice its size across the
  process tree while it is open.
- **`win32job` gaps.** It exposes kill-on-close, CPU affinity and soft working-set limits. It does
  **not** expose a hard commit limit or an active-process limit. The memory cap is therefore
  enforced by polling the OS counters, which bounds a *growing* worker but not a single large
  allocation: a 200 MiB cap was crossed to a 268 MiB working set and a 602 MiB commit before the
  worker was stopped. A hard limit needs the job object's per-process memory limit, which the
  wrapper does not offer.
- **A process is not a sandbox.** The worker keeps the user's file and network access. Removing it
  needs an AppContainer or restricted token, which the safe-wrapper set evaluated here does not
  provide. The protocol is ready for it (the worker never needs a path: bytes arrive over the pipe),
  so a sandboxed worker could be denied the filesystem and network entirely.

## Residual risk: what still parses untrusted input in the application process

Until each is moved or separately contained, a crash in any of these still takes the application
down. Each needs its own containment evidence before release (spec section 7):

- opening in the UI (`open_worker` is a thread, with a last-resort `catch_unwind`, not a process);
- every edit, save, organize, redact, optimize and convert operation in the engine (`lopdf` and the
  `cos` model);
- text extraction, search and annotation or form reading;
- signature verification and certificate parsing, OCR, and document JavaScript;
- the web build, which has no processes.

## Consequences

- BUILD-010 integrates the worker: the supervisor and protocol move to `pdfcraft-platform` (layer 7)
  and the process body stays in `apps/pdfcraft-worker` (layer 8).
- A hard memory limit and a sandbox are open items, tracked as risks, not claimed. Polling bounds a
  growing worker, not one large allocation, and the per-operation admission limits of BDF-0003 do not
  replace a whole-process-tree budget.
- Fail-closed containment means a machine on which the job object cannot be created gets an error
  from the worker path, not a silently unconfined worker; the integration must decide what the
  application does then (refuse, or an explicit, visible opt-in).
- If a compliant way to set a hard commit limit or an AppContainer cannot be found, record the
  architecture blocker and keep the in-process containment (deadline, cap by polling) as the
  documented limit.

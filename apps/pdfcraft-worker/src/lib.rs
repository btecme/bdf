//! Prototype of the private document-processing worker (BUILD-002, work item 4).
//!
//! The question this crate answers: can open and render be moved into a separately spawned,
//! supervised process, over a bounded protocol, with a crash, hang or runaway allocation in the
//! worker unable to erase the parent's accepted state, using only safe Rust? The decision it
//! supports is recorded in `docs/adr/`. It is not wired into the application.
//!
//! * [`protocol`]: finite, versioned frames; every limit checked before allocation.
//! * [`worker`]: the process body.
//! * [`supervisor`]: the parent's spawn, deadline, memory cap, validation, cancel and restart.
//!
//! In a release these would be split: the protocol and supervisor into `pdfcraft-platform`
//! (layer 7), the process body staying in `apps/pdfcraft-worker` (layer 8).

#![deny(clippy::print_stdout, clippy::dbg_macro)]

pub mod protocol;
pub mod supervisor;
pub mod worker;

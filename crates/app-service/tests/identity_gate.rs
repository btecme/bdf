//! Nothing but the applications' own start-up may speak as the person at the keyboard (BUILD-008).
//!
//! `ActorContext::local_user()` is the one way to get a human actor, and it takes no argument: it reads the operating-system
//! account. That makes it impossible to *claim* to be someone else, but it also means that any code that can call it
//! acts as the person. So the question is who can call it. This test reads the workspace's production sources and fails
//! if any crate other than this one calls it: an adapter (the tools, the MCP server, the UI's control channel) gets an actor
//! from a client's grant, never from here. The applications (`apps/`) are where the person's session starts, and tests may
//! stand in for the person.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

#[test]
fn only_this_crate_and_the_applications_can_speak_as_the_person() {
    let root = workspace();
    let crates = root.join("crates");
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(&crates) else { panic!("the workspace's crates folder is missing") };
    for entry in entries.flatten() {
        if entry.file_name() == "app-service" {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&entry.path().join("src"), &mut files);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            if text.contains("local_user(") {
                found.push(file.strip_prefix(&root).unwrap_or(&file).display().to_string());
            }
        }
    }
    assert!(found.is_empty(), "these production sources can speak as the person, which only the applications may: {found:?}");
}

#[test]
fn the_scan_can_see_the_service_crates_own_caller() {
    // The same scan over this crate does find the call, so the test above is not blind (a wrong path would find nothing).
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let hits = files.iter().filter(|f| std::fs::read_to_string(f).unwrap_or_default().contains("local_user(")).count();
    assert!(hits >= 1, "the scan found nothing in the crate that defines the call");
    let mut sources = Vec::new();
    rust_files(&workspace().join("crates"), &mut sources);
    assert!(sources.len() > 100, "the scan found only {} sources: the path is wrong", sources.len());
}

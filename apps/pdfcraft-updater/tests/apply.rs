//! The updater's command line, run in-process with a test trust root over real packages.

use std::path::Path;

use pdfcraft_update::apply::{LastResult, peek_result};
use pdfcraft_update::layout;
use pdfcraft_update::package::PACKAGE_FILE;
use pdfcraft_update::testing::{installed_copy, release, trust};
use pdfcraft_update::{Channel, Version};

const V0: &[(&str, &[u8])] = &[("bdf.exe", b"exe-0"), ("bdf-cli.exe", b"cli-0"), ("portable.txt", b"p")];
const V1: &[(&str, &[u8])] = &[("bdf.exe", b"exe-1"), ("bdf-cli.exe", b"cli-1"), ("portable.txt", b"p"), ("new.txt", b"n")];

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn args(app: &Path, release_file: &Path, zip: &Path, extra: &[&str]) -> Vec<String> {
    let mut a = s(&[
        "apply",
        "--app-dir",
        &app.display().to_string(),
        "--release",
        &release_file.display().to_string(),
        "--zip",
        &zip.display().to_string(),
        "--wait-secs",
        "1",
    ]);
    a.extend(s(extra));
    a
}

fn result(app: &Path) -> LastResult {
    peek_result(app).expect("the updater leaves a result")
}

#[test]
fn apply_updates_and_reports_it() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let (pkg, manifest) = release(root.path(), "0.8.1", V1);
    let release_file = root.path().join("release.json");
    std::fs::write(&release_file, manifest.envelope()).unwrap();
    // The downloads folder is the updater's to clear after a success.
    std::fs::create_dir_all(layout::downloads_dir(&app)).unwrap();
    std::fs::write(layout::downloads_dir(&app).join("leftover.zip"), b"x").unwrap();
    let code = pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &trust());
    assert_eq!(code, pdfcraft_updater::OK);
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-1");
    assert_eq!(layout::detect(&app).unwrap().version, Version(0, 8, 1));
    let r = result(&app);
    assert_eq!((r.outcome.as_str(), r.from.as_str(), r.to.as_str()), ("updated", "0.8.0", "0.8.1"));
    assert!(!layout::downloads_dir(&app).exists());
    assert!(std::fs::read_to_string(layout::update_dir(&app).join("updater.log")).unwrap().contains("apply finished with code 0"));
    // BDFData is untouched.
    assert_eq!(std::fs::read(app.join("BDFData").join("recovery").join("doc1.autosave")).unwrap(), b"unsaved work");
}

#[test]
fn a_build_without_a_trust_key_installs_nothing_and_says_why() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let (pkg, manifest) = release(root.path(), "0.8.1", V1);
    let release_file = root.path().join("release.json");
    std::fs::write(&release_file, manifest.envelope()).unwrap();
    let code = pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &pdfcraft_update::Trust::none(Channel::Stable));
    assert_eq!(code, pdfcraft_updater::FAILED);
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-0");
    let r = result(&app);
    assert_eq!(r.outcome, "failed");
    assert!(r.message.contains("no release signing key"), "{}", r.message);
}

#[test]
fn a_tampered_package_or_release_leaves_the_copy_alone_and_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let (pkg, manifest) = release(root.path(), "0.8.1", V1);
    let release_file = root.path().join("release.json");
    std::fs::write(&release_file, manifest.envelope()).unwrap();
    let mut bytes = std::fs::read(&pkg.zip).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&pkg.zip, bytes).unwrap();
    assert_eq!(pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &trust()), pdfcraft_updater::FAILED);
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-0");
    assert!(result(&app).message.contains("failed verification"), "{}", result(&app).message);
    // A release file with a changed payload.
    let text = std::fs::read_to_string(&release_file).unwrap();
    std::fs::write(&release_file, text.replacen("signature\": \"", "signature\": \"0", 1)).unwrap();
    assert_eq!(pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &trust()), pdfcraft_updater::FAILED);
}

#[cfg(windows)]
#[test]
fn a_program_still_running_postpones_the_update_without_changing_anything() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let (pkg, manifest) = release(root.path(), "0.8.1", V1);
    let release_file = root.path().join("release.json");
    std::fs::write(&release_file, manifest.envelope()).unwrap();
    let running = std::fs::File::options().read(true).share_mode(1).open(app.join("bdf-cli.exe")).unwrap();
    let code = pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &trust());
    assert_eq!(code, pdfcraft_updater::POSTPONED);
    assert_eq!(result(&app).outcome, "postponed");
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-0");
    drop(running);
}

#[test]
fn rollback_and_recover_work_from_the_command_line() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let (pkg, manifest) = release(root.path(), "0.8.1", V1);
    let release_file = root.path().join("release.json");
    std::fs::write(&release_file, manifest.envelope()).unwrap();
    assert_eq!(pdfcraft_updater::run_with(&args(&app, &release_file, &pkg.zip, &[]), &trust()), pdfcraft_updater::OK);
    let rb = s(&["rollback", "--app-dir", &app.display().to_string(), "--wait-secs", "1"]);
    assert_eq!(pdfcraft_updater::run_with(&rb, &trust()), pdfcraft_updater::OK);
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-0");
    assert!(app.join(PACKAGE_FILE).is_file());
    let rc = s(&["recover", "--app-dir", &app.display().to_string()]);
    assert_eq!(pdfcraft_updater::run_with(&rc, &trust()), pdfcraft_updater::OK);
}

#[test]
fn bad_command_lines_are_usage_errors_not_crashes() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let dir = app.display().to_string();
    for bad in [
        s(&[]),
        s(&["frobnicate"]),
        s(&["apply"]),
        s(&["apply", "--app-dir", "relative"]),
        s(&["apply", "--app-dir", &dir]),
        s(&["apply", "--app-dir", &dir, "--zip"]),
        s(&["recover", "--app-dir", &dir, "--wait-secs", "soon"]),
        s(&["recover", "--app-dir", &dir, "--bogus"]),
        s(&["recover", "--app-dir", "C:\\definitely\\not\\here"]),
    ] {
        assert_eq!(pdfcraft_updater::run_with(&bad, &trust()), pdfcraft_updater::USAGE, "{bad:?}");
    }
    assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"exe-0");
}

#[test]
fn it_only_restarts_the_launcher_of_the_folder_it_updated() {
    let root = tempfile::tempdir().unwrap();
    let app = installed_copy(root.path(), "0.8.0", V0);
    let other = root.path().join("evil.exe");
    std::fs::write(&other, b"not a program").unwrap();
    // A restart request for some other program is ignored (and logged), never spawned.
    let rc = s(&["recover", "--app-dir", &app.display().to_string(), "--restart", &other.display().to_string()]);
    assert_eq!(pdfcraft_updater::run_with(&rc, &trust()), pdfcraft_updater::OK);
    let log = std::fs::read_to_string(layout::update_dir(&app).join("updater.log")).unwrap();
    assert!(log.contains("not restarting"), "{log}");
}

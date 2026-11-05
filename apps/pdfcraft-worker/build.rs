//! Windows only: embed version info (VERSIONINFO) into `bdf-worker.exe`, so Explorer's Details tab
//! and inventory tools show BDF and its version for the private worker process too, as they do for
//! `bdf.exe` and `bdf-cli.exe`.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile still links, unless `PDFCRAFT_REQUIRE_WINRES=1` turns it into an error (release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PDFCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "BDF")
        .set("FileDescription", "BDF document worker")
        .set(
            "LegalCopyright",
            "Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors; BDF changes by the BDF contributors. MIT OR Apache-2.0.",
        )
        .set("OriginalFilename", "bdf-worker.exe")
        .set("InternalName", "bdf-worker");
    if let Err(e) = res.compile() {
        if std::env::var_os("PDFCRAFT_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources failed: {e}");
            return;
        }
        println!("cargo:warning=bdf-worker.exe built without version resources: {e}");
    }
}

//! Release tooling for the portable package and its signed update information (BUILD-018).
//!
//! * `update-keygen --out FILE`: make a signing key pair. The private key goes to FILE (refused if it exists)
//!   and **never** into the repository; the public key (hex) is printed, and is what a build is given as
//!   `BDF_UPDATE_PUBKEY`.
//! * `portable-package --bin-dir DIR --version X.Y.Z --out DIR`: the portable ZIP, with `bdf-package.json`.
//! * `update-sign --zip FILE --version X.Y.Z --key FILE --out DIR [...]`: the signed `bdf-update.json` and
//!   `SHA256SUMS.txt` for a package.
//! * `update-verify --release FILE --zip FILE --pubkey HEX`: check a release the way the app does.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use p256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use pdfcraft_update::builder::{Input, write_package};
use pdfcraft_update::manifest::{Manifest, PackageEntry, make_envelope};
use pdfcraft_update::{Channel, SignedManifest, Trust, Version, hex, package};
use sha2::{Digest, Sha256};

use crate::gates::root;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn required(args: &[String], name: &str) -> Result<String> {
    flag(args, name).with_context(|| format!("{name} is required"))
}

/// `update-keygen --out FILE`
pub fn keygen(args: &[String]) -> Result<()> {
    let out = PathBuf::from(required(args, "--out")?);
    if out.exists() {
        bail!("{} already exists; refusing to overwrite a signing key", out.display());
    }
    if out.starts_with(root()) {
        bail!("{} is inside the repository; a signing key must never be committed. Choose a folder outside it", out.display());
    }
    let key = loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random numbers available: {e}"))?;
        if let Ok(k) = SigningKey::from_slice(&bytes) {
            break k;
        }
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, format!("{}\n", hex(&key.to_bytes())))?;
    let public = hex(key.verifying_key().to_sec1_point(false).as_bytes());
    println!("private key written to {} (keep it secret; back it up; never commit it)", out.display());
    println!("public key (BDF_UPDATE_PUBKEY): {public}");
    println!("key id: {}", Trust::from_hex(&public, Channel::Stable).ok().and_then(|t| t.key_id().map(str::to_string)).unwrap_or_default());
    Ok(())
}

fn load_key(path: &Path) -> Result<SigningKey> {
    let text = std::fs::read_to_string(path).with_context(|| format!("couldn't read the signing key {}", path.display()))?;
    let bytes = pdfcraft_update::unhex(text.trim()).context("the signing key file is not hex")?;
    SigningKey::from_slice(&bytes).map_err(|_| anyhow::anyhow!("the signing key file is not a P-256 private key"))
}

/// `portable-package --bin-dir DIR --version X.Y.Z --out DIR [--arch x64]`
pub fn portable_package(args: &[String]) -> Result<()> {
    let bin = PathBuf::from(required(args, "--bin-dir")?);
    let version = required(args, "--version")?;
    let out = PathBuf::from(required(args, "--out")?);
    let arch = flag(args, "--arch").unwrap_or_else(|| "x64".into());
    let root = root();
    let mut inputs = Vec::new();
    for exe in ["bdf.exe", "bdf-cli.exe", "bdf-worker.exe", "bdf-updater.exe"] {
        let p = bin.join(exe);
        if !p.is_file() {
            bail!("{} is missing; build the release binaries first", p.display());
        }
        inputs.push(Input { rel: exe.into(), source: p });
    }
    let mut add = |rel: &str, source: PathBuf| -> Result<()> {
        if !source.is_file() {
            bail!("{} is missing", source.display());
        }
        inputs.push(Input { rel: rel.into(), source });
        Ok(())
    };
    for f in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE"] {
        add(f, root.join(f))?;
    }
    add("LICENSE-document.txt", root.join("assets/app-icon/LICENSE-document.txt"))?;
    add("portable.txt", root.join("packaging/windows/portable.txt"))?;
    add("README.txt", root.join("packaging/windows/PORTABLE-README.txt"))?;
    // The licences of the fonts a craft-fonts build embeds, as the other packages carry them.
    if let Some(fonts) = std::env::var_os("CRAFT_FONTS_DIR") {
        let mut families: Vec<_> = std::fs::read_dir(Path::new(&fonts).join("fonts")).into_iter().flatten().flatten().collect();
        families.sort_by_key(|e| e.file_name());
        for family in families {
            let ofl = family.path().join("OFL.txt");
            if ofl.is_file() {
                add(&format!("OFL-{}.txt", family.file_name().to_string_lossy()), ofl)?;
            }
        }
    }
    let zip = out.join(format!("bdf-{version}-windows-{arch}-portable.zip"));
    let entry = write_package(&zip, &version, "windows", &arch, &inputs).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{}  {} bytes  sha256 {}", zip.display(), entry.size, entry.sha256);
    Ok(())
}

/// `update-sign --zip FILE --version X.Y.Z --key FILE --out DIR [--channel stable|dev] [--notes FILE] [--url-base URL] [--arch x64] [--expect-pubkey HEX]`
pub fn sign(args: &[String]) -> Result<()> {
    let zip = PathBuf::from(required(args, "--zip")?);
    let version = required(args, "--version")?;
    let key_path = PathBuf::from(required(args, "--key")?);
    let out = PathBuf::from(required(args, "--out")?);
    let channel = flag(args, "--channel").unwrap_or_else(|| "stable".into());
    let arch = flag(args, "--arch").unwrap_or_else(|| "x64".into());
    let parsed = Version::parse(&version).context("the version must be MAJOR.MINOR.PATCH")?;
    let key = load_key(&key_path)?;
    let public = hex(key.verifying_key().to_sec1_point(false).as_bytes());
    if let Some(expected) = flag(args, "--expect-pubkey")
        && !expected.trim().eq_ignore_ascii_case(&public)
    {
        bail!("the signing key is not the expected one (its public key is {public})");
    }
    let name = zip.file_name().and_then(|n| n.to_str()).context("the package has no file name")?.to_string();
    let url_base = flag(args, "--url-base").unwrap_or_else(|| format!("https://github.com/btecme/bdf/releases/download/v{parsed}"));
    let notes = match flag(args, "--notes") {
        Some(p) => std::fs::read_to_string(&p).with_context(|| format!("couldn't read the notes {p}"))?,
        None => String::new(),
    };
    let (size, sha256) = package::hash_file(&zip)?;
    let manifest = Manifest {
        schema: 1,
        product: "bdf".into(),
        channel: channel.clone(),
        version: parsed.to_string(),
        published: now_rfc3339(),
        notes,
        notes_url: Some(format!("https://github.com/btecme/bdf/releases/tag/v{parsed}")),
        min_from: None,
        packages: vec![PackageEntry {
            os: "windows".into(),
            arch,
            kind: "portable-zip".into(),
            url: format!("{url_base}/{name}"),
            size,
            sha256: sha256.clone(),
        }],
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    let signature: p256::ecdsa::Signature = key.sign_prehash(&Sha256::digest(&json)).map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;
    let envelope = make_envelope(&json, &hex(&signature.to_bytes()));
    std::fs::create_dir_all(&out)?;
    let release_path = out.join("bdf-update.json");
    std::fs::write(&release_path, &envelope)?;
    let copy = out.join(&name);
    if copy.canonicalize().ok() != zip.canonicalize().ok() {
        std::fs::copy(&zip, &copy)?;
    }
    let sums = format!("{}  {name}\n{}  bdf-update.json\n", sha256, hex(&Sha256::digest(&envelope)));
    std::fs::write(out.join("SHA256SUMS.txt"), sums)?;
    // Check what was just made exactly as the app will.
    let trust =
        Trust::from_hex(&public, Channel::parse(&channel).context("the channel must be stable or dev")?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let verified = SignedManifest::from_envelope(&envelope, &trust).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("signed {} {} with key {}: {}", name, parsed, trust.key_id().unwrap_or(""), release_path.display());
    println!("package sha256 {}  ({} bytes); release version {}", verified.manifest.packages[0].sha256, size, verified.version);
    Ok(())
}

/// `update-verify --release FILE --zip FILE --pubkey HEX [--channel stable|dev]`
pub fn verify(args: &[String]) -> Result<()> {
    let release = PathBuf::from(required(args, "--release")?);
    let zip = PathBuf::from(required(args, "--zip")?);
    let channel = Channel::parse(&flag(args, "--channel").unwrap_or_else(|| "stable".into())).context("the channel must be stable or dev")?;
    let trust = Trust::from_hex(&required(args, "--pubkey")?, channel).map_err(|e| anyhow::anyhow!("{e}"))?;
    let signed = SignedManifest::from_envelope(&std::fs::read(&release)?, &trust).map_err(|e| anyhow::anyhow!("{e}"))?;
    let entry = signed
        .manifest
        .packages
        .iter()
        .find(|p| p.os == "windows" && p.kind == "portable-zip")
        .context("the release has no Windows portable package")?;
    let verified = package::verify_zip(&zip, entry, signed.version, "windows", &entry.arch).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("OK: version {} package verified ({} files), key {}", signed.version, verified.info.files.len(), trust.key_id().unwrap_or(""));
    Ok(())
}

fn now_rfc3339() -> String {
    let secs = pdfcraft_update::now_unix();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_is_a_real_rfc3339_timestamp() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.starts_with("20") && s.ends_with('Z'), "{s}");
    }

    #[test]
    fn keys_and_releases_round_trip_through_the_tools() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key.txt");
        // Not inside the repository, so keygen accepts it.
        keygen(&["--out".into(), key.display().to_string()]).unwrap();
        assert!(keygen(&["--out".into(), key.display().to_string()]).is_err(), "never overwrites a key");
        let signing = load_key(&key).unwrap();
        let public = hex(signing.verifying_key().to_sec1_point(false).as_bytes());
        let src = dir.path().join("a.exe");
        std::fs::write(&src, b"program").unwrap();
        let zip = dir.path().join("bdf-0.8.1-windows-x64-portable.zip");
        write_package(&zip, "0.8.1", "windows", "x64", &[Input { rel: "bdf.exe".into(), source: src }]).unwrap();
        let out = dir.path().join("out");
        let base = [
            "--zip",
            &zip.display().to_string(),
            "--version",
            "0.8.1",
            "--key",
            &key.display().to_string(),
            "--out",
            &out.display().to_string(),
            "--channel",
            "dev",
        ];
        let args: Vec<String> = base.iter().map(|s| s.to_string()).collect();
        sign(&args).unwrap();
        let release = out.join("bdf-update.json");
        let v = |extra: &[&str]| {
            let mut a: Vec<String> = [
                "--release",
                &release.display().to_string(),
                "--zip",
                &out.join("bdf-0.8.1-windows-x64-portable.zip").display().to_string(),
                "--channel",
                "dev",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            a.extend(extra.iter().map(|s| s.to_string()));
            verify(&a)
        };
        v(&["--pubkey", &public]).unwrap();
        assert!(v(&["--pubkey", &"04".repeat(65)]).is_err());
        // Signing with a key other than the expected one is refused.
        let mut wrong = args.clone();
        wrong.extend(["--expect-pubkey".into(), "04".repeat(65)]);
        assert!(sign(&wrong).is_err());
        assert!(out.join("SHA256SUMS.txt").is_file());
    }
}

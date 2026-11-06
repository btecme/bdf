//! The HTTPS transport: fetch the release envelope and download the package.
//!
//! Nothing fetched here is trusted by being fetched. The envelope is verified against the build's signing
//! key by the caller; the package is verified against the size and SHA-256 inside that signed envelope. So
//! the transport only has to be careful: HTTPS only (a downgrade by redirect is refused), the operating
//! system's trusted certificates, hard limits on size and time, and a partial file that is never left
//! looking like a finished one.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::manifest::{MAX_ENVELOPE_BYTES, PackageEntry, SignedManifest, package_url_allowed};
use crate::{Channel, Result, Trust, UpdateError, io_err};

/// Where BDF releases publish their signed update information: the newest release's `bdf-update.json`.
pub const STABLE_FEED: &str = "https://github.com/btecme/bdf/releases/latest/download/bdf-update.json";

/// The feed this build reads, or `None` when it has none (a development build that was not given one).
///
/// A release build always reads [`STABLE_FEED`] and ignores the environment. Only a `dev` build reads
/// `BDF_UPDATE_FEED`, and only an HTTPS address or a loopback test server.
pub fn feed_url(trust: &Trust) -> Option<String> {
    match trust.channel() {
        Channel::Stable => Some(STABLE_FEED.to_string()),
        Channel::Dev => std::env::var("BDF_UPDATE_FEED").ok().filter(|u| dev_feed_allowed(u)),
    }
}

fn dev_feed_allowed(url: &str) -> bool {
    url.starts_with("https://") || ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| url.starts_with(p))
}

fn agent(https_only: bool, global: Option<Duration>) -> Result<ureq::Agent> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() && https_only {
        return Err(UpdateError::Network("no trusted certificates found on this system".into()));
    }
    let mut config = ureq::Agent::config_builder()
        .https_only(https_only)
        .timeout_global(global)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .http_status_as_error(false);
    if !certs.is_empty() {
        config = config.tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::new_with_certs(&certs)).build());
    }
    Ok(config.build().new_agent())
}

fn user_agent() -> String {
    format!("BDF/{} (update check)", env!("CARGO_PKG_VERSION"))
}

fn is_loopback(url: &str) -> bool {
    url.starts_with("http://")
}

/// Fetch and verify the release envelope at `url`.
pub fn fetch_release(trust: &Trust, url: &str) -> Result<SignedManifest> {
    if !trust.is_configured() {
        return Err(UpdateError::NoTrustRoot);
    }
    let agent = agent(!is_loopback(url), Some(Duration::from_secs(30)))?;
    let mut response = agent
        .get(url)
        .header("User-Agent", user_agent())
        .header("Accept", "application/json")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if status == 404 {
        return Err(UpdateError::Network("no BDF release has been published yet".into()));
    }
    if !(200..300).contains(&status) {
        return Err(UpdateError::Network(format!("http status {status}")));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit((MAX_ENVELOPE_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(|e| UpdateError::Network(format!("unreadable answer ({e})")))?;
    SignedManifest::from_envelope(&body, trust)
}

/// Download `entry` to `dest`, reporting `(received, total)` to `progress`, which returns `false` to cancel.
///
/// The bytes arrive in `dest.part` and are renamed only when exactly the signed size has arrived; a failure or
/// a cancel removes the partial file. The hash is checked afterwards by [`crate::package::verify_zip`].
pub fn download(entry: &PackageEntry, channel: Channel, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<()> {
    if !package_url_allowed(&entry.url, channel) {
        return Err(UpdateError::Malformed("the package address is not a BDF release address".into()));
    }
    let part = dest.with_extension("part");
    let _ = std::fs::remove_file(&part);
    let result = download_to(entry, &part, progress);
    match result {
        Ok(()) => std::fs::rename(&part, dest).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            io_err("couldn't finish the download")(e)
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn download_to(entry: &PackageEntry, part: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<()> {
    let agent = agent(!is_loopback(&entry.url), None)?;
    let response = agent.get(&entry.url).header("User-Agent", user_agent()).call().map_err(|e| UpdateError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(UpdateError::Network(format!("http status {status}")));
    }
    if let Some(len) = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok())
        && len != entry.size
    {
        return Err(UpdateError::Tampered(format!("the server offers {len} bytes, the signed release says {}", entry.size)));
    }
    let mut body = response.into_body();
    let mut reader = body.as_reader();
    let mut out = std::fs::File::create(part).map_err(io_err("couldn't start the download"))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut received = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| UpdateError::Network(format!("the download was interrupted ({e})")))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > entry.size {
            return Err(UpdateError::Tampered("the server sent more than the signed size".into()));
        }
        out.write_all(buf.get(..n).unwrap_or(&[])).map_err(io_err("couldn't save the download"))?;
        if !progress(received, entry.size) {
            return Err(UpdateError::Cancelled);
        }
    }
    out.sync_all().map_err(io_err("couldn't save the download"))?;
    if received != entry.size {
        return Err(UpdateError::Network(format!("the download ended early ({received} of {} bytes)", entry.size)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;
    use crate::manifest::make_envelope;
    use crate::manifest::testing::manifest_json;
    use crate::trust::testkeys;

    /// A one-shot-per-connection HTTP server on loopback serving `routes` (path to status, body).
    pub(crate) fn serve(routes: Vec<(String, u16, Vec<u8>)>) -> (String, std::thread::JoinHandle<()>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        s.set_nonblocking(false).unwrap();
                        let mut buf = [0u8; 4096];
                        let n = s.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]).to_string();
                        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                        let (status, body) =
                            routes.iter().find(|(p, _, _)| *p == path).map(|(_, st, b)| (*st, b.clone())).unwrap_or((404, b"nope".to_vec()));
                        let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(&body);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        (base, handle, stop)
    }

    #[test]
    fn a_release_build_ignores_the_feed_override_and_a_dev_build_takes_only_safe_ones() {
        assert_eq!(feed_url(&Trust::none(Channel::Stable)).as_deref(), Some(STABLE_FEED));
        for ok in ["https://example.test/bdf-update.json", "http://127.0.0.1:9/x", "http://localhost:9/x"] {
            assert!(dev_feed_allowed(ok), "{ok}");
        }
        for bad in ["http://example.test/x", "file:///C:/x", "ftp://127.0.0.1:1/x", "http://127.0.0.1/x", "javascript:1"] {
            assert!(!dev_feed_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn a_signed_release_is_fetched_and_a_forged_or_missing_one_is_not() {
        let json = manifest_json("0.8.1", "dev", &"a".repeat(64), 10);
        let good = make_envelope(json.as_bytes(), &testkeys::sign(1, json.as_bytes()));
        let forged = make_envelope(json.as_bytes(), &testkeys::sign(2, json.as_bytes()));
        let (base, handle, stop) = serve(vec![
            ("/good.json".into(), 200, good),
            ("/forged.json".into(), 200, forged),
            ("/junk.json".into(), 200, b"<html>".to_vec()),
            ("/err.json".into(), 500, vec![]),
        ]);
        let trust = testkeys::trust(1, Channel::Dev);
        assert_eq!(fetch_release(&trust, &format!("{base}/good.json")).unwrap().version.to_string(), "0.8.1");
        assert!(matches!(fetch_release(&trust, &format!("{base}/forged.json")), Err(UpdateError::Tampered(_))));
        assert!(matches!(fetch_release(&trust, &format!("{base}/junk.json")), Err(UpdateError::Malformed(_))));
        assert!(matches!(fetch_release(&trust, &format!("{base}/err.json")), Err(UpdateError::Network(_))));
        assert!(matches!(fetch_release(&trust, &format!("{base}/missing.json")), Err(UpdateError::Network(_))));
        assert!(matches!(fetch_release(&Trust::none(Channel::Dev), &format!("{base}/good.json")), Err(UpdateError::NoTrustRoot)));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn a_download_arrives_whole_or_not_at_all() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let (base, handle, stop) = serve(vec![
            ("/ok.zip".into(), 200, body.clone()),
            ("/short.zip".into(), 200, body[..1000].to_vec()),
            ("/long.zip".into(), 200, [body.clone(), vec![0; 10]].concat()),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let entry = |name: &str| PackageEntry {
            os: "windows".into(),
            arch: "x64".into(),
            kind: "portable-zip".into(),
            url: format!("{base}/{name}"),
            size: body.len() as u64,
            sha256: "0".repeat(64),
        };
        let mut seen = 0u64;
        let dest = dir.path().join("a.zip");
        download(&entry("ok.zip"), Channel::Dev, &dest, &mut |got, total| {
            assert!(got <= total);
            seen = got;
            true
        })
        .unwrap();
        assert_eq!((std::fs::read(&dest).unwrap(), seen), (body.clone(), body.len() as u64));
        assert!(!dest.with_extension("part").exists());
        for bad in ["short.zip", "long.zip", "missing.zip"] {
            let dest = dir.path().join(bad);
            assert!(download(&entry(bad), Channel::Dev, &dest, &mut |_, _| true).is_err(), "{bad}");
            assert!(!dest.exists() && !dest.with_extension("part").exists(), "{bad}: no partial file is left");
        }
        // Cancelling leaves nothing either.
        let dest = dir.path().join("cancel.zip");
        assert!(matches!(download(&entry("ok.zip"), Channel::Dev, &dest, &mut |_, _| false), Err(UpdateError::Cancelled)));
        assert!(!dest.exists() && !dest.with_extension("part").exists());
        // A release build will not take a loopback address, signed or not.
        assert!(matches!(download(&entry("ok.zip"), Channel::Stable, &dir.path().join("x.zip"), &mut |_, _| true), Err(UpdateError::Malformed(_))));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        handle.join().unwrap();
    }
}

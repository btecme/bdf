//! Reads the update trust configuration at build time.
//!
//! * `BDF_UPDATE_PUBKEY`: the release signing public key, an uncompressed SEC1 P-256 point in
//!   hex (130 characters). Production builds set it; without it the build carries **no** trust
//!   root and every update operation is refused (fail closed).
//! * `BDF_UPDATE_CHANNEL`: `stable` (the default) or `dev`. A `dev` build trusts a test key,
//!   labels itself in the interface and may read its feed from `BDF_UPDATE_FEED`.

fn main() {
    println!("cargo:rerun-if-env-changed=BDF_UPDATE_PUBKEY");
    println!("cargo:rerun-if-env-changed=BDF_UPDATE_CHANNEL");
    let key = std::env::var("BDF_UPDATE_PUBKEY").unwrap_or_default();
    let channel = std::env::var("BDF_UPDATE_CHANNEL").unwrap_or_else(|_| "stable".into());
    if channel != "stable" && channel != "dev" {
        eprintln!("BDF_UPDATE_CHANNEL must be `stable` or `dev`, not `{channel}`");
        std::process::exit(1);
    }
    println!("cargo:rustc-env=BDF_TRUST_KEY={}", key.trim());
    println!("cargo:rustc-env=BDF_TRUST_CHANNEL={channel}");
}

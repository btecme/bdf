//! `cargo xtask network`: the product reaches the network only through the one place that is reviewed
//! for it (BUILD-003 deliverable 5, "network off by default").
//!
//! A workspace crate that adds an HTTP or WebSocket client or server library is a new way for the
//! product to talk to the network, so it fails this check until the allowlist below names it, which
//! needs a review of what it asks for, when, and whether the user started it. Today the only entry is
//! the desktop app, whose single request is Help > Check for updates (it asks only when the user does).
//!
//! This is a dependency-surface check, not a proof that nothing opens a socket: `std::net` needs no
//! dependency, and the loopback-only UI control channel and the single-instance socket use it. Those
//! are listed, with what keeps them off, in `docs/bdf-network-and-activation.md`.

use crate::layers::Crate;

/// Libraries that make or accept network requests above the socket level. Matched by crate name.
const NETWORK_LIBRARIES: &[&str] = &[
    "attohttpc",
    "axum",
    "curl",
    "h2",
    "http-body",
    "hyper",
    "isahc",
    "minreq",
    "reqwest",
    "rouille",
    "surf",
    "tiny_http",
    "tonic",
    "tungstenite",
    "tokio-tungstenite",
    "ureq",
    "warp",
    "websocket",
];

/// The workspace crates allowed to depend on a network library, and which library. Every addition
/// is a reviewed decision.
const ALLOWED: &[(&str, &str)] = &[("pdfcraft", "ureq")];

/// One message per workspace crate that depends on a network library it is not allowed.
pub fn check(crates: &[Crate]) -> Vec<String> {
    let mut out = Vec::new();
    for krate in crates {
        for dep in &krate.deps {
            if NETWORK_LIBRARIES.contains(&dep.name.as_str()) && !ALLOWED.contains(&(krate.name.as_str(), dep.name.as_str())) {
                out.push(format!(
                    "`{}` depends on `{}`, a network library; the product's network use is reviewed in one place (xtask/src/network.rs ALLOWED, docs/bdf-network-and-activation.md)",
                    krate.name, dep.name
                ));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The allowed pairs that no crate uses any more, so the allowlist cannot quietly outlive the code.
pub fn stale_allowances(crates: &[Crate]) -> Vec<String> {
    ALLOWED
        .iter()
        .filter(|(krate, lib)| !crates.iter().any(|c| c.name == *krate && c.deps.iter().any(|d| d.name == *lib)))
        .map(|(krate, lib)| format!("the allowlist permits `{krate}` to depend on `{lib}`, but it does not; remove the entry"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{Dep, DepKind};

    fn krate(name: &str, deps: &[(&str, DepKind)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(d, kind)| Dep { name: (*d).into(), kind: *kind, workspace: false }).collect() }
    }

    #[test]
    fn the_desktop_app_may_use_its_http_client_and_nothing_else_may() {
        let ok = [krate("pdfcraft", &[("ureq", DepKind::Normal)]), krate("pdfcraft-engine", &[("serde", DepKind::Normal)])];
        assert!(check(&ok).is_empty() && stale_allowances(&ok).is_empty());
        for (who, lib, kind) in [
            ("pdfcraft-engine", "ureq", DepKind::Normal),
            ("pdfcraft-cli", "reqwest", DepKind::Normal),
            ("pdfcraft-automation", "hyper", DepKind::Build),
            ("pdfcraft-ui-egui", "tiny_http", DepKind::Dev),
            ("pdfcraft", "reqwest", DepKind::Normal),
        ] {
            let found = check(&[krate(who, &[(lib, kind)])]);
            assert_eq!(found.len(), 1, "{who} -> {lib}: {found:?}");
            assert!(found[0].contains(who) && found[0].contains(lib), "{found:?}");
        }
    }

    #[test]
    fn an_allowance_nobody_uses_is_reported() {
        let none = [krate("pdfcraft", &[("serde", DepKind::Normal)])];
        assert_eq!(stale_allowances(&none).len(), 1);
        assert_eq!(stale_allowances(&[]).len(), 1);
    }
}

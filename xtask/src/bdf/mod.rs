//! BUILD-002 tooling: the provenance-controlled fixture corpus and its acceptance matrix.
//!
//! `cargo xtask bdf-corpus` generates every fixture from source into `target/bdf-corpus/` (git
//! ignored; fixtures are never committed) and writes a manifest of what each one is, where it came
//! from, its SHA-256, what a reader must find in it and how closely a render must match the
//! oracle's. `--write-manifest` also refreshes the committed copy under `docs/bdf-evidence/`.

mod annot_cases;
mod form_cases;
mod graphics_cases;
mod hostile_cases;
mod imgcmp;
mod model;
mod oracle;
mod pdfgen;
mod proc;
mod structure_cases;
mod text_cases;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::assets::sha256_hex;
use crate::gates::root;
use model::{Case, Expect, Outcome, Tolerance};

pub use oracle::run as oracle;

/// Inputs shared by the case generators.
pub struct Ctx {
    pub root: PathBuf,
    pub inter_regular: Vec<u8>,
    pub inter_semibold: Vec<u8>,
    pub jetbrains: Vec<u8>,
    pub dancing: Vec<u8>,
    /// The external oracle, when available; derived (encrypted) cases need it.
    pub mutool: Option<PathBuf>,
}

/// The font files the corpus embeds. All are OFL-1.1 assets already in the repository and listed in
/// `ATTRIBUTION.toml`; nothing from the system font directory is ever used.
const FONTS: &[(&str, &str)] = &[
    ("Inter-Regular.ttf", "inter_regular"),
    ("Inter-SemiBold.ttf", "inter_semibold"),
    ("JetBrainsMono-Regular.ttf", "jetbrains"),
    ("DancingScript.ttf", "dancing"),
];

impl Ctx {
    fn load(root: &Path, mutool: Option<PathBuf>) -> Result<(Self, Vec<(String, String)>)> {
        let mut hashes = Vec::new();
        let mut get = |name: &str| -> Result<Vec<u8>> {
            let path = root.join("assets").join("fonts").join(name);
            let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            hashes.push((name.to_string(), sha256_hex(&bytes)));
            Ok(bytes)
        };
        let ctx = Self {
            root: root.to_path_buf(),
            inter_regular: get(FONTS[0].0)?,
            inter_semibold: get(FONTS[1].0)?,
            jetbrains: get(FONTS[2].0)?,
            dancing: get(FONTS[3].0)?,
            mutool,
        };
        Ok((ctx, hashes))
    }
}

#[derive(Serialize)]
struct ManifestCase<'a> {
    id: &'a str,
    family: &'a str,
    class: &'a str,
    description: &'a str,
    bytes: Option<usize>,
    sha256: Option<String>,
    deterministic: bool,
    outcome: Outcome,
    tolerance: Tolerance,
    provenance: &'a str,
    expect: &'a Expect,
}

#[derive(Serialize)]
struct FontPin {
    file: String,
    sha256: String,
}

#[derive(Serialize)]
struct Manifest<'a> {
    schema: u32,
    generator: &'a str,
    note: &'a str,
    cases: usize,
    fonts: Vec<FontPin>,
    case: Vec<ManifestCase<'a>>,
}

fn all_cases(ctx: &Ctx) -> Result<Vec<Case>> {
    let mut out = Vec::new();
    text_cases::generate(ctx, &mut out)?;
    graphics_cases::generate(ctx, &mut out)?;
    form_cases::generate(ctx, &mut out)?;
    annot_cases::generate(ctx, &mut out)?;
    structure_cases::generate(ctx, &mut out)?;
    hostile_cases::generate(ctx, &mut out)?;
    let mut seen = BTreeSet::new();
    for c in &out {
        if !seen.insert(c.id.clone()) {
            bail!("duplicate case id {}", c.id);
        }
    }
    Ok(out)
}

fn manifest_text(cases: &[Case], fonts: &[(String, String)], committed: bool) -> Result<String> {
    let m = Manifest {
        schema: 1,
        generator: "xtask/src/bdf (cargo xtask bdf-corpus)",
        note: "Generated. Fixtures are never committed; regenerate them with the command above. Hashes cover the exact bytes of each fixture.",
        cases: cases.len(),
        fonts: fonts.iter().map(|(f, h)| FontPin { file: f.clone(), sha256: h.clone() }).collect(),
        case: cases
            .iter()
            .map(|c| ManifestCase {
                id: &c.id,
                family: c.family,
                class: c.class,
                description: &c.desc,
                bytes: if committed && !c.deterministic { None } else { Some(c.bytes.len()) },
                sha256: if committed && !c.deterministic { None } else { Some(sha256_hex(&c.bytes)) },
                deterministic: c.deterministic,
                outcome: c.outcome,
                tolerance: c.tolerance,
                provenance: &c.provenance,
                expect: &c.expect,
            })
            .collect(),
    };
    toml::to_string_pretty(&m).context("serializing the manifest")
}

/// `cargo xtask bdf-corpus [--out DIR] [--only ID[,ID…]] [--mutool PATH] [--write-manifest] [--list]`
pub fn corpus(args: &[String]) -> Result<()> {
    let root = root();
    let mut out_dir = crate::gates::target_dir().join("bdf-corpus");
    let mut only: Option<BTreeSet<String>> = None;
    let mut mutool = None;
    let mut write_manifest = false;
    let mut list = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out_dir = PathBuf::from(it.next().context("--out needs a directory")?),
            "--only" => only = Some(it.next().context("--only needs ids")?.split(',').map(str::to_string).collect()),
            "--mutool" => mutool = Some(PathBuf::from(it.next().context("--mutool needs a path")?)),
            "--write-manifest" => write_manifest = true,
            "--list" => list = true,
            other => bail!("unknown argument `{other}`"),
        }
    }
    let (ctx, fonts) = Ctx::load(&root, mutool)?;
    let mut cases = all_cases(&ctx)?;
    if let Some(only) = &only {
        cases.retain(|c| only.contains(&c.id));
        if cases.is_empty() {
            bail!("--only matched no case");
        }
    }
    if list {
        for c in &cases {
            println!("{:<34} {:<10} {:>10} bytes  {}", c.id, c.family, c.bytes.len(), c.desc);
        }
        return Ok(());
    }
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    for c in &cases {
        let path = out_dir.join(format!("{}.pdf", c.id));
        std::fs::write(&path, &c.bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    let local = manifest_text(&cases, &fonts, false)?;
    std::fs::write(out_dir.join("manifest.toml"), &local)?;
    let mut by_family = std::collections::BTreeMap::<&str, (usize, usize)>::new();
    for c in &cases {
        let e = by_family.entry(c.family).or_default();
        e.0 += 1;
        e.1 += c.bytes.len();
    }
    let mut summary = String::new();
    for (f, (n, b)) in &by_family {
        let _ = writeln!(summary, "  {f:<12} {n:>3} cases {:>10.1} MiB", *b as f64 / 1_048_576.0);
    }
    println!("bdf-corpus: {} cases written to {}\n{summary}", cases.len(), out_dir.display());
    if write_manifest && only.is_none() {
        let path = root.join("docs").join("bdf-evidence").join("BUILD-002-corpus-manifest.toml");
        std::fs::write(&path, manifest_text(&cases, &fonts, true)?)?;
        println!("bdf-corpus: committed manifest refreshed: {}", path.display());
    } else if write_manifest {
        bail!("--write-manifest needs the full corpus (do not combine with --only)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The families that build in milliseconds; the size-class ones (scans, bombs, plans) are checked
    /// by running the command itself.
    fn light(ctx: &Ctx) -> Vec<Case> {
        let mut v = Vec::new();
        text_cases::generate(ctx, &mut v).unwrap();
        form_cases::generate(ctx, &mut v).unwrap();
        annot_cases::generate(ctx, &mut v).unwrap();
        structure_cases::generate(ctx, &mut v).unwrap();
        v
    }

    #[test]
    fn light_families_are_deterministic_and_have_unique_ids() {
        let (ctx, _) = Ctx::load(&root(), None).unwrap();
        let (a, b) = (light(&ctx), light(&ctx));
        assert!(a.len() >= 35, "{} cases", a.len());
        let mut ids = BTreeSet::new();
        for (x, y) in a.iter().zip(&b) {
            assert!(ids.insert(x.id.clone()), "duplicate id {}", x.id);
            assert_eq!(sha256_hex(&x.bytes), sha256_hex(&y.bytes), "{} differs between two runs", x.id);
        }
    }

    #[test]
    fn expectations_are_consistent_with_the_case() {
        let (ctx, _) = Ctx::load(&root(), None).unwrap();
        for c in light(&ctx) {
            if !c.expect.sizes.is_empty() {
                assert_eq!(Some(c.expect.sizes.len()), c.expect.pages, "{}: one size per page", c.id);
            }
            for (page, token) in &c.expect.text {
                assert!(c.expect.pages.is_none_or(|n| *page <= n), "{}: text expected on page {page}", c.id);
                assert!(!token.is_empty(), "{}: empty token", c.id);
            }
            assert!(c.bytes.starts_with(b"%PDF-"), "{} does not start like a PDF", c.id);
        }
    }

    #[test]
    fn the_manifest_serializes_without_hashes_for_derived_cases() {
        let (ctx, fonts) = Ctx::load(&root(), None).unwrap();
        let mut cases = light(&ctx);
        cases[0].deterministic = false;
        let text = manifest_text(&cases, &fonts, true).unwrap();
        assert!(text.contains("[[case]]"));
        assert!(text.contains("deterministic = false"));
        let local = manifest_text(&cases, &fonts, false).unwrap();
        // Every case carries a hash locally; the font pins carry one each too.
        assert_eq!(local.matches("sha256 =").count(), cases.len() + fonts.len());
        assert_eq!(text.matches("sha256 =").count(), cases.len() - 1 + fonts.len());
    }
}

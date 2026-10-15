//! The BUILD-002 corpus data model: one [`Case`] per fixture, with what its author knows to be in
//! it ([`Expect`]) and how a render of it may differ from the oracle's ([`Tolerance`]).
//!
//! Expectations come from the generator, not from any parser: the generator wrote the text, the
//! fields and the outline, so it can say what must be found. That is the semantic oracle; the
//! external reader is a second, independent check.

use serde::Serialize;

/// What a conforming reader may do with the fixture.
#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Opens and renders every page without repair.
    Opens,
    /// Any of: opens, opens after repair, or refuses with a specific, actionable error. Never a
    /// crash or hang, and never a silent blank page when a lenient reader recovers content.
    Robust,
    /// Must be refused with an error (over a documented admission limit).
    Refused,
    /// Needs a password; opens with `password` and refuses without it.
    Password,
}

/// How far a render of a page may differ from the oracle's at the same DPI. Set before the first
/// comparison run and revised only with a recorded reason (spec §9).
#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Tolerance {
    /// Text-dominated pages: glyph antialiasing and hinting differ between rasterizers.
    Text,
    /// Strokes, fills, gradients and transparency.
    Vector,
    /// Photographs and scans: resampling differs.
    Image,
    /// No pixel comparison (refusal, hostile or damaged inputs): only the outcome is checked.
    None,
}

/// The numeric limits of a tolerance class, applied to [`super::imgcmp::Metrics`].
///
/// **These were fixed before the first oracle comparison was run** (they are committed ahead of the
/// harness that uses them) so they cannot have been tuned to the results. They are starting
/// values: a class is revised only with a recorded reason after the measured distribution is
/// known, never to turn a failing case into a passing one (spec section 9).
///
/// A render passes when its `diff_frac` and `mae` are at or below the limits and its `ink_ratio`
/// lies in `ink`. Every case outside them is triaged by looking at the two images.
#[allow(dead_code)] // used by the oracle harness (next commit)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Fraction of pixels that differ by more than 64 grey levels after a one-pixel allowance.
    pub diff_frac: f64,
    /// Mean absolute difference of 4x4 block averages, in grey levels.
    pub mae: f64,
    /// Allowed range of our total darkness over the oracle's.
    pub ink: (f64, f64),
}

impl Tolerance {
    #[allow(dead_code)] // used by the oracle harness (next commit)
    pub fn limits(self) -> Option<Limits> {
        match self {
            // Glyphs differ in hinting and weight between rasterizers, so text gets the widest ink range.
            Self::Text => Some(Limits { diff_frac: 0.03, mae: 4.0, ink: (0.80, 1.25) }),
            // Strokes, fills and gradients have exact geometry; only edge antialiasing may differ.
            Self::Vector => Some(Limits { diff_frac: 0.015, mae: 3.0, ink: (0.90, 1.10) }),
            // Pictures are resampled differently, so pixels move more, but total tone must agree.
            Self::Image => Some(Limits { diff_frac: 0.05, mae: 5.0, ink: (0.95, 1.05) }),
            Self::None => None,
        }
    }
}

/// What the author knows is in the fixture.
#[derive(Clone, Serialize, Default, Debug)]
pub struct Expect {
    pub pages: Option<usize>,
    /// `(page, token)`: the token must appear in the text extracted from that 1-based page; page 0
    /// means anywhere in the document (used when damage makes page numbers unreliable).
    pub text: Vec<(usize, String)>,
    /// Page sizes in points after `/CropBox`, `/Rotate` and `/UserUnit`, width then height.
    pub sizes: Vec<[f64; 2]>,
    pub annotations: Option<usize>,
    pub fields: Vec<String>,
    pub outlines: Vec<String>,
    pub attachments: Vec<String>,
    pub page_labels: Vec<String>,
    pub password: Option<String>,
    pub notes: String,
}

/// One fixture.
pub struct Case {
    pub id: String,
    /// Family, e.g. `office`, `forms`, `hostile`.
    pub family: &'static str,
    /// Timing class for the PRD budgets: `ordinary`, `scan`, `plan`, `small`, `hostile`.
    pub class: &'static str,
    pub desc: String,
    pub bytes: Vec<u8>,
    pub outcome: Outcome,
    pub tolerance: Tolerance,
    pub expect: Expect,
    /// Where the bytes come from, for the provenance record.
    pub provenance: String,
    /// False when the bytes depend on something outside the generator (an external tool's random
    /// salts); the committed manifest then records the recipe instead of a hash.
    pub deterministic: bool,
}

impl Case {
    pub fn new(id: &str, family: &'static str, class: &'static str, desc: &str, bytes: Vec<u8>) -> Self {
        Self {
            id: id.to_string(),
            family,
            class,
            desc: desc.to_string(),
            bytes,
            outcome: Outcome::Opens,
            tolerance: Tolerance::Text,
            expect: Expect::default(),
            provenance: "synthetic: generated by xtask/src/bdf from source; no external content".to_string(),
            deterministic: true,
        }
    }

    pub fn outcome(mut self, o: Outcome) -> Self {
        self.outcome = o;
        self
    }

    pub fn tolerance(mut self, t: Tolerance) -> Self {
        self.tolerance = t;
        self
    }

    pub fn pages(mut self, n: usize) -> Self {
        self.expect.pages = Some(n);
        self
    }

    pub fn text(mut self, page: usize, token: &str) -> Self {
        self.expect.text.push((page, token.to_string()));
        self
    }

    pub fn note(mut self, n: &str) -> Self {
        self.expect.notes = n.to_string();
        self
    }
}

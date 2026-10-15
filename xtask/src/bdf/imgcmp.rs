//! Pixel comparison of two renders of the same page by two independent rasterizers.
//!
//! Two correct renderers never agree pixel for pixel: glyph hinting, antialiasing and resampling
//! differ. So the metrics are built to ignore a one-pixel shift and edge softness but not missing
//! or misplaced content:
//!
//! * `diff_frac`: the fraction of pixels that differ by more than [`BIG`] grey levels *even after*
//!   looking at the 3x3 neighbourhood in the other image (checked both ways, so content that is
//!   present in only one image counts);
//! * `mae`: the mean absolute difference of 4x4 block averages, which smooths antialiasing but
//!   keeps tone and layout;
//! * `ink_ratio`: total darkness of ours over total darkness of theirs, which catches missing or
//!   doubled content that is small in area;
//! * `max_block`: the largest difference between the mean tone of any 16x16 block in the two
//!   images. The three metrics above are averages over the page, so a small, localized error (an
//!   arrow on the wrong side of a rotated page, one missing word) hides in them. This one cannot
//!   hide: it was added after the negative control showed that a page rotated by 90 degrees instead
//!   of 270 was accepted (see `model::Limits`).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// A difference larger than this many grey levels (of 255) counts as a real difference.
pub const BIG: i32 = 64;

/// A grey image: 0 is black, 255 white. Transparency is composited over white.
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    fn at(&self, x: usize, y: usize) -> i32 {
        i32::from(self.px.get(y * self.w + x).copied().unwrap_or(255))
    }
}

/// Decode a PNG to grey.
pub fn load(path: &Path) -> Result<Gray> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut dec = png::Decoder::new(BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().with_context(|| format!("reading {}", path.display()))?;
    let mut buf = vec![0u8; reader.output_buffer_size().context("image too large")?];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = buf.get(..info.buffer_size()).context("short image buffer")?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => bail!("indexed PNG was not expanded"),
    };
    let mut px = Vec::with_capacity(w * h);
    for p in data.chunks_exact(channels) {
        let (luma, alpha) = match channels {
            1 => (f64::from(p[0]), 255.0),
            2 => (f64::from(p[0]), f64::from(p[1])),
            3 => (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]), 255.0),
            _ => (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]), f64::from(p[3])),
        };
        let a = alpha / 255.0;
        px.push((luma * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8);
    }
    Ok(Gray { w, h, px })
}

/// What the comparison found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Width and height of ours, then theirs.
    pub ours: (usize, usize),
    pub theirs: (usize, usize),
    /// Both dimensions within one pixel of each other.
    pub size_match: bool,
    pub diff_frac: f64,
    pub mae: f64,
    pub ink_ratio: f64,
    pub max_block: f64,
}

/// Smallest difference between `a[x,y]` and any pixel of the 3x3 neighbourhood of `(x,y)` in `b`.
fn near(a: i32, b: &Gray, x: usize, y: usize) -> i32 {
    let mut best = i32::MAX;
    for ny in y.saturating_sub(1)..=(y + 1).min(b.h.saturating_sub(1)) {
        for nx in x.saturating_sub(1)..=(x + 1).min(b.w.saturating_sub(1)) {
            best = best.min((a - b.at(nx, ny)).abs());
        }
    }
    best
}

fn block_mean(g: &Gray, bx: usize, by: usize, size: usize) -> f64 {
    let (mut sum, mut n) = (0i64, 0i64);
    for y in by * size..(by * size + size).min(g.h) {
        for x in bx * size..(bx * size + size).min(g.w) {
            sum += i64::from(g.at(x, y));
            n += 1;
        }
    }
    if n == 0 { 255.0 } else { sum as f64 / n as f64 }
}

/// Compare over the region both images cover.
pub fn compare(ours: &Gray, theirs: &Gray) -> Metrics {
    let (w, h) = (ours.w.min(theirs.w), ours.h.min(theirs.h));
    let size_match = ours.w.abs_diff(theirs.w) <= 1 && ours.h.abs_diff(theirs.h) <= 1;
    let mut bad = 0u64;
    let (mut ink_a, mut ink_b) = (0f64, 0f64);
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (ours.at(x, y), theirs.at(x, y));
            ink_a += f64::from(255 - a);
            ink_b += f64::from(255 - b);
            if (a - b).abs() > BIG && near(a, theirs, x, y) > BIG && near(b, ours, x, y) > BIG {
                bad += 1;
            }
        }
    }
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut mae = 0f64;
    for by in 0..bh {
        for bx in 0..bw {
            mae += (block_mean(ours, bx, by, 4) - block_mean(theirs, bx, by, 4)).abs();
        }
    }
    let mut max_block = 0f64;
    for by in 0..h.div_ceil(16) {
        for bx in 0..w.div_ceil(16) {
            max_block = max_block.max((block_mean(ours, bx, by, 16) - block_mean(theirs, bx, by, 16)).abs());
        }
    }
    let cells = (bw * bh).max(1) as f64;
    let ink_ratio = if ink_b > 0.0 {
        ink_a / ink_b
    } else if ink_a > 0.0 {
        f64::INFINITY
    } else {
        1.0
    };
    Metrics {
        ours: (ours.w, ours.h),
        theirs: (theirs.w, theirs.h),
        size_match,
        diff_frac: bad as f64 / ((w * h).max(1)) as f64,
        mae: mae / cells,
        ink_ratio,
        max_block,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, v: u8) -> Gray {
        Gray { w, h, px: vec![v; w * h] }
    }

    fn with_box(mut g: Gray, x0: usize, y0: usize, size: usize, v: u8) -> Gray {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                g.px[y * g.w + x] = v;
            }
        }
        g
    }

    #[test]
    fn identical_images_have_no_difference() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let m = compare(&a, &a);
        assert_eq!((m.diff_frac, m.mae), (0.0, 0.0));
        assert!((m.ink_ratio - 1.0).abs() < 1e-9);
        assert!(m.size_match);
    }

    #[test]
    fn a_one_pixel_shift_is_ignored() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let b = with_box(flat(64, 64, 255), 11, 10, 20, 0);
        let m = compare(&a, &b);
        assert_eq!(m.diff_frac, 0.0, "{m:?}");
    }

    #[test]
    fn a_three_pixel_shift_is_seen() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let b = with_box(flat(64, 64, 255), 14, 10, 20, 0);
        assert!(compare(&a, &b).diff_frac > 0.0);
    }

    #[test]
    fn missing_content_is_seen_in_every_metric() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let blank = flat(64, 64, 255);
        let m = compare(&blank, &a);
        assert!(m.diff_frac > 0.05, "{m:?}");
        assert!(m.mae > 5.0, "{m:?}");
        assert_eq!(m.ink_ratio, 0.0);
        assert!(compare(&a, &blank).ink_ratio.is_infinite());
    }

    #[test]
    fn a_small_localized_error_is_caught_by_the_block_metric() {
        // A 12x12 dark square on a 400x400 page: the page averages barely move, the block does not.
        let a = with_box(flat(400, 400, 255), 100, 100, 12, 0);
        let m = compare(&a, &flat(400, 400, 255));
        assert!(m.diff_frac < 0.01 && m.mae < 1.0, "{m:?}");
        assert!(m.max_block > 100.0, "{m:?}");
        // The same square one pixel over is still not an error.
        let b = with_box(flat(400, 400, 255), 101, 100, 12, 0);
        assert!(compare(&a, &b).max_block < 30.0);
    }

    #[test]
    fn faint_antialiasing_noise_is_not_a_difference() {
        let a = flat(64, 64, 255);
        let b = flat(64, 64, 240);
        let m = compare(&a, &b);
        assert_eq!(m.diff_frac, 0.0);
        assert!(m.mae < 16.0);
    }

    #[test]
    fn a_size_difference_of_two_pixels_is_flagged() {
        let m = compare(&flat(64, 64, 255), &flat(66, 64, 255));
        assert!(!m.size_match);
        assert!(compare(&flat(64, 64, 255), &flat(65, 63, 255)).size_match);
    }
}

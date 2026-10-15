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
//!
//! All four work on grey, so on their own they cannot see a wrong colour: solid red (255,0,0) and
//! green (0,130,0) have the same luma. Three more metrics run on the composited RGB image and mirror
//! the first three (`color_diff_frac`, `color_mae`, `max_color_block`), with a difference of a pixel or
//! a block measured as the largest difference in any channel. Transparency is composited over white
//! before either set is computed.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// A difference larger than this many grey levels (of 255) counts as a real difference.
pub const BIG: i32 = 64;

/// A difference of more than this in any colour channel (of 255) counts as a real colour difference.
/// The same size as [`BIG`]: a channel error as large as a visible grey error is judged the same way.
pub const BIG_RGB: i32 = 64;

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

/// A decoded render: the grey image the first metrics use, and the colour image (transparency
/// composited over white) the colour metrics use. Same size.
pub struct Image {
    pub gray: Gray,
    pub rgb: Vec<[u8; 3]>,
}

impl Image {
    /// A colourless image (grey replicated into the three channels).
    #[cfg(test)]
    pub fn from_gray(gray: Gray) -> Self {
        let rgb = gray.px.iter().map(|&v| [v, v, v]).collect();
        Self { gray, rgb }
    }

    fn rgb_at(&self, x: usize, y: usize) -> [i32; 3] {
        match self.rgb.get(y * self.gray.w + x) {
            Some(p) => [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])],
            None => [255, 255, 255],
        }
    }
}

/// Decode a PNG to grey and colour.
pub fn load(path: &Path) -> Result<Image> {
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
    let mut rgb = Vec::with_capacity(w * h);
    let over_white = |c: f64, a: f64| (c * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    for p in data.chunks_exact(channels) {
        // Samples are plain (not premultiplied), as PNG stores them.
        let (c, alpha) = match channels {
            1 => ([f64::from(p[0]); 3], 255.0),
            2 => ([f64::from(p[0]); 3], f64::from(p[1])),
            3 => ([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], 255.0),
            _ => ([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], f64::from(p[3])),
        };
        let a = alpha / 255.0;
        let px3 = [over_white(c[0], a), over_white(c[1], a), over_white(c[2], a)];
        // Luma of the composited colour (compositing is linear, so this equals compositing the luma).
        let luma = 0.299 * f64::from(px3[0]) + 0.587 * f64::from(px3[1]) + 0.114 * f64::from(px3[2]);
        px.push(luma.round().clamp(0.0, 255.0) as u8);
        rgb.push(px3);
    }
    Ok(Image { gray: Gray { w, h, px }, rgb })
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
    /// Like `diff_frac`, but a pixel differs when any colour channel differs by more than
    /// [`BIG_RGB`] after the same one-pixel allowance.
    pub color_diff_frac: f64,
    /// Like `mae`: mean over 4x4 blocks of the largest channel difference of the block's mean colour.
    pub color_mae: f64,
    /// Like `max_block`: the largest channel difference of the mean colour of any 16x16 block.
    pub max_color_block: f64,
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

/// The grey-level metrics, over the region both images cover.
pub fn compare_gray(ours: &Gray, theirs: &Gray) -> Metrics {
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
        color_diff_frac: 0.0,
        color_mae: 0.0,
        max_color_block: 0.0,
    }
}

fn channel_gap(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

/// Smallest channel gap between `a` and any pixel of the 3x3 neighbourhood of `(x,y)` in `b`.
fn near_rgb(a: [i32; 3], b: &Image, x: usize, y: usize) -> i32 {
    let mut best = i32::MAX;
    for ny in y.saturating_sub(1)..=(y + 1).min(b.gray.h.saturating_sub(1)) {
        for nx in x.saturating_sub(1)..=(x + 1).min(b.gray.w.saturating_sub(1)) {
            best = best.min(channel_gap(a, b.rgb_at(nx, ny)));
        }
    }
    best
}

fn block_mean_rgb(g: &Image, bx: usize, by: usize, size: usize) -> [f64; 3] {
    let (mut sum, mut n) = ([0i64; 3], 0i64);
    for y in by * size..(by * size + size).min(g.gray.h) {
        for x in bx * size..(bx * size + size).min(g.gray.w) {
            let p = g.rgb_at(x, y);
            for (s, c) in sum.iter_mut().zip(p) {
                *s += i64::from(c);
            }
            n += 1;
        }
    }
    if n == 0 { [255.0; 3] } else { [sum[0] as f64 / n as f64, sum[1] as f64 / n as f64, sum[2] as f64 / n as f64] }
}

fn block_gap(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

/// Compare two renders: the grey-level metrics and the colour metrics.
pub fn compare(ours: &Image, theirs: &Image) -> Metrics {
    let mut m = compare_gray(&ours.gray, &theirs.gray);
    let (w, h) = (ours.gray.w.min(theirs.gray.w), ours.gray.h.min(theirs.gray.h));
    let mut bad = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (ours.rgb_at(x, y), theirs.rgb_at(x, y));
            if channel_gap(a, b) > BIG_RGB && near_rgb(a, theirs, x, y) > BIG_RGB && near_rgb(b, ours, x, y) > BIG_RGB {
                bad += 1;
            }
        }
    }
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut mae = 0f64;
    for by in 0..bh {
        for bx in 0..bw {
            mae += block_gap(block_mean_rgb(ours, bx, by, 4), block_mean_rgb(theirs, bx, by, 4));
        }
    }
    let mut max_block = 0f64;
    for by in 0..h.div_ceil(16) {
        for bx in 0..w.div_ceil(16) {
            max_block = max_block.max(block_gap(block_mean_rgb(ours, bx, by, 16), block_mean_rgb(theirs, bx, by, 16)));
        }
    }
    m.color_diff_frac = bad as f64 / ((w * h).max(1)) as f64;
    m.color_mae = mae / ((bw * bh).max(1)) as f64;
    m.max_color_block = max_block;
    m
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
        let m = compare_gray(&a, &a);
        assert_eq!((m.diff_frac, m.mae), (0.0, 0.0));
        assert!((m.ink_ratio - 1.0).abs() < 1e-9);
        assert!(m.size_match);
    }

    #[test]
    fn a_one_pixel_shift_is_ignored() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let b = with_box(flat(64, 64, 255), 11, 10, 20, 0);
        let m = compare_gray(&a, &b);
        assert_eq!(m.diff_frac, 0.0, "{m:?}");
    }

    #[test]
    fn a_three_pixel_shift_is_seen() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let b = with_box(flat(64, 64, 255), 14, 10, 20, 0);
        assert!(compare_gray(&a, &b).diff_frac > 0.0);
    }

    #[test]
    fn missing_content_is_seen_in_every_metric() {
        let a = with_box(flat(64, 64, 255), 10, 10, 20, 0);
        let blank = flat(64, 64, 255);
        let m = compare_gray(&blank, &a);
        assert!(m.diff_frac > 0.05, "{m:?}");
        assert!(m.mae > 5.0, "{m:?}");
        assert_eq!(m.ink_ratio, 0.0);
        assert!(compare_gray(&a, &blank).ink_ratio.is_infinite());
    }

    #[test]
    fn a_small_localized_error_is_caught_by_the_block_metric() {
        // A 12x12 dark square on a 400x400 page: the page averages barely move, the block does not.
        let a = with_box(flat(400, 400, 255), 100, 100, 12, 0);
        let m = compare_gray(&a, &flat(400, 400, 255));
        assert!(m.diff_frac < 0.01 && m.mae < 1.0, "{m:?}");
        assert!(m.max_block > 100.0, "{m:?}");
        // The same square one pixel over is still not an error.
        let b = with_box(flat(400, 400, 255), 101, 100, 12, 0);
        assert!(compare_gray(&a, &b).max_block < 30.0);
    }

    #[test]
    fn faint_antialiasing_noise_is_not_a_difference() {
        let a = flat(64, 64, 255);
        let b = flat(64, 64, 240);
        let m = compare_gray(&a, &b);
        assert_eq!(m.diff_frac, 0.0);
        assert!(m.mae < 16.0);
    }

    #[test]
    fn a_size_difference_of_two_pixels_is_flagged() {
        let m = compare_gray(&flat(64, 64, 255), &flat(66, 64, 255));
        assert!(!m.size_match);
        assert!(compare_gray(&flat(64, 64, 255), &flat(65, 63, 255)).size_match);
    }

    fn luma(p: [u8; 3]) -> u8 {
        (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])).round() as u8
    }

    fn solid(w: usize, h: usize, p: [u8; 3]) -> Image {
        Image { gray: Gray { w, h, px: vec![luma(p); w * h] }, rgb: vec![p; w * h] }
    }

    fn with_square(mut im: Image, x0: usize, y0: usize, size: usize, p: [u8; 3]) -> Image {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                let i = y * im.gray.w + x;
                im.rgb[i] = p;
                im.gray.px[i] = luma(p);
            }
        }
        im
    }

    /// Codex's reproduction of R5: solid red and solid green of equal luma.
    #[test]
    fn red_and_green_of_equal_luma_differ_in_every_colour_metric_and_in_none_of_the_grey_ones() {
        let (red, green) = (solid(64, 64, [255, 0, 0]), solid(64, 64, [0, 130, 0]));
        let m = compare(&red, &green);
        assert_eq!((m.diff_frac, m.mae, m.max_block), (0.0, 0.0, 0.0), "the grey metrics are blind to it: {m:?}");
        assert!((m.ink_ratio - 1.0).abs() < 1e-9);
        assert!(m.color_diff_frac > 0.99 && m.color_mae > 200.0 && m.max_color_block > 200.0, "{m:?}");
    }

    #[test]
    fn colour_metrics_keep_the_geometry_and_noise_allowances_of_the_grey_ones() {
        let page = || solid(64, 64, [255, 255, 255]);
        let a = with_square(page(), 10, 10, 20, [200, 30, 30]);
        // A one-pixel shift is not an error.
        let shifted = with_square(page(), 11, 10, 20, [200, 30, 30]);
        assert_eq!(compare(&a, &shifted).color_diff_frac, 0.0);
        // A four-pixel shift is.
        assert!(compare(&a, &with_square(page(), 14, 10, 20, [200, 30, 30])).color_diff_frac > 0.0);
        // A faint tint (20 levels in a channel) is antialiasing-scale noise.
        let tinted = with_square(page(), 10, 10, 20, [220, 40, 20]);
        let m = compare(&a, &tinted);
        assert_eq!(m.color_diff_frac, 0.0);
        assert!(m.max_color_block < 30.0, "{m:?}");
    }

    #[test]
    fn a_small_wrong_coloured_object_is_caught_by_the_colour_block_metric() {
        // A 12x12 square of the wrong colour on a 400x400 page: the page averages barely move.
        let page = || solid(400, 400, [255, 255, 255]);
        let m = compare(&with_square(page(), 100, 100, 12, [255, 0, 0]), &with_square(page(), 100, 100, 12, [0, 130, 0]));
        assert!(m.color_diff_frac < 0.01 && m.color_mae < 3.0, "{m:?}");
        assert!(m.max_color_block > 100.0, "{m:?}");
    }

    #[test]
    fn identical_colour_images_have_no_colour_difference() {
        let a = with_square(solid(64, 64, [255, 255, 255]), 5, 5, 30, [10, 120, 200]);
        let m = compare(&a, &a);
        assert_eq!((m.color_diff_frac, m.color_mae, m.max_color_block), (0.0, 0.0, 0.0));
    }

    #[test]
    fn a_grey_image_has_the_same_colour_and_grey_metrics() {
        let a = Image::from_gray(with_box(flat(64, 64, 255), 10, 10, 20, 0));
        let b = Image::from_gray(with_box(flat(64, 64, 255), 14, 10, 20, 0));
        let m = compare(&a, &b);
        assert_eq!(m.color_diff_frac, m.diff_frac);
    }
}

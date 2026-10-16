//! Corpus families built from drawing operations: vector graphics, plan sheets, scans and page
//! geometry. No trigonometry anywhere: coordinates come from integers and the Bézier circle
//! constant, so the bytes (and hashes) do not depend on the platform's math library.

use std::fmt::Write as _;

use anyhow::{Result, ensure};

use super::Ctx;
use super::model::{Case, Tolerance};
use super::pdfgen::{Doc, Page, Prng, lit, num};
use super::text_cases::block;

/// Kappa for approximating a quarter circle with one cubic Bézier.
const K: f64 = 0.552_284_75;

fn circle(cx: f64, cy: f64, r: f64) -> String {
    let k = K * r;
    format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h\n",
        num(cx + r),
        num(cy),
        num(cx + r),
        num(cy + k),
        num(cx + k),
        num(cy + r),
        num(cx),
        num(cy + r),
        num(cx - k),
        num(cy + r),
        num(cx - r),
        num(cy + k),
        num(cx - r),
        num(cy),
        num(cx - r),
        num(cy - k),
        num(cx - k),
        num(cy - r),
        num(cx),
        num(cy - r),
        num(cx + k),
        num(cy - r),
        num(cx + r),
        num(cy - k),
        num(cx + r),
        num(cy),
    )
}

/// A five-point star as an integer polygon centred near `(cx, cy)` with radius about `r`
/// (precomputed unit coordinates, scaled).
fn star(cx: f64, cy: f64, r: f64) -> String {
    const PTS: [(f64, f64); 5] = [(0.0, 1.0), (-0.588, -0.809), (0.951, 0.309), (-0.951, 0.309), (0.588, -0.809)];
    let mut s = String::new();
    for (i, (x, y)) in PTS.iter().enumerate() {
        let _ = writeln!(s, "{} {} {}", num((cx + x * r).round()), num((cy + y * r).round()), if i == 0 { "m" } else { "l" });
    }
    s.push_str("h\n");
    s
}

fn vector_page(content: String, resources: String) -> Page {
    Page::letter().with_content(content.into_bytes()).with_resources(resources)
}

fn vector_case(id: &str, desc: &str, doc: Doc) -> Result<Case> {
    Ok(Case::new(id, "vector", "small", desc, doc.finish()?).tolerance(Tolerance::Vector).pages(1))
}

fn vector(out: &mut Vec<Case>) -> Result<()> {
    // Line widths, caps, joins, miter limits and dashes.
    {
        let mut doc = Doc::new("v-strokes");
        let mut c = String::new();
        for (i, w) in [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 12.0].iter().enumerate() {
            let _ = writeln!(c, "{} w 0 0 0 RG 72 {} m 300 {} l S", num(*w), 720 - i * 24, 720 - i * 24);
        }
        for cap in 0..3 {
            let _ = writeln!(c, "14 w {cap} J 0.8 0.1 0.1 RG {} 520 m {} 520 l S", 330 + cap * 70, 370 + cap * 70);
        }
        for join in 0..3 {
            let _ = writeln!(c, "10 w {join} j 0.1 0.1 0.8 RG {} 440 m {} 490 l {} 440 l S", 330 + join * 70, 345 + join * 70, 360 + join * 70);
        }
        c.push_str("4 w 0 J 0 j 0 0 0 RG [12 6] 0 d 72 380 m 540 380 l S [20 4 2 4] 5 d 72 360 m 540 360 l S [1 6] 0 d 1 J 72 340 m 540 340 l S [] 0 d 0 J\n");
        c.push_str("2 w 0 0.5 0 RG 72 200 m 150 320 250 80 330 200 c 400 300 480 120 540 220 c S\n");
        doc.add_page(vector_page(c, String::new()))?;
        out.push(vector_case("vector-strokes", "Line widths, caps, joins, dashes and cubic curves", doc)?);
    }
    // Fill rules.
    {
        let mut doc = Doc::new("v-fillrules");
        let mut c = String::from("0.9 0.4 0.1 rg\n");
        c.push_str(&star(180.0, 560.0, 110.0));
        c.push_str("f\n0.1 0.4 0.9 rg\n");
        c.push_str(&star(430.0, 560.0, 110.0));
        c.push_str("f*\n0.2 0.6 0.2 rg 72 300 200 100 re 112 320 120 60 re f*\n0.5 0.2 0.6 rg 320 300 200 100 re 360 320 120 60 re f\n");
        doc.add_page(vector_page(c, String::new()))?;
        out.push(vector_case("vector-fillrules", "Non-zero and even-odd fills of a star and of nested rectangles", doc)?);
    }
    // Clipping.
    {
        let mut doc = Doc::new("v-clip");
        let mut c = String::from("q\n");
        c.push_str(&circle(306.0, 500.0, 150.0));
        c.push_str("W n\nq\n");
        c.push_str(&circle(306.0, 500.0, 90.0));
        c.push_str("W* n\n");
        for i in 0..40 {
            let _ = writeln!(c, "{} {} {} rg 0 {} 612 8 re f", num(f64::from(i) / 40.0), num(1.0 - f64::from(i) / 40.0), 0.5, 350 + i * 8);
        }
        c.push_str("Q\n0.1 0.1 0.5 rg 100 450 420 30 re f\nQ\n");
        doc.add_page(vector_page(c, String::new()))?;
        out.push(vector_case("vector-clip", "Nested clipping paths with a circle inside a circle", doc)?);
    }
    // Axial and radial shadings.
    for (id, shading, desc) in [
        (
            "vector-shading-axial",
            "/ShadingType 2 /ColorSpace /DeviceRGB /Coords [72 0 540 0] /Extend [true true]",
            "Axial shading between two colours",
        ),
        (
            "vector-shading-radial",
            "/ShadingType 3 /ColorSpace /DeviceRGB /Coords [306 450 10 306 450 200] /Extend [true false]",
            "Radial shading with a hole",
        ),
    ] {
        let mut doc = Doc::new(id);
        let res = format!("/Shading << /Sh1 << {shading} /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0.8 0] /C1 [0.1 0.1 0.8] /N 1 >> >> >>");
        let c = "q 72 250 468 400 re W n /Sh1 sh Q\n0 0 0 RG 1 w 72 250 468 400 re S\n".to_string();
        doc.add_page(vector_page(c, res))?;
        out.push(vector_case(id, desc, doc)?);
    }
    // Transparency and blend modes.
    {
        let mut doc = Doc::new("v-transparency");
        let res = "/ExtGState << /Half << /ca 0.5 /CA 0.5 >> /Mul << /BM /Multiply >> /Opaque << /ca 1 /CA 1 /BM /Normal >> >>";
        let mut c = String::from("/Opaque gs 0.9 0.1 0.1 rg 100 500 200 200 re f\n/Half gs 0.1 0.1 0.9 rg 200 450 200 200 re f\n/Opaque gs\n");
        c.push_str("0.95 0.8 0.1 rg 100 200 200 200 re f\n/Mul gs 0.1 0.8 0.9 rg 200 150 200 200 re f\n");
        doc.add_page(vector_page(c, res.to_string()))?;
        out.push(vector_case("vector-transparency", "Constant alpha and Multiply blending over opaque squares", doc)?);
    }
    // Tiling pattern.
    {
        let mut doc = Doc::new("v-pattern");
        let pat = doc.pdf.add_stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 /Resources << >>",
            b"0.8 0.2 0.2 rg 0 0 10 10 re f 0.2 0.2 0.8 rg 10 10 10 10 re f 0 0 0 RG 0.5 w 0 0 20 20 re S",
            false,
        )?;
        let c = "/Pattern cs /P1 scn 72 250 468 400 re f\n0 0 0 RG 1 w 72 250 468 400 re S\n".to_string();
        doc.add_page(vector_page(c, format!("/Pattern << /P1 {pat} 0 R >>")))?;
        out.push(vector_case("vector-pattern", "A coloured tiling pattern fill", doc)?);
    }
    // Form XObject reused with different matrices, and several colour spaces.
    {
        let mut doc = Doc::new("v-forms-colours");
        let form = doc.pdf.add_stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << >>",
            b"0.2 0.5 0.8 rg 0 0 100 100 re f 1 1 1 rg 20 20 60 60 re f 0.9 0.2 0.2 rg 40 40 20 20 re f",
            true,
        )?;
        let mut c = String::new();
        for i in 0..12 {
            let (col, row) = (i % 4, i / 4);
            let _ = writeln!(c, "q 0.6 0 0 0.6 {} {} cm /Fm1 Do Q", 72 + col * 120, 640 - row * 120);
        }
        c.push_str("0 1 1 0 k 72 220 100 80 re f 1 0 1 0 k 190 220 100 80 re f 1 1 0 0 k 308 220 100 80 re f 0 0 0 1 k 426 220 100 80 re f\n");
        c.push_str("0.25 g 72 120 100 60 re f 0.5 g 190 120 100 60 re f 0.75 g 308 120 100 60 re f\n");
        doc.add_page(vector_page(c, format!("/XObject << /Fm1 {form} 0 R >>")))?;
        out.push(vector_case("vector-forms-colours", "One form XObject drawn twelve times, plus CMYK and grey fills", doc)?);
    }
    // Text render modes and text transforms (fill, stroke, clip, invisible).
    {
        let mut doc = Doc::new("v-text-modes");
        let font = doc.std_font("Helvetica-Bold");
        let mut c = String::new();
        for (i, mode) in [0, 1, 2, 3].iter().enumerate() {
            let _ = writeln!(c, "BT /F1 40 Tf {mode} Tr 1 0 0 rg 0 0 1 RG 1.5 w 72 {} Td (BDFMODE-{mode}) Tj ET", 700 - i * 80);
        }
        c.push_str("BT /F1 30 Tf 0 Tr 0.866 0.5 -0.5 0.866 300 300 Tm 0 g (BDFROT rotated 30) Tj ET\n");
        doc.add_page(vector_page(c, format!("/Font << /F1 {font} 0 R >>")))?;
        out.push(
            vector_case("vector-text-modes", "Fill, stroke, fill-stroke and invisible text and a rotated text matrix", doc)?
                .text(1, "BDFMODE-0")
                .text(1, "BDFROT"),
        );
    }
    Ok(())
}

/// Large-format plan sheets: many path segments, many symbols and many labels.
fn plans(out: &mut Vec<Case>) -> Result<()> {
    let (w, h) = (2592.0, 1728.0); // 36 x 24 inches
    for (id, n, desc) in [
        ("plan-hatch-100k", 100_000usize, "A 36x24 in sheet with 100,000 stroked line segments in one path"),
        ("plan-hatch-400k", 400_000, "A 36x24 in sheet with 400,000 stroked line segments in one path"),
    ] {
        let mut doc = Doc::new(id);
        let mut rng = Prng::new(n as u64);
        let mut c = String::from("0.25 w 0 0 0 RG\n");
        for i in 0..n {
            let x = (rng.next_u64() % 2500) as f64 + 40.0;
            let y = (rng.next_u64() % 1650) as f64 + 40.0;
            let (dx, dy) = ((rng.next_u64() % 40) as f64, (rng.next_u64() % 40) as f64);
            let _ = writeln!(c, "{} {} m {} {} l", num(x), num(y), num(x + dx), num(y + dy));
            if i % 5000 == 4999 {
                c.push_str("S\n");
            }
        }
        c.push_str("S\n");
        doc.add_page(Page::sized(w, h).with_content(c.into_bytes()))?;
        let mut case = Case::new(id, "plans", "plan", desc, doc.finish()?).tolerance(Tolerance::Vector).pages(1);
        case.expect.sizes = vec![[w, h]];
        out.push(case);
    }
    // A symbol form repeated 5,000 times.
    {
        let mut doc = Doc::new("plan-symbols-5k");
        let sym = doc.pdf.add_stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20] /Resources << >>",
            b"0 0 0 RG 1 w 0 0 20 20 re S 0 0 m 20 20 l S 0 20 m 20 0 l S",
            false,
        )?;
        let mut c = String::new();
        for i in 0..5000usize {
            let _ = writeln!(c, "q 1 0 0 1 {} {} cm /S1 Do Q", 40 + (i % 100) * 25, 40 + (i / 100) * 30);
        }
        doc.add_page(Page::sized(w, h).with_content(c.into_bytes()).with_resources(format!("/XObject << /S1 {sym} 0 R >>")))?;
        let mut case = Case::new("plan-symbols-5k", "plans", "plan", "5,000 uses of one symbol form on a 36x24 in sheet", doc.finish()?)
            .tolerance(Tolerance::Vector)
            .pages(1);
        case.expect.sizes = vec![[w, h]];
        out.push(case);
    }
    // 3,000 small text labels.
    {
        let mut doc = Doc::new("plan-labels-3k");
        let font = doc.std_font("Helvetica");
        let mut c = String::new();
        for i in 0..3000usize {
            let _ = writeln!(c, "BT /F1 7 Tf {} {} Td (BDFPLAN-LBL-{i:04}) Tj ET", 40 + (i % 60) * 42, 60 + (i / 60) * 33);
        }
        doc.add_page(Page::sized(w, h).with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {font} 0 R >>")))?;
        let mut case = Case::new("plan-labels-3k", "plans", "plan", "3,000 small text labels on a 36x24 in sheet", doc.finish()?)
            .pages(1)
            .text(1, "BDFPLAN-LBL-0123")
            .text(1, "BDFPLAN-LBL-2999");
        case.expect.sizes = vec![[w, h]];
        out.push(case);
    }
    Ok(())
}

/// A document-like grey raster: paper colour, noise of the given amplitude, and dark bars that
/// resemble text lines.
fn paper(w: usize, h: usize, seed: u64, amp: u64) -> Vec<u8> {
    let mut rng = Prng::new(seed);
    let mut px = vec![0u8; w * h];
    rng.fill(&mut px);
    let amp = amp.max(1);
    for (y, row) in px.chunks_mut(w).enumerate() {
        let band = y % 66 < 26 && y > 150 && y + 150 < h;
        for (x, p) in row.iter_mut().enumerate() {
            let noise = u64::from(*p) % amp;
            let base: u64 = if band && (x / 41 + y / 66 * 7) % 4 != 0 && x > 150 && x + 150 < w { 45 } else { 238 };
            *p = (base + noise).saturating_sub(amp / 2).min(255) as u8;
        }
    }
    px
}

fn scan_page(doc: &mut Doc, w: u32, h: u32, gray: &[u8], extra: &str) -> Result<u32> {
    let im = doc.add_image_gray(w, h, gray, true)?;
    doc.add_page({
        let mut p = Page::letter().with_content(b"q 612 0 0 792 0 0 cm /Im1 Do Q\n".to_vec()).with_resources(format!("/XObject << /Im1 {im} 0 R >>"));
        p.extra = extra.to_string();
        p
    })
}

fn scans(out: &mut Vec<Case>) -> Result<()> {
    let scan =
        |id: &str, desc: &str, doc: Doc| -> Result<Case> { Ok(Case::new(id, "scans", "scan", desc, doc.finish()?).tolerance(Tolerance::Image)) };
    {
        let mut doc = Doc::new("scan-gray-300");
        scan_page(&mut doc, 2550, 3300, &paper(2550, 3300, 1, 12), "")?;
        out.push(scan("scan-gray-300", "One letter page, 8-bit grey at 300 dpi", doc)?.pages(1));
    }
    {
        let mut doc = Doc::new("scan-rgb-300");
        let g = paper(2550, 3300, 2, 12);
        let mut rgb = Vec::with_capacity(g.len() * 3);
        for v in &g {
            rgb.extend_from_slice(&[*v, v.saturating_sub(6), v.saturating_sub(14)]);
        }
        let im = doc.add_image_rgb(2550, 3300, &rgb, true)?;
        doc.add_page(
            Page::letter().with_content(b"q 612 0 0 792 0 0 cm /Im1 Do Q\n".to_vec()).with_resources(format!("/XObject << /Im1 {im} 0 R >>")),
        )?;
        out.push(scan("scan-rgb-300", "One letter page, 24-bit colour at 300 dpi", doc)?.pages(1));
    }
    {
        // 1-bit at 600 dpi: pack the grey page's dark bars to bits.
        let (w, h) = (5100usize, 6600usize);
        let mut bits = vec![0xFFu8; (w / 8) * h];
        for y in 0..h {
            let band = y % 132 < 52 && y > 300 && y + 300 < h;
            if !band {
                continue;
            }
            for x in 300..w - 300 {
                if (x / 82 + y / 132 * 7) % 4 != 0
                    && let Some(b) = bits.get_mut(y * (w / 8) + x / 8)
                {
                    *b &= !(0x80 >> (x % 8));
                }
            }
        }
        let mut doc = Doc::new("scan-bitonal-600");
        let im =
            doc.pdf.add_stream("/Type /XObject /Subtype /Image /Width 5100 /Height 6600 /ColorSpace /DeviceGray /BitsPerComponent 1", &bits, true)?;
        doc.add_page(
            Page::letter().with_content(b"q 612 0 0 792 0 0 cm /Im1 Do Q\n".to_vec()).with_resources(format!("/XObject << /Im1 {im} 0 R >>")),
        )?;
        out.push(scan("scan-bitonal-600", "One letter page, 1-bit black and white at 600 dpi", doc)?.pages(1));
    }
    {
        // The "scan/plan opening" class: a multi-page scan close to, but under, 100 MiB.
        let mut doc = Doc::new("scan-multipage-16");
        for n in 0..16u64 {
            scan_page(&mut doc, 2550, 3300, &paper(2550, 3300, 100 + n, 40), "")?;
        }
        let bytes = doc.finish()?;
        ensure!(bytes.len() < 100 * 1_048_576, "scan-multipage-16 is {} bytes; the class is at most 100 MiB", bytes.len());
        ensure!(bytes.len() > 40 * 1_048_576, "scan-multipage-16 is only {} bytes; it should be a large scan", bytes.len());
        out.push(
            Case::new("scan-multipage-16", "scans", "scan", "Sixteen 300 dpi grey letter pages, a large scanned file under 100 MiB", bytes)
                .tolerance(Tolerance::Image)
                .pages(16),
        );
    }
    {
        let mut doc = Doc::new("scan-rotated-90");
        scan_page(&mut doc, 1275, 1650, &paper(1275, 1650, 7, 12), "/Rotate 90")?;
        let mut case = scan("scan-rotated-90", "A 150 dpi grey scan stored upright but flagged /Rotate 90", doc)?.pages(1);
        case.expect.sizes = vec![[792.0, 612.0]];
        out.push(case);
    }
    {
        // A scan with an invisible OCR text layer (render mode 3) over the image.
        let mut doc = Doc::new("scan-ocr-layer");
        let font = doc.std_font("Helvetica");
        let im = doc.add_image_gray(1275, 1650, &paper(1275, 1650, 9, 12), true)?;
        let c = "q 612 0 0 792 0 0 cm /Im1 Do Q\nBT 3 Tr /F1 12 Tf 72 700 Td (BDFOCR-LAYER invisible recognised text) Tj ET\n";
        doc.add_page(
            Page::letter().with_content(c.as_bytes().to_vec()).with_resources(format!("/XObject << /Im1 {im} 0 R >> /Font << /F1 {font} 0 R >>")),
        )?;
        out.push(scan("scan-ocr-layer", "A scanned page with an invisible OCR text layer", doc)?.pages(1).text(1, "BDFOCR-LAYER"));
    }
    Ok(())
}

/// Page geometry: rotation, crop boxes, odd origins, inherited attributes, user units, mixed sizes.
fn boxes(out: &mut Vec<Case>) -> Result<()> {
    // A frame, a label and an "up" marker so orientation errors are visible.
    fn marked(font: u32, token: &str, w: f64, h: f64, ox: f64, oy: f64) -> (String, String) {
        let mut c = format!("1 w 0 0 0 RG {} {} {} {} re S\n", num(ox + 10.0), num(oy + 10.0), num(w - 20.0), num(h - 20.0));
        c.push_str(&format!(
            "0.8 0.1 0.1 rg {} {} m {} {} l {} {} l f\n",
            num(ox + w / 2.0 - 20.0),
            num(oy + h - 30.0),
            num(ox + w / 2.0 + 20.0),
            num(oy + h - 30.0),
            num(ox + w / 2.0),
            num(oy + h - 10.0)
        ));
        c.push_str(&block("F1", 18.0, ox + 40.0, oy + h - 80.0, 22.0, &[token.to_string(), "UP is toward the red arrow".to_string()], &|s| lit(s)));
        (c, format!("/Font << /F1 {font} 0 R >>"))
    }
    for rot in [90, 180, 270] {
        let id = format!("box-rotate-{rot}");
        let mut doc = Doc::new(&id);
        let font = doc.std_font("Helvetica");
        let (c, res) = marked(font, &format!("BDFBOX-ROTATE-{rot}"), 612.0, 792.0, 0.0, 0.0);
        let mut p = Page::letter().with_content(c.into_bytes()).with_resources(res);
        p.rotate = Some(rot);
        doc.add_page(p)?;
        let mut case = Case::new(&id, "boxes", "small", &format!("Letter page with /Rotate {rot}"), doc.finish()?)
            .pages(1)
            .text(1, &format!("BDFBOX-ROTATE-{rot}"));
        case.expect.sizes = vec![if rot == 180 { [612.0, 792.0] } else { [792.0, 612.0] }];
        out.push(case);
    }
    {
        let mut doc = Doc::new("box-crop");
        let font = doc.std_font("Helvetica");
        let (c, res) = marked(font, "BDFBOX-CROP", 612.0, 792.0, 0.0, 0.0);
        let mut p = Page::letter().with_content(c.into_bytes()).with_resources(res);
        p.crop = Some([100.0, 300.0, 400.0, 700.0]);
        doc.add_page(p)?;
        let mut case =
            Case::new("box-crop", "boxes", "small", "MediaBox letter with a smaller /CropBox; content outside it must not show", doc.finish()?)
                .pages(1);
        case.expect.sizes = vec![[300.0, 400.0]];
        out.push(case);
    }
    {
        let mut doc = Doc::new("box-negative-origin");
        let font = doc.std_font("Helvetica");
        let (c, res) = marked(font, "BDFBOX-NEGATIVE", 612.0, 792.0, -306.0, -396.0);
        let mut p = Page::letter().with_content(c.into_bytes()).with_resources(res);
        p.media = [-306.0, -396.0, 306.0, 396.0];
        doc.add_page(p)?;
        let mut case =
            Case::new("box-negative-origin", "boxes", "small", "A MediaBox centred on the origin", doc.finish()?).pages(1).text(1, "BDFBOX-NEGATIVE");
        case.expect.sizes = vec![[612.0, 792.0]];
        out.push(case);
    }
    {
        // MediaBox, Rotate and Resources all inherited from the page-tree root.
        let mut doc = Doc::new("box-inherited");
        let font = doc.std_font("Helvetica");
        doc.pages_extra = format!("/MediaBox [0 0 595 842] /Rotate 90 /Resources << /Font << /F1 {font} 0 R >> >>");
        let (c, _) = marked(font, "BDFBOX-INHERITED", 595.0, 842.0, 0.0, 0.0);
        let content = doc.pdf.add_stream("", c.as_bytes(), true)?;
        let page = doc.pdf.add(format!("<< /Type /Page /Parent @PARENT@ 0 R /Contents {content} 0 R >>").into_bytes());
        doc.kids.push(page);
        let mut case = Case::new(
            "box-inherited",
            "boxes",
            "small",
            "MediaBox, Rotate and Resources inherited from the Pages node (A4 landscape)",
            doc.finish()?,
        )
        .pages(1)
        .text(1, "BDFBOX-INHERITED");
        case.expect.sizes = vec![[842.0, 595.0]];
        out.push(case);
    }
    {
        let mut doc = Doc::new("box-userunit");
        let font = doc.std_font("Helvetica");
        let (c, res) = marked(font, "BDFBOX-USERUNIT", 306.0, 396.0, 0.0, 0.0);
        let mut p = Page::sized(306.0, 396.0).with_content(c.into_bytes()).with_resources(res);
        p.extra = "/UserUnit 2.0".to_string();
        doc.add_page(p)?;
        let mut case = Case::new("box-userunit", "boxes", "small", "A 306x396-unit page with /UserUnit 2.0 (physically letter size)", doc.finish()?)
            .pages(1)
            .text(1, "BDFBOX-USERUNIT");
        case.expect.sizes = vec![[612.0, 792.0]];
        case.expect.notes =
            "Readers differ on UserUnit support; a render at the unscaled size is a recorded difference, not a defect by itself".to_string();
        out.push(case);
    }
    {
        let mut doc = Doc::new("box-mixed-sizes");
        let font = doc.std_font("Helvetica");
        let mut sizes = Vec::new();
        for (name, w, h) in [("LETTER", 612.0, 792.0), ("A4", 595.0, 842.0), ("TABLOID", 792.0, 1224.0), ("LANDSCAPE", 792.0, 612.0)] {
            let (c, res) = marked(font, &format!("BDFBOX-MIXED-{name}"), w, h, 0.0, 0.0);
            doc.add_page(Page::sized(w, h).with_content(c.into_bytes()).with_resources(res))?;
            sizes.push([w, h]);
        }
        let mut case = Case::new("box-mixed-sizes", "boxes", "small", "Four pages of different sizes and orientations", doc.finish()?)
            .pages(4)
            .text(3, "BDFBOX-MIXED-TABLOID");
        case.expect.sizes = sizes;
        out.push(case);
    }
    Ok(())
}

pub fn generate(_ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    vector(out)?;
    plans(out)?;
    scans(out)?;
    boxes(out)?;
    Ok(())
}

//! Corpus families built from text: office documents, embedded fonts and Unicode.

use anyhow::Result;

use super::Ctx;
use super::model::Case;
use super::pdfgen::{Doc, Page, Prng, embed_cid, embed_simple, lit, num};
use super::structure_cases::{OutlineItem, outline_titles, outlines};

const WORDS: &[&str] = &[
    "report",
    "budget",
    "schedule",
    "drawing",
    "revision",
    "approval",
    "contract",
    "meeting",
    "invoice",
    "summary",
    "review",
    "project",
    "site",
    "design",
    "owner",
    "scope",
    "change",
    "request",
    "safety",
    "quality",
    "delivery",
    "package",
    "survey",
    "estimate",
    "payment",
    "permit",
    "inspection",
    "document",
    "section",
    "detail",
    "elevation",
    "foundation",
    "structure",
    "supplier",
    "contractor",
    "milestone",
    "forecast",
];

/// Deterministic filler text, wrapped at `cols` characters.
pub fn paragraph(rng: &mut Prng, lines: usize, cols: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    for _ in 0..lines {
        let mut line = String::new();
        while line.len() < cols.saturating_sub(12) {
            let w = WORDS.get((rng.next_u64() % WORDS.len() as u64) as usize).copied().unwrap_or("text");
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        out.push(line);
    }
    out
}

/// `BT … ET` showing `lines` from `(x, y)` going down by `leading`; `show` renders one line as a
/// `Tj` operand (a literal string for simple fonts, hex for CID fonts).
pub fn block(font: &str, size: f64, x: f64, y: f64, leading: f64, lines: &[String], show: &dyn Fn(&str) -> String) -> String {
    let mut s = format!("BT /{font} {} Tf {} TL {} {} Td\n", num(size), num(leading), num(x), num(y));
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            s.push_str("T*\n");
        }
        s.push_str(&show(l));
        s.push_str(" Tj\n");
    }
    s.push_str("ET\n");
    s
}

fn lit_show(s: &str) -> String {
    lit(s)
}

pub fn generate(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    office_std14(out)?;
    office_embedded_100(ctx, out)?;
    fonts(ctx, out)?;
    unicode(ctx, out)?;
    Ok(())
}

/// A page of Helvetica text with a unique marker token in its heading.
fn std14_page(doc: &mut Doc, helv: u32, bold: u32, times: u32, rng: &mut Prng, n: usize) -> Result<()> {
    let mut c = String::new();
    c.push_str(&block(
        "F2",
        16.0,
        72.0,
        720.0,
        20.0,
        &[format!("BDFPAGE-{n:04} Quarterly {}", WORDS.get(n % WORDS.len()).copied().unwrap_or("review"))],
        &lit_show,
    ));
    c.push_str(&block("F1", 10.0, 72.0, 690.0, 13.0, &paragraph(rng, 40, 92), &lit_show));
    c.push_str(&block("F3", 9.0, 72.0, 150.0, 11.0, &paragraph(rng, 8, 100), &lit_show));
    let res = format!("/Font << /F1 {helv} 0 R /F2 {bold} 0 R /F3 {times} 0 R >>");
    doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(res))?;
    Ok(())
}

fn office_std14(out: &mut Vec<Case>) -> Result<()> {
    for (id, pages, desc) in [
        ("office-1p-std14", 1usize, "One letter page of Helvetica and Times text (standard 14 fonts, not embedded)"),
        ("office-10p-std14", 10, "Ten letter pages of standard-14 text"),
        ("office-30p-std14", 30, "Thirty letter pages of standard-14 text"),
    ] {
        let mut doc = Doc::new(id);
        let helv = doc.std_font("Helvetica");
        let bold = doc.std_font("Helvetica-Bold");
        let times = doc.std_font("Times-Roman");
        let mut rng = Prng::new(0x00B0_0001 + pages as u64);
        for n in 1..=pages {
            std14_page(&mut doc, helv, bold, times, &mut rng, n)?;
        }
        let mut case = Case::new(id, "office", "ordinary", desc, doc.finish()?).pages(pages);
        for n in [1, pages] {
            case = case.text(n, &format!("BDFPAGE-{n:04}"));
        }
        case.expect.sizes = vec![[612.0, 792.0]; pages];
        out.push(case);
    }
    // Two-column page: the content stream draws the right column first, so reading order and
    // stream order disagree.
    let mut doc = Doc::new("office-two-column");
    let helv = doc.std_font("Helvetica");
    let mut rng = Prng::new(77);
    let mut c = block(
        "F1",
        10.0,
        330.0,
        720.0,
        13.0,
        &std::iter::once("BDFRIGHT column starts here".to_string()).chain(paragraph(&mut rng, 30, 44)).collect::<Vec<_>>(),
        &lit_show,
    );
    c.push_str(&block(
        "F1",
        10.0,
        72.0,
        720.0,
        13.0,
        &std::iter::once("BDFLEFT column starts here".to_string()).chain(paragraph(&mut rng, 30, 44)).collect::<Vec<_>>(),
        &lit_show,
    ));
    doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {helv} 0 R >>")))?;
    out.push(
        Case::new("office-two-column", "office", "small", "Two columns whose stream order is right-then-left", doc.finish()?)
            .pages(1)
            .text(1, "BDFLEFT")
            .text(1, "BDFRIGHT"),
    );
    Ok(())
}

/// The PRD's "ordinary" class: about 100 pages, embedded fonts, images, links and bookmarks.
fn office_embedded_100(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    let mut doc = Doc::new("office-100p-embedded");
    let regular = embed_simple(&mut doc, &ctx.inter_regular, "Inter-Regular")?;
    let semibold = embed_simple(&mut doc, &ctx.inter_semibold, "Inter-SemiBold")?;
    let mut rng = Prng::new(0x1_0000);
    // One gradient picture reused by every tenth page.
    let (w, h) = (320u32, 200u32);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            rgb.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) * 255 / (w + h)) as u8]);
        }
    }
    let image = doc.add_image_rgb(w, h, &rgb, true)?;
    let mut page_ids = Vec::new();
    for n in 1..=100usize {
        let mut c = String::new();
        c.push_str(&block("F2", 18.0, 72.0, 720.0, 22.0, &[format!("BDFPAGE-{n:04} Chapter {}", (n - 1) / 10 + 1)], &lit_show));
        // Pages that carry a picture (drawn from y=140 to y=290) get fewer lines so the two never overlap.
        let body_lines = if n % 10 == 1 { 26 } else { 36 };
        c.push_str(&block("F1", 10.5, 72.0, 690.0, 14.0, &paragraph(&mut rng, body_lines, 88), &lit_show));
        let mut res = format!("/Font << /F1 {} 0 R /F2 {} 0 R >>", regular.id, semibold.id);
        let mut annots = Vec::new();
        if n % 10 == 1 {
            c.push_str("q 240 0 0 150 72 140 cm /Im1 Do Q\n");
            res.push_str(&format!(" /XObject << /Im1 {image} 0 R >>"));
        }
        // A footer link to the next page, and an external link on every page. Object numbers are
        // fixed by construction: this page adds two links, its content stream and the page, so the
        // next page object is seven after the first link (the last page links to itself).
        let base = doc.pdf.next_id();
        let target = if n < 100 { base + 7 } else { base + 3 };
        let link_internal =
            doc.pdf.add(format!("<< /Type /Annot /Subtype /Link /Rect [72 90 200 108] /Border [0 0 0] /Dest [{target} 0 R /Fit] >>").into_bytes());
        let link_uri = doc.pdf.add(
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [300 90 440 108] /Border [0 0 0] /A << /S /URI /URI {} >> >>",
                lit("https://example.invalid/bdf")
            )
            .into_bytes(),
        );
        annots.push(link_internal);
        annots.push(link_uri);
        c.push_str(&block("F1", 9.0, 72.0, 94.0, 10.0, &["Next page".to_string()], &lit_show));
        c.push_str(&block("F1", 9.0, 300.0, 94.0, 10.0, &["example.invalid/bdf".to_string()], &lit_show));
        let mut page = Page::letter().with_content(c.into_bytes()).with_resources(res);
        page.annots = annots;
        page_ids.push(doc.add_page(page)?);
    }
    // Bookmarks: ten chapters, each with children for pages 1, 5 and 9 of the chapter.
    let mut items = Vec::new();
    for ch in 0..10usize {
        let kids = [0usize, 4, 8]
            .iter()
            .map(|o| OutlineItem {
                title: format!("Chapter {} page {}", ch + 1, o + 1),
                page: page_ids.get(ch * 10 + o).copied().unwrap_or(0),
                kids: Vec::new(),
            })
            .collect();
        items.push(OutlineItem { title: format!("Chapter {}", ch + 1), page: page_ids.get(ch * 10).copied().unwrap_or(0), kids });
    }
    let titles = outline_titles(&items);
    let root = outlines(&mut doc, &items)?;
    doc.catalog = format!("/Outlines {root} 0 R /PageMode /UseOutlines");
    let mut case = Case::new(
        "office-100p-embedded",
        "office",
        "ordinary",
        "100 letter pages, embedded Inter (OFL), gradient pictures, internal and external links, two-level bookmarks",
        doc.finish()?,
    )
    .pages(100)
    .text(1, "BDFPAGE-0001")
    .text(50, "BDFPAGE-0050")
    .text(100, "BDFPAGE-0100");
    case.expect.outlines = titles;
    case.expect.links = Some(200);
    case.expect.sizes = vec![[612.0, 792.0]; 100];
    out.push(case);
    Ok(())
}

fn fonts(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    for (id, bytes, base, desc) in [
        ("font-inter-semibold", &ctx.inter_semibold, "Inter-SemiBold", "Embedded Inter SemiBold (OFL) as a simple TrueType font"),
        ("font-jetbrains-mono", &ctx.jetbrains, "JetBrainsMono-Regular", "Embedded JetBrains Mono (OFL), fixed pitch"),
        ("font-dancing-script", &ctx.dancing, "DancingScript-Regular", "Embedded Dancing Script (OFL), a connected script face"),
    ] {
        let mut doc = Doc::new(id);
        let f = embed_simple(&mut doc, bytes, base)?;
        let token = format!("BDFFONT-{}", id.to_uppercase());
        let mut rng = Prng::new(id.len() as u64 * 31);
        let mut lines = vec![token.clone()];
        lines.extend(paragraph(&mut rng, 24, 70));
        let c = block("F1", 14.0, 72.0, 720.0, 20.0, &lines, &lit_show);
        doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {} 0 R >>", f.id)))?;
        out.push(Case::new(id, "fonts", "small", desc, doc.finish()?).pages(1).text(1, &token));
    }
    // The same Inter face as a composite (Type0/Identity-H) font.
    let text = "BDFCID Composite font text, pangram: The quick brown fox jumps over the lazy dog 0123456789";
    let mut doc = Doc::new("font-cid-inter");
    let f = embed_cid(&mut doc, &ctx.inter_regular, "Inter-Regular", text)?;
    let c = format!("BT /F1 14 Tf 72 720 Td {} Tj ET\n", f.show(text));
    doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {} 0 R >>", f.id)))?;
    out.push(
        Case::new("font-cid-inter", "fonts", "small", "Inter as a Type0 Identity-H composite font with ToUnicode", doc.finish()?)
            .pages(1)
            .text(1, "BDFCID"),
    );
    Ok(())
}

fn unicode(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    // Latin-1 accents through a simple WinAnsi font.
    let mut doc = Doc::new("unicode-latin1");
    let f = embed_simple(&mut doc, &ctx.inter_regular, "Inter-Regular")?;
    let line = "BDFLATIN caf\u{e9} na\u{ef}ve Zo\u{eb} \u{c5}ngstr\u{f6}m \u{a9} \u{a3}5 \u{bd}";
    doc.add_page(
        Page::letter()
            .with_content(block("F1", 14.0, 72.0, 720.0, 20.0, &[line.to_string()], &lit_show).into_bytes())
            .with_resources(format!("/Font << /F1 {} 0 R >>", f.id)),
    )?;
    out.push(Case::new("unicode-latin1", "unicode", "small", "Latin-1 accents and symbols in a WinAnsi font", doc.finish()?).pages(1).text(1, line));

    for (id, text, desc, token) in [
        (
            "unicode-greek-cyrillic",
            "BDFGREEK \u{391}\u{3b8}\u{3ae}\u{3bd}\u{3b1} \u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430} \u{444}\u{438}\u{43b}\u{44c}\u{43c}",
            "Greek and Cyrillic through a CID font and ToUnicode",
            "BDFGREEK",
        ),
        ("unicode-combining", "BDFCOMB e\u{301} n\u{303} a\u{308} o\u{302} combining marks", "Base letters followed by combining accents", "BDFCOMB"),
        (
            "unicode-symbols",
            "BDFSYM \u{2192} \u{2264} \u{221e} \u{20ac} \u{2022} \u{2713} \u{2605}",
            "Arrows, math, currency and dingbat symbols",
            "BDFSYM",
        ),
    ] {
        let mut doc = Doc::new(id);
        let f = embed_cid(&mut doc, &ctx.inter_regular, "Inter-Regular", text)?;
        let c = format!("BT /F1 16 Tf 72 720 Td {} Tj ET\n", f.show(text));
        doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {} 0 R >>", f.id)))?;
        out.push(
            Case::new(id, "unicode", "small", desc, doc.finish()?)
                .pages(1)
                .text(1, token)
                .text(1, text.split_once(' ').map_or(text, |(_, rest)| rest)),
        );
    }
    Ok(())
}

//! Corpus family: document structure (outlines, destinations, page labels, attachments, layers,
//! tagged content).

use anyhow::{Result, bail};

use super::Ctx;
use super::model::Case;
use super::pdfgen::{Doc, Page, lit, utf16};
use super::text_cases::block;

/// A bookmark and its children.
pub struct OutlineItem {
    pub title: String,
    /// Object number of the destination page.
    pub page: u32,
    pub kids: Vec<OutlineItem>,
}

/// Titles in depth-first order, the order a reader lists them in.
pub fn outline_titles(items: &[OutlineItem]) -> Vec<String> {
    let mut out = Vec::new();
    for i in items {
        out.push(i.title.clone());
        out.extend(outline_titles(&i.kids));
    }
    out
}

/// Build an `/Outlines` tree and return the root's object number.
pub fn outlines(doc: &mut Doc, items: &[OutlineItem]) -> Result<u32> {
    if items.is_empty() {
        bail!("an outline needs at least one item");
    }
    let root = doc.pdf.reserve();
    let (first, last, count) = level(doc, items, root)?;
    doc.pdf.set(root, format!("<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {count} >>").into_bytes())?;
    Ok(root)
}

/// One sibling run. Returns `(first, last, number of visible descendants including the run)`.
fn level(doc: &mut Doc, items: &[OutlineItem], parent: u32) -> Result<(u32, u32, usize)> {
    let ids: Vec<u32> = items.iter().map(|_| doc.pdf.reserve()).collect();
    let mut total = 0usize;
    for (i, item) in items.iter().enumerate() {
        let id = ids[i];
        let mut d = format!("<< /Title {} /Parent {parent} 0 R /Dest [{} 0 R /Fit]", utf16(&item.title), item.page);
        if i > 0 {
            d.push_str(&format!(" /Prev {} 0 R", ids[i - 1]));
        }
        if let Some(next) = ids.get(i + 1) {
            d.push_str(&format!(" /Next {next} 0 R"));
        }
        total += 1;
        if !item.kids.is_empty() {
            let (f, l, c) = level(doc, &item.kids, id)?;
            d.push_str(&format!(" /First {f} 0 R /Last {l} 0 R /Count {c}"));
            total += c;
        }
        d.push_str(" >>");
        doc.pdf.set(id, d.into_bytes())?;
    }
    Ok((ids[0], ids[ids.len() - 1], total))
}

fn tree(depth: usize, breadth: usize, prefix: &str, pages: &[u32], n: &mut usize) -> Vec<OutlineItem> {
    (0..breadth)
        .map(|i| {
            *n += 1;
            let title = format!("{prefix}{}", i + 1);
            let kids = if depth > 1 { tree(depth - 1, breadth, &format!("{title}."), pages, n) } else { Vec::new() };
            OutlineItem { title, page: pages.get((*n) % pages.len().max(1)).copied().unwrap_or(0), kids }
        })
        .collect()
}

/// A page with a unique token and a heading; returns its object number.
fn token_page(doc: &mut Doc, font: u32, token: &str, extra: &str) -> Result<u32> {
    let c = block("F1", 16.0, 72.0, 720.0, 20.0, &[token.to_string()], &|s| lit(s)) + extra;
    doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {font} 0 R >>")))
}

pub fn generate(_ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    // Flat and deeply nested bookmarks.
    for (id, depth, breadth, desc) in [
        ("struct-outline-flat", 1usize, 10usize, "Ten flat bookmarks over ten pages"),
        ("struct-outline-nested", 4, 3, "Four levels of three bookmarks each (120 entries), expanded"),
    ] {
        let mut doc = Doc::new(id);
        let font = doc.std_font("Helvetica");
        let mut pages = Vec::new();
        for n in 1..=10 {
            pages.push(token_page(&mut doc, font, &format!("BDFOUTLINE-PAGE-{n:02}"), "")?);
        }
        let mut n = 0;
        let items = tree(depth, breadth, "Section ", &pages, &mut n);
        let titles = outline_titles(&items);
        let root = outlines(&mut doc, &items)?;
        doc.catalog = format!("/Outlines {root} 0 R /PageMode /UseOutlines");
        let mut case = Case::new(id, "structure", "small", desc, doc.finish()?).pages(10).text(1, "BDFOUTLINE-PAGE-01");
        case.expect.outlines = titles;
        out.push(case);
    }

    // Named destinations in both the legacy /Dests dictionary and the /Names tree.
    {
        let mut doc = Doc::new("struct-named-dests");
        let font = doc.std_font("Helvetica");
        let link_old = doc.pdf.add(b"<< /Type /Annot /Subtype /Link /Rect [72 600 300 630] /Border [0 0 0] /Dest /legacy3 >>".to_vec());
        let link_new = doc.pdf.add(b"<< /Type /Annot /Subtype /Link /Rect [72 540 300 570] /Border [0 0 0] /Dest (modern4) >>".to_vec());
        let c = block(
            "F1",
            14.0,
            72.0,
            720.0,
            20.0,
            &["BDFNAMED-PAGE-01".to_string(), "Link to legacy3 (below)".to_string(), "Link to modern4 (below)".to_string()],
            &|s| lit(s),
        );
        let mut p1 = Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {font} 0 R >>"));
        p1.annots = vec![link_old, link_new];
        let p1 = doc.add_page(p1)?;
        let _p2 = token_page(&mut doc, font, "BDFNAMED-PAGE-02", "")?;
        let p3 = token_page(&mut doc, font, "BDFNAMED-PAGE-03", "")?;
        let p4 = token_page(&mut doc, font, "BDFNAMED-PAGE-04", "")?;
        doc.catalog = format!(
            "/Dests << /legacy3 [{p3} 0 R /XYZ 72 700 0] >> /Names << /Dests << /Names [(modern4) [{p4} 0 R /Fit] (start) [{p1} 0 R /Fit]] >> >>"
        );
        let mut case = Case::new(
            "struct-named-dests",
            "structure",
            "small",
            "Links resolved through a legacy /Dests dictionary and a /Names destination tree",
            doc.finish()?,
        )
        .pages(4)
        .text(3, "BDFNAMED-PAGE-03");
        case.expect.links = Some(2);
        out.push(case);
    }

    // Page labels: lower roman, decimal, then a prefixed run.
    {
        let mut doc = Doc::new("struct-page-labels");
        let font = doc.std_font("Helvetica");
        for n in 1..=8 {
            token_page(&mut doc, font, &format!("BDFLABEL-PHYSICAL-{n}"), "")?;
        }
        doc.catalog = "/PageLabels << /Nums [0 << /S /r >> 4 << /S /D >> 7 << /S /D /P (App-) >>] >>".to_string();
        let mut case = Case::new("struct-page-labels", "structure", "small", "Page labels i-iv, 1-3, App-1", doc.finish()?).pages(8);
        case.expect.page_labels = ["i", "ii", "iii", "iv", "1", "2", "3", "App-1"].map(String::from).to_vec();
        out.push(case);
    }

    // Embedded files, including a nested PDF.
    {
        let mut nested = Doc::new("nested");
        let nf = nested.std_font("Helvetica");
        token_page(&mut nested, nf, "BDFNESTED-INNER", "")?;
        let nested_bytes = nested.finish()?;
        let mut doc = Doc::new("struct-attachments");
        let font = doc.std_font("Helvetica");
        token_page(&mut doc, font, "BDFATTACH-PAGE-01", "")?;
        let files: [(&str, &str, Vec<u8>); 3] = [
            ("notes.txt", "text#2Fplain", b"BDF attachment notes\n".to_vec()),
            ("data.csv", "text#2Fcsv", b"a,b,c\n1,2,3\n".to_vec()),
            ("nested.pdf", "application#2Fpdf", nested_bytes),
        ];
        let mut names = String::new();
        let mut attachments = Vec::new();
        for (name, subtype, data) in &files {
            let stream = doc.pdf.add_stream(&format!("/Type /EmbeddedFile /Subtype /{subtype} /Params << /Size {} >>", data.len()), data, true)?;
            let spec = doc.pdf.add(
                format!(
                    "<< /Type /Filespec /F {} /UF {} /Desc {} /EF << /F {stream} 0 R >> >>",
                    lit(name),
                    lit(name),
                    lit(&format!("BDF test file {name}"))
                )
                .into_bytes(),
            );
            names.push_str(&format!("{} {spec} 0 R ", lit(name)));
            attachments.push((*name).to_string());
        }
        doc.catalog = format!("/Names << /EmbeddedFiles << /Names [{names}] >> >>");
        let mut case = Case::new(
            "struct-attachments",
            "structure",
            "small",
            "Three embedded files (text, CSV, nested PDF) in the /EmbeddedFiles name tree",
            doc.finish()?,
        )
        .pages(1);
        case.expect.attachments = attachments;
        out.push(case);
    }

    // Optional content: one layer on, one off by default.
    {
        let mut doc = Doc::new("struct-layers");
        let font = doc.std_font("Helvetica");
        let on = doc.pdf.add(b"<< /Type /OCG /Name (Visible layer) >>".to_vec());
        let off = doc.pdf.add(b"<< /Type /OCG /Name (Hidden layer) >>".to_vec());
        let c =
            "/OC /ocOn BDC BT /F1 16 Tf 72 720 Td (BDFLAYER-VISIBLE) Tj ET EMC\n/OC /ocOff BDC BT /F1 16 Tf 72 680 Td (BDFLAYER-HIDDEN) Tj ET EMC\n";
        let res = format!("/Font << /F1 {font} 0 R >> /Properties << /ocOn {on} 0 R /ocOff {off} 0 R >>");
        doc.add_page(Page::letter().with_content(c.as_bytes().to_vec()).with_resources(res))?;
        doc.catalog = format!("/OCProperties << /OCGs [{on} 0 R {off} 0 R] /D << /Order [{on} 0 R {off} 0 R] /OFF [{off} 0 R] >> >>");
        out.push(
            Case::new("struct-layers", "structure", "small", "Two optional-content layers; the second is off by default", doc.finish()?)
                .pages(1)
                .text(1, "BDFLAYER-VISIBLE")
                .note("BDFLAYER-HIDDEN is in a layer that is off: it must not be drawn; whether text extraction includes it differs between readers"),
        );
    }

    // Tagged content with a structure tree.
    {
        let mut doc = Doc::new("struct-tagged");
        let font = doc.std_font("Helvetica");
        let c = "/P << /MCID 0 >> BDC BT /F1 16 Tf 72 720 Td (BDFTAG-FIRST paragraph) Tj ET EMC\n/P << /MCID 1 >> BDC BT /F1 12 Tf 72 690 Td (BDFTAG-SECOND paragraph) Tj ET EMC\n";
        // Reserve the structure objects first; the page is then the object after its content stream.
        let root = doc.pdf.reserve();
        let docel = doc.pdf.reserve();
        let p1 = doc.pdf.reserve();
        let p2 = doc.pdf.reserve();
        let page = doc.pdf.next_id() + 1;
        doc.pdf.set(p1, format!("<< /Type /StructElem /S /P /P {docel} 0 R /Pg {page} 0 R /K 0 >>").into_bytes())?;
        doc.pdf.set(p2, format!("<< /Type /StructElem /S /P /P {docel} 0 R /Pg {page} 0 R /K 1 >>").into_bytes())?;
        doc.pdf.set(docel, format!("<< /Type /StructElem /S /Document /P {root} 0 R /K [{p1} 0 R {p2} 0 R] >>").into_bytes())?;
        doc.pdf.set(root, format!("<< /Type /StructTreeRoot /K {docel} 0 R >>").into_bytes())?;
        let mut pg = Page::letter().with_content(c.as_bytes().to_vec()).with_resources(format!("/Font << /F1 {font} 0 R >>"));
        pg.extra = "/StructParents 0".to_string();
        let got = doc.add_page(pg)?;
        if got != page {
            bail!("tagged page expected object {page}, got {got}");
        }
        doc.catalog = format!("/MarkInfo << /Marked true >> /StructTreeRoot {root} 0 R /Lang (en-US)");
        out.push(
            Case::new("struct-tagged", "structure", "small", "A tagged page with a Document and two paragraph structure elements", doc.finish()?)
                .pages(1)
                .text(1, "BDFTAG-FIRST")
                .text(1, "BDFTAG-SECOND"),
        );
    }

    // Bookmarks that run actions (URI and GoToR-free named action) instead of plain destinations.
    {
        let mut doc = Doc::new("struct-outline-actions");
        let font = doc.std_font("Helvetica");
        token_page(&mut doc, font, "BDFACTION-PAGE-01", "")?;
        let p2 = token_page(&mut doc, font, "BDFACTION-PAGE-02", "")?;
        let root = doc.pdf.reserve();
        let a = doc.pdf.reserve();
        let b = doc.pdf.reserve();
        doc.pdf.set(
            a,
            format!("<< /Title {} /Parent {root} 0 R /Next {b} 0 R /A << /S /GoTo /D [{p2} 0 R /Fit] >> >>", utf16("Go to page two (GoTo action)"))
                .into_bytes(),
        )?;
        doc.pdf.set(
            b,
            format!(
                "<< /Title {} /Parent {root} 0 R /Prev {a} 0 R /A << /S /URI /URI {} >> >>",
                utf16("Open a web page (URI action)"),
                lit("https://example.invalid/bdf-bookmark")
            )
            .into_bytes(),
        )?;
        doc.pdf.set(root, format!("<< /Type /Outlines /First {a} 0 R /Last {b} 0 R /Count 2 >>").into_bytes())?;
        doc.catalog = format!("/Outlines {root} 0 R");
        let mut case = Case::new("struct-outline-actions", "structure", "small", "Bookmarks driven by GoTo and URI actions", doc.finish()?).pages(2);
        case.expect.outlines = vec!["Go to page two (GoTo action)".into(), "Open a web page (URI action)".into()];
        out.push(case);
    }
    Ok(())
}

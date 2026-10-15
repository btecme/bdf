//! Corpus families that attack or stress the reader: damaged files, hostile structures, size and
//! decode bombs, encrypted and signed documents, and a regression scenario page.
//!
//! Every case here is synthetic. Bombs are described by their expansion, not stored expanded: a
//! gigabyte of zeros compresses to about a megabyte.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::Ctx;
use super::model::{Case, Outcome, Tolerance};
use super::pdfgen::{Doc, Page, Pdf, embed_cid, lit};
use super::text_cases::block;

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len().max(1)).position(|w| w == needle)
}

fn replace_first(bytes: &mut Vec<u8>, from: &[u8], to: &[u8]) -> Result<()> {
    let at = find(bytes, from).with_context(|| format!("pattern {:?} not found", String::from_utf8_lossy(from)))?;
    bytes.splice(at..at + from.len(), to.iter().copied());
    Ok(())
}

/// Offset named by the last `startxref`.
fn startxref(bytes: &[u8]) -> Result<usize> {
    let at = bytes.windows(9).rposition(|w| w == b"startxref").context("no startxref")?;
    let tail = std::str::from_utf8(bytes.get(at + 9..).context("short file")?)?;
    Ok(tail.split_whitespace().next().context("no offset")?.parse()?)
}

/// Rewrite the xref table, trailer and startxref so every offset is right again, keeping the
/// objects exactly as they are. Used after a byte-level edit that changes the file's length, so the
/// only fault left in the file is the intended one and not a broken xref as well.
fn reindex(bytes: &[u8]) -> Result<Vec<u8>> {
    let xref_at = bytes.windows(6).rposition(|w| w == b"\nxref\n").context("xref table")? + 1;
    let trailer_at = find(bytes.get(xref_at..).context("xref")?, b"trailer\n").context("trailer")? + xref_at;
    let sx_at = find(bytes.get(trailer_at..).context("trailer")?, b"startxref").context("startxref")? + trailer_at;
    let trailer = bytes.get(trailer_at + 8..sx_at).context("trailer dictionary")?;
    let body = bytes.get(..xref_at).context("body")?;
    let mut offsets = std::collections::BTreeMap::new();
    let mut pos = 0usize;
    for line in body.split(|b| *b == b'\n') {
        if let Some(num) = line.strip_suffix(b" 0 obj").and_then(|n| std::str::from_utf8(n).ok()).and_then(|n| n.parse::<u32>().ok()) {
            offsets.insert(num, pos);
        }
        pos += line.len() + 1;
    }
    let size = offsets.keys().next_back().copied().unwrap_or(0) + 1;
    let mut out = body.to_vec();
    let new_xref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for id in 1..size {
        match offsets.get(&id) {
            Some(o) => out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    out.extend_from_slice(b"trailer\n");
    out.extend_from_slice(trailer);
    out.extend_from_slice(format!("startxref\n{new_xref}\n%%EOF\n").as_bytes());
    Ok(out)
}

// A small document of `pages` pages with tokens `BDFPAGE-0001`.. on each.
fn base(pages: usize, compress: bool) -> Result<Vec<u8>> {
    let mut doc = Doc::new("damage-base");
    let font = doc.std_font("Helvetica");
    for n in 1..=pages {
        let c = block("F1", 18.0, 72.0, 720.0, 22.0, &[format!("BDFPAGE-{n:04} damaged-input fixture")], &|s| lit(s));
        let mut p = Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {font} 0 R >>"));
        p.compress = compress;
        doc.add_page(p)?;
    }
    doc.finish()
}

fn robust(id: &str, family: &'static str, desc: &str, bytes: Vec<u8>) -> Case {
    Case::new(id, family, "hostile", desc, bytes).outcome(Outcome::Robust).tolerance(Tolerance::None)
}

fn damaged(out: &mut Vec<Case>) -> Result<()> {
    let plain = base(3, false)?;
    let zipped = base(3, true)?;
    let mut push = |id: &str, desc: &str, bytes: Vec<u8>, note: &str| {
        let mut c = robust(id, "damaged", desc, bytes).pages(3).text(0, "BDFPAGE-0001");
        c.expect.notes = note.to_string();
        out.push(c);
    };
    let xref_at = startxref(&plain)?;
    push(
        "damaged-truncated-xref",
        "Cut off right where the xref table starts: no xref, trailer or startxref",
        plain[..xref_at].to_vec(),
        "Readers that rebuild the table by scanning recover every page",
    );
    push(
        "damaged-truncated-stream",
        "Cut off 55% of the way in, in the middle of a page",
        plain[..plain.len() * 55 / 100].to_vec(),
        "Some pages are gone for good; a reader must not report all three",
    );
    let mut b = plain.clone();
    let at = b
        .windows(10)
        .rposition(|w| {
            w == b"startxref
"
        })
        .context("startxref")?
        + 10;
    let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    b[at..at + digits].fill(b'9');
    push(
        "damaged-startxref-wrong",
        "startxref names an offset that is far past the end of the file",
        b,
        "A bad startxref is the commonest corruption; readers rebuild",
    );
    let mut b = vec![b'X'; 2048];
    b.extend_from_slice(&plain);
    push(
        "damaged-junk-prefix-2k",
        "2 KiB of junk before the %PDF header, so every xref offset is off by 2048",
        b,
        "Readers differ on how much leading junk they accept (1,024 bytes is the usual limit)",
    );
    // Shift every in-use xref offset by 7.
    let mut b = plain.clone();
    let n = b.windows(5).position(|w| w == b"xref\n").context("xref")? + 5;
    let header_end = n + find(&b[n..], b"\n").context("xref header")? + 1;
    let mut i = header_end;
    while b.get(i + 17..i + 18) == Some(b"n") {
        let off: usize = std::str::from_utf8(&b[i..i + 10])?.parse()?;
        b[i..i + 10].copy_from_slice(format!("{:010}", off + 7).as_bytes());
        i += 20;
    }
    push("damaged-xref-offsets-shifted", "Every in-use xref offset is 7 bytes too large", b, "Each object lookup lands in the middle of a neighbour");
    let mut b = plain.clone();
    let at = find(&b, b"/Length ").context("/Length")? + 8;
    let width = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    b[at..at + width].copy_from_slice(&format!("{:0width$}", 3).into_bytes());
    push(
        "damaged-wrong-stream-length",
        "The first content stream declares /Length 3 but is longer",
        b,
        "Readers must look for endstream rather than trust the length",
    );
    let mut b = plain.clone();
    replace_first(&mut b, b"/Root ", b"/Roxt ")?;
    push("damaged-no-root", "The trailer has no /Root entry", b, "A reader must find the catalog by scanning for /Type /Catalog");
    let mut b = plain.clone();
    replace_first(&mut b, b"/Count 3", b"/Count 99")?;
    let b = reindex(&b)?;
    push("damaged-count-lies", "The page tree claims 99 pages and has 3", b, "Page counts must come from the leaves, not from /Count");
    let mut b = plain.clone();
    replace_first(&mut b, b"/Kids [", b"/Kids [9999 0 R ")?;
    let b = reindex(&b)?;
    push(
        "damaged-bad-kid",
        "A /Kids array starts with a reference to an object that does not exist",
        b,
        "One bad kid must not hide the pages after it",
    );
    let mut b = zipped.clone();
    let at = find(&b, b"stream\n").context("stream")? + 7;
    for x in b.iter_mut().skip(at + 12).take(16) {
        *x ^= 0xFF;
    }
    push(
        "damaged-corrupt-flate",
        "A page's compressed content stream is corrupted partway through",
        b,
        "A reader should draw what decoded before the damage or report the page as damaged",
    );
    let mut b = plain;
    let ids = b.windows(8).filter(|w| *w == b"endobj\n").count();
    while let Some(at) = find(&b, b"endobj\n") {
        b.drain(at..at + 7);
    }
    let b = reindex(&b)?;
    push(
        "damaged-missing-endobj",
        &format!("All {ids} endobj keywords are missing"),
        b,
        "Objects run into each other; a lenient reader splits them at the next `n 0 obj`",
    );
    Ok(())
}

/// A document whose objects are given verbatim, in order; `root` is the catalog's number.
fn manual(objects: &[&str], root: u32) -> Vec<u8> {
    let mut pdf = Pdf::new("1.7");
    for o in objects {
        pdf.add(o.as_bytes().to_vec());
    }
    pdf.finish(root, "")
}

fn hostile(out: &mut Vec<Case>) -> Result<()> {
    // Page-tree cycles.
    let b = manual(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        ],
        1,
    );
    out.push(
        robust("hostile-pagetree-self", "hostile", "A /Pages node that lists itself as a kid", b)
            .note("Must terminate; the one real page may still be shown"),
    );
    // Recursive form XObjects.
    for (id, desc, annotation) in [
        ("hostile-form-recursion", "A form XObject that draws itself", false),
        ("hostile-ap-recursion", "A widget whose appearance stream draws itself", true),
    ] {
        let mut doc = Doc::new(id);
        let form = doc.pdf.reserve();
        doc.pdf.set(
            form,
            Pdf::stream_body(
                &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /Fm {form} 0 R >> >>"),
                b"0 g 10 10 80 80 re f /Fm Do",
                false,
            )?,
        )?;
        let mut page = Page::sized(200.0, 200.0)
            .with_content(if annotation { b"0 g 0 0 10 10 re f".to_vec() } else { b"/Fm Do".to_vec() })
            .with_resources(format!("/XObject << /Fm {form} 0 R >>"));
        if annotation {
            let w = doc.pdf.add(
                format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (loop) /Rect [10 10 110 110] /F 4 /AP << /N {form} 0 R >> >>").into_bytes(),
            );
            page.annots = vec![w];
        }
        doc.add_page(page)?;
        out.push(robust(id, "hostile", desc, doc.finish()?).pages(1).note("Must terminate with a bounded recursion depth"));
    }

    // Deep operator and object nesting.
    {
        let mut doc = Doc::new("hostile-deep-q");
        let mut c = "q\n".repeat(2_000_000);
        c.push_str("0 g 50 50 100 100 re f\n");
        doc.add_page(Page::sized(200.0, 200.0).with_content(c.into_bytes()))?;
        out.push(
            robust("hostile-deep-q", "hostile", "Two million unbalanced q operators before one filled rectangle", doc.finish()?)
                .pages(1)
                .note("The graphics-state stack must be bounded"),
        );
    }
    {
        let mut doc = Doc::new("hostile-deep-array");
        let depth = 200_000;
        let deep = doc.pdf.add(format!("{}{}", "[".repeat(depth), "]".repeat(depth)).into_bytes());
        doc.add_page(Page::sized(200.0, 200.0).with_content(b"0 g 10 10 50 50 re f".to_vec()))?;
        doc.catalog = format!("/BdfDeep {deep} 0 R");
        out.push(
            robust("hostile-deep-array", "hostile", "An object holding 200,000 nested arrays, reachable from the catalog", doc.finish()?)
                .pages(1)
                .note("Parsing must not recurse to the stack limit"),
        );
    }

    // Decode bombs.
    {
        let mut doc = Doc::new("hostile-flate-bomb-image");
        let (w, h) = (32_768u64, 32_768u64);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
        let zeros = vec![0u8; 1 << 20];
        for _ in 0..((w * h) >> 20) {
            enc.write_all(&zeros)?;
        }
        let packed = enc.finish()?;
        let dict = format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode");
        let im = doc.pdf.add(Pdf::stream_body(&dict, &packed, false)?);
        doc.add_page(
            Page::sized(200.0, 200.0)
                .with_content(b"q 180 0 0 180 10 10 cm /Im1 Do Q".to_vec())
                .with_resources(format!("/XObject << /Im1 {im} 0 R >>")),
        )?;
        out.push(
            robust(
                "hostile-flate-bomb-image",
                "hostile",
                &format!("A {w}x{h} grey image (1 GiB decoded) stored as about {} KiB of zlib", packed.len() / 1024),
                doc.finish()?,
            )
            .pages(1)
            .note("Decoding must stay inside a memory budget; refusing the image or the page is acceptable"),
        );
    }
    {
        // 512 MiB of zeros, deflated twice and hex-encoded: [/ASCIIHexDecode /FlateDecode /FlateDecode].
        let deflate = |data: &[u8]| -> Result<Vec<u8>> {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
            e.write_all(data)?;
            Ok(e.finish()?)
        };
        let mut inner = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
        let zeros = vec![0u8; 1 << 20];
        for _ in 0..512 {
            inner.write_all(&zeros)?;
        }
        let once = inner.finish()?;
        let twice = deflate(&once)?;
        let hex: String = twice.iter().map(|b| format!("{b:02x}")).collect::<String>() + ">";
        let mut doc = Doc::new("hostile-filter-chain-bomb");
        let content = doc.pdf.add(Pdf::stream_body("/Filter [/ASCIIHexDecode /FlateDecode /FlateDecode]", hex.as_bytes(), false)?);
        let page = format!("<< /Type /Page /Parent @PARENT@ 0 R /MediaBox [0 0 200 200] /Contents {content} 0 R /Resources << >> >>");
        let id = doc.pdf.add(page.into_bytes());
        doc.kids.push(id);
        out.push(
            robust(
                "hostile-filter-chain-bomb",
                "hostile",
                &format!("A content stream that decodes to 512 MiB through three filters, stored in {} bytes", hex.len()),
                doc.finish()?,
            )
            .pages(1)
            .note("Every filter stage must be bounded, not just the last"),
        );
    }

    // Absurd numbers.
    {
        let mut doc = Doc::new("hostile-huge-mediabox");
        doc.add_page(Page::sized(1.0e7, 1.0e7).with_content(b"0 g 100 100 1000 1000 re f".to_vec()))?;
        out.push(
            robust("hostile-huge-mediabox", "hostile", "A MediaBox of 10,000,000 x 10,000,000 points", doc.finish()?)
                .pages(1)
                .note("Rendering at any DPI must refuse or clamp, not allocate a raster of that size"),
        );
    }
    {
        let mut b = base(1, false)?;
        let at = find(&b, b"/Size ").context("/Size")? + 6;
        let w = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
        b.splice(at..at + w, b"2147483647".iter().copied());
        out.push(
            robust("hostile-xref-size", "hostile", "The trailer claims /Size 2147483647 objects", b)
                .pages(1)
                .note("Table sizes must not be taken from /Size"),
        );
    }
    {
        let mut b = base(1, false)?;
        let x = startxref(&b)?;
        replace_first(&mut b, b"trailer\n<< ", format!("trailer\n<< /Prev {x} ").as_bytes())?;
        out.push(
            robust("hostile-xref-prev-loop", "hostile", "The newest xref section lists itself as its /Prev", b)
                .pages(1)
                .note("The /Prev chain needs a seen-set"),
        );
    }
    {
        let mut doc = Doc::new("hostile-length-selfref");
        let content = doc.pdf.reserve();
        doc.pdf.set(content, format!("<< /Length {content} 0 R >>\nstream\n0 g 10 10 50 50 re f\nendstream").into_bytes())?;
        let page = format!("<< /Type /Page /Parent @PARENT@ 0 R /MediaBox [0 0 200 200] /Contents {content} 0 R >>");
        let id = doc.pdf.add(page.into_bytes());
        doc.kids.push(id);
        out.push(
            robust("hostile-length-selfref", "hostile", "A stream whose /Length is an indirect reference to the stream itself", doc.finish()?)
                .pages(1)
                .note("Resolving the length must not loop"),
        );
    }

    // Admission limits from spec section 7.
    {
        let mut doc = Doc::new("hostile-pages-2001");
        for _ in 0..2001 {
            doc.add_page(Page::sized(100.0, 100.0).with_content(b"0 g 10 10 10 10 re f".to_vec()))?;
        }
        let mut c = Case::new(
            "hostile-pages-2001",
            "hostile",
            "hostile",
            "2,001 tiny pages, one over the proposed 2,000-page admission limit",
            doc.finish()?,
        )
        .outcome(Outcome::Refused)
        .tolerance(Tolerance::None)
        .pages(2001);
        c.expect.notes =
            "Spec section 7 proposes refusing more than 2,000 pages after bounded inspection. The current build's behaviour is measured, not assumed"
                .to_string();
        out.push(c);
    }
    {
        let mut doc = Doc::new("hostile-size-251mib");
        let pad = vec![b' '; 251 * 1_048_576];
        doc.pdf.add_stream("/Type /BdfPadding", &pad, false)?;
        doc.add_page(Page::sized(200.0, 200.0).with_content(b"0 g 10 10 50 50 re f".to_vec()))?;
        let mut c = Case::new(
            "hostile-size-251mib",
            "hostile",
            "hostile",
            "A valid one-page file padded to 251 MiB with an unused stream, one MiB over the proposed 250 MiB limit",
            doc.finish()?,
        )
        .outcome(Outcome::Refused)
        .tolerance(Tolerance::None)
        .pages(1);
        c.expect.notes = "Spec section 7 proposes refusing input above 250 MiB before reading it whole (finding F10). The current build's behaviour is measured, not assumed".to_string();
        out.push(c);
    }
    Ok(())
}

/// Run `mutool clean` with `args`, from `input` bytes to output bytes.
fn mutool_clean(mutool: &Path, work: &Path, name: &str, input: &[u8], args: &[&str]) -> Result<Vec<u8>> {
    std::fs::create_dir_all(work)?;
    let src = work.join(format!("{name}-in.pdf"));
    let dst = work.join(format!("{name}-out.pdf"));
    std::fs::write(&src, input)?;
    let status =
        Command::new(mutool).arg("clean").args(args).arg(&src).arg(&dst).output().with_context(|| format!("running {}", mutool.display()))?;
    if !status.status.success() {
        bail!("mutool clean {args:?} failed: {}", String::from_utf8_lossy(&status.stderr));
    }
    let bytes = std::fs::read(&dst)?;
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    Ok(bytes)
}

fn derived(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    let Some(mutool) = &ctx.mutool else {
        eprintln!("bdf-corpus: no --mutool: skipping the 7 derived cases (encrypted and subset-font)");
        return Ok(());
    };
    let work = crate::gates::target_dir().join("bdf-corpus-tmp");
    let plain = base(3, true)?;
    let recipe = "derived from damage-base by `mutool clean`, which salts and encrypts with its own random values: the bytes differ per run, the content does not";
    for (id, flags, password, desc) in [
        ("enc-rc4-40-user", vec!["-E", "rc4-40", "-U", "user40", "-O", "owner40"], Some("user40"), "RC4 40-bit, user password required"),
        ("enc-rc4-128-user", vec!["-E", "rc4-128", "-U", "user128", "-O", "owner128"], Some("user128"), "RC4 128-bit, user password required"),
        ("enc-aes-128-user", vec!["-E", "aes-128", "-U", "useraes", "-O", "owneraes"], Some("useraes"), "AES 128-bit, user password required"),
        (
            "enc-aes-256-user",
            vec!["-E", "aes-256", "-U", "user256", "-O", "owner256"],
            Some("user256"),
            "AES 256-bit (revision 6), user password required",
        ),
        (
            "enc-aes-256-unicode-user",
            vec!["-E", "aes-256", "-U", "p\u{e4}ssw\u{f6}rd", "-O", "owner256"],
            Some("p\u{e4}ssw\u{f6}rd"),
            "AES 256-bit with a non-ASCII user password",
        ),
        (
            "enc-aes-128-no-permissions",
            vec!["-E", "aes-128", "-O", "ownerperm", "-P", "-3904"],
            None,
            "AES 128-bit, empty user password, every permission denied",
        ),
    ] {
        let bytes = mutool_clean(mutool, &work, id, &plain, &flags)?;
        let mut c = Case::new(id, "encrypted", "small", desc, bytes)
            .outcome(if password.is_some() { Outcome::Password } else { Outcome::Opens })
            .tolerance(Tolerance::Text)
            .pages(3)
            .text(1, "BDFPAGE-0001");
        c.deterministic = false;
        c.provenance = format!("synthetic: {recipe}; command `mutool clean {}`", flags.join(" "));
        c.expect.password = password.map(str::to_string);
        if password.is_none() {
            // Copying and extraction are denied, so a reader that honours permissions must withhold
            // the text; MuPDF's text output ignores them. Asserting the token would assert a violation.
            c.expect.text.clear();
            c.expect.notes = "Opens without a password. Printing, copying and editing are denied by the permission flags and the owner password lifts them, so a conforming reader withholds the extracted text".to_string();
        }
        out.push(c);
    }
    // A real subset font: the embedded Inter face reduced to the glyphs the page uses.
    let mut doc = Doc::new("font-subset-inter");
    let text = "BDFSUBSET Subset font text";
    let f = embed_cid(&mut doc, &ctx.inter_regular, "Inter-Regular", text)?;
    doc.add_page(
        Page::letter()
            .with_content(format!("BT /F1 16 Tf 72 720 Td {} Tj ET\n", f.show(text)).into_bytes())
            .with_resources(format!("/Font << /F1 {} 0 R >>", f.id)),
    )?;
    let bytes = mutool_clean(mutool, &work, "font-subset-inter", &doc.finish()?, &["-S", "-z"])?;
    let mut c = Case::new("font-subset-inter", "fonts", "small", "Inter reduced to a real subset (ABCDEF+ prefix) by mutool clean -S", bytes)
        .pages(1)
        .text(1, "BDFSUBSET");
    c.deterministic = false;
    c.provenance =
        "synthetic: font-cid-inter-style page subset by `mutool clean -S -z` (EXPERIMENTAL in MuPDF); not byte-reproducible without the same mutool"
            .to_string();
    out.push(c);
    Ok(())
}

fn signed_and_scenarios(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    for (file, id, desc) in [
        ("openssl-signed.pdf", "signed-openssl", "A CMS-signed PDF produced with OpenSSL (in-repo fixture of crates/sign)"),
        ("x509-rsa-sha1.pdf", "signed-x509-rsa-sha1", "A PDF signed with an RSA-SHA1 certificate (in-repo fixture of crates/sign)"),
    ] {
        let path = ctx.root.join("crates").join("sign").join("tests").join("data").join(file);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut c = Case::new(id, "signed", "small", desc, bytes).outcome(Outcome::Opens).tolerance(Tolerance::Text);
        c.provenance = format!("in-repo test fixture crates/sign/tests/data/{file}, committed upstream and already attributed there");
        c.expect.notes = "A signed file must be detected and reported as signed; its page content is not asserted here".to_string();
        out.push(c);
    }
    // Issue #1368 (text-drag data loss): a page with three separately placed text blocks.
    let mut doc = Doc::new("workflow-1368-text-blocks");
    let font = doc.std_font("Helvetica");
    let mut c = String::new();
    for (i, t) in ["BDF1368-ALPHA", "BDF1368-BETA", "BDF1368-GAMMA"].iter().enumerate() {
        c.push_str(&block(
            "F1",
            16.0,
            72.0 + 40.0 * i as f64,
            700.0 - 120.0 * i as f64,
            18.0,
            &[format!("{t} drag me"), "second line of the same block".to_string()],
            &|s| lit(s),
        ));
    }
    doc.add_page(Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {font} 0 R >>")))?;
    let mut case = Case::new(
        "workflow-1368-text-blocks",
        "workflows",
        "small",
        "Three text blocks for the text-drag round trip behind upstream issue #1368",
        doc.finish()?,
    )
    .pages(1)
    .text(1, "BDF1368-ALPHA")
    .text(1, "BDF1368-BETA")
    .text(1, "BDF1368-GAMMA");
    case.expect.notes = "Workflow, run in BUILD-012: drag each block, save, reopen; every token must survive and the drag must not delete a neighbour (issue #1368, fixed in #1381)".to_string();
    out.push(case);
    Ok(())
}

pub fn generate(ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    damaged(out)?;
    hostile(out)?;
    derived(ctx, out)?;
    signed_and_scenarios(ctx, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reindex_is_the_identity_on_a_clean_file() {
        for compress in [false, true] {
            let clean = base(3, compress).unwrap();
            assert_eq!(reindex(&clean).unwrap(), clean, "compress={compress}");
        }
    }

    #[test]
    fn reindex_repairs_a_length_changing_edit() {
        let mut b = base(3, false).unwrap();
        replace_first(&mut b, b"/Count 3", b"/Count 99").unwrap();
        let fixed = reindex(&b).unwrap();
        let at = startxref(&fixed).unwrap();
        assert!(fixed[at..].starts_with(b"xref\n"));
        // Every in-use entry now points at an "N 0 obj" header.
        let text = String::from_utf8_lossy(&fixed[at..]).into_owned();
        let entries: Vec<&str> = text.lines().skip(3).take_while(|l| !l.starts_with("trailer")).collect();
        assert!(entries.len() >= 9, "{entries:?}");
        for (i, e) in entries.iter().enumerate() {
            let off: usize = e[..10].parse().unwrap();
            assert!(fixed[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "entry {i} at {off}");
        }
    }

    #[test]
    fn startxref_reads_the_last_marker() {
        let b = base(1, false).unwrap();
        let at = startxref(&b).unwrap();
        assert!(b[at..].starts_with(b"xref\n"));
    }
}

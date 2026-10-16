//! A deliberately small PDF writer for the BUILD-002 corpus.
//!
//! It shares no code with `pdfcraft-cos`, `lopdf` or the engine's writer, so a fixture's bytes do
//! not come from the parser or serializer under test (spec §9: "do not replace independent
//! document evidence with mocks of the same writer/parser"). Output is fully deterministic: no
//! clock, no randomness, fixed compression level, so a fixture's SHA-256 is a function of the
//! generator source and the pinned font files alone.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider as _;
use skrifa::{FontRef, MetadataProvider as _};

/// A PDF literal string `(…)` for ASCII or Latin-1 text.
pub fn lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('(');
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (' '..='~').contains(&c) => out.push(c),
            c => {
                let _ = write!(out, "\\{:03o}", u32::from(c) & 0xFF);
            }
        }
    }
    out.push(')');
    out
}

/// A UTF-16BE text string `<FEFF…>` for any Unicode text (titles, field values, outline names).
pub fn utf16(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        let _ = write!(out, "{u:04X}");
    }
    out.push('>');
    out
}

/// Format a number compactly and deterministically.
pub fn num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        // Integral: print without a fraction. The cast cannot truncate: |v| < 1e15 fits in i64.
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// zlib-compress at a fixed level (deterministic for a given flate2 version).
pub fn flate(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

/// The object store and serializer.
pub struct Pdf {
    version: &'static str,
    objects: Vec<Vec<u8>>,
}

impl Pdf {
    pub fn new(version: &'static str) -> Self {
        Self { version, objects: Vec::new() }
    }

    /// Reserve an object number to be filled later with [`Pdf::set`].
    pub fn reserve(&mut self) -> u32 {
        self.objects.push(b"null".to_vec());
        self.objects.len() as u32
    }

    pub fn set(&mut self, id: u32, body: impl Into<Vec<u8>>) -> Result<()> {
        let slot = usize::try_from(id).ok().and_then(|i| i.checked_sub(1)).and_then(|i| self.objects.get_mut(i));
        *slot.ok_or_else(|| anyhow!("object {id} was never reserved"))? = body.into();
        Ok(())
    }

    pub fn add(&mut self, body: impl Into<Vec<u8>>) -> u32 {
        self.objects.push(body.into());
        self.objects.len() as u32
    }

    /// A stream object. `dict` holds the extra dictionary entries (no `/Length`, no `/Filter`).
    pub fn stream_body(dict: &str, data: &[u8], compress: bool) -> Result<Vec<u8>> {
        let (data, filter) = if compress { (flate(data)?, " /Filter /FlateDecode") } else { (data.to_vec(), "") };
        let mut body = format!("<< {dict}{filter} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        Ok(body)
    }

    pub fn add_stream(&mut self, dict: &str, data: &[u8], compress: bool) -> Result<u32> {
        Ok(self.add(Self::stream_body(dict, data, compress)?))
    }

    /// Number of the next object [`Pdf::add`] would create.
    pub fn next_id(&self) -> u32 {
        self.objects.len() as u32 + 1
    }

    /// Serialize with a classic cross-reference table. `trailer` holds extra trailer entries.
    pub fn finish(&self, root: u32, trailer: &str) -> Vec<u8> {
        let mut out = format!("%PDF-{}\n", self.version).into_bytes();
        // A comment of four bytes above 127 marks the file as binary.
        out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        let mut offsets = Vec::with_capacity(self.objects.len());
        let mut hasher = Sha256::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
            hasher.update(body);
        }
        let id = hasher.finalize();
        let id_hex: String = id.iter().take(16).map(|b| format!("{b:02x}")).collect();
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes());
        for off in &offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R /ID [<{id_hex}> <{id_hex}>] {trailer} >>\nstartxref\n{xref_at}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

/// One page to add to a [`Doc`].
#[derive(Clone)]
pub struct Page {
    pub media: [f64; 4],
    pub rotate: Option<i32>,
    pub crop: Option<[f64; 4]>,
    pub content: Vec<u8>,
    /// Inner text of the `/Resources << … >>` dictionary.
    pub resources: String,
    pub annots: Vec<u32>,
    /// Extra page-dictionary entries.
    pub extra: String,
    pub compress: bool,
}

impl Page {
    pub fn letter() -> Self {
        Self::sized(612.0, 792.0)
    }

    pub fn sized(w: f64, h: f64) -> Self {
        Self {
            media: [0.0, 0.0, w, h],
            rotate: None,
            crop: None,
            content: Vec::new(),
            resources: String::new(),
            annots: Vec::new(),
            extra: String::new(),
            compress: true,
        }
    }

    pub fn with_content(mut self, content: impl Into<Vec<u8>>) -> Self {
        self.content = content.into();
        self
    }

    pub fn with_resources(mut self, resources: impl Into<String>) -> Self {
        self.resources = resources.into();
        self
    }
}

/// A document under construction: a page tree, a catalog and an info dictionary.
pub struct Doc {
    pub pdf: Pdf,
    pub pages_id: u32,
    pub kids: Vec<u32>,
    /// Extra entries for the page-tree root (e.g. inherited `/Rotate`, `/MediaBox`, `/Resources`).
    pub pages_extra: String,
    /// Extra catalog entries (`/Outlines`, `/AcroForm`, `/Names`, …).
    pub catalog: String,
    pub title: String,
    /// Group the leaves into intermediate `/Pages` nodes of at most this many kids (0 = flat tree).
    pub group: usize,
    pub trailer: String,
}

impl Doc {
    pub fn new(title: &str) -> Self {
        let mut pdf = Pdf::new("1.7");
        let pages_id = pdf.reserve();
        Self {
            pdf,
            pages_id,
            kids: Vec::new(),
            pages_extra: String::new(),
            catalog: String::new(),
            title: title.to_string(),
            group: 0,
            trailer: String::new(),
        }
    }

    /// Add a page; its `/Parent` is patched when the tree is built.
    pub fn add_page(&mut self, p: Page) -> Result<u32> {
        let content = self.pdf.add_stream("", &p.content, p.compress)?;
        let mut d = String::from("<< /Type /Page /Parent @PARENT@ 0 R");
        let _ = write!(d, " /MediaBox [{}]", p.media.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" "));
        if let Some(c) = p.crop {
            let _ = write!(d, " /CropBox [{}]", c.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" "));
        }
        if let Some(r) = p.rotate {
            let _ = write!(d, " /Rotate {r}");
        }
        let _ = write!(d, " /Contents {content} 0 R /Resources << {} >>", p.resources);
        if !p.annots.is_empty() {
            let _ = write!(d, " /Annots [{}]", p.annots.iter().map(|a| format!("{a} 0 R")).collect::<Vec<_>>().join(" "));
        }
        let _ = write!(d, " {} >>", p.extra);
        let id = self.pdf.add(d.into_bytes());
        self.kids.push(id);
        Ok(id)
    }

    /// Standard-14 font, WinAnsi encoded.
    pub fn std_font(&mut self, base: &str) -> u32 {
        self.pdf.add(format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>").into_bytes())
    }

    pub fn add_image_rgb(&mut self, w: u32, h: u32, rgb: &[u8], compress: bool) -> Result<u32> {
        if usize::try_from(u64::from(w) * u64::from(h) * 3)? != rgb.len() {
            bail!("RGB image {w}x{h} needs {} bytes, got {}", u64::from(w) * u64::from(h) * 3, rgb.len());
        }
        self.pdf.add_stream(
            &format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8"),
            rgb,
            compress,
        )
    }

    pub fn add_image_gray(&mut self, w: u32, h: u32, gray: &[u8], compress: bool) -> Result<u32> {
        if usize::try_from(u64::from(w) * u64::from(h))? != gray.len() {
            bail!("gray image {w}x{h} needs {} bytes, got {}", u64::from(w) * u64::from(h), gray.len());
        }
        self.pdf.add_stream(
            &format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"),
            gray,
            compress,
        )
    }

    /// Build the page tree and catalog, then serialize.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        let leaves = std::mem::take(&mut self.kids);
        let root_kids = if self.group > 1 && leaves.len() > self.group {
            let mut nodes = Vec::new();
            for chunk in leaves.chunks(self.group) {
                let node = self.pdf.reserve();
                for leaf in chunk {
                    self.patch_parent(*leaf, node)?;
                }
                let kids = chunk.iter().map(|k| format!("{k} 0 R")).collect::<Vec<_>>().join(" ");
                self.pdf.set(node, format!("<< /Type /Pages /Parent {} 0 R /Kids [{kids}] /Count {} >>", self.pages_id, chunk.len()).into_bytes())?;
                nodes.push(node);
            }
            nodes
        } else {
            for leaf in &leaves {
                self.patch_parent(*leaf, self.pages_id)?;
            }
            leaves.clone()
        };
        let kids = root_kids.iter().map(|k| format!("{k} 0 R")).collect::<Vec<_>>().join(" ");
        self.pdf.set(self.pages_id, format!("<< /Type /Pages /Kids [{kids}] /Count {} {} >>", leaves.len(), self.pages_extra).into_bytes())?;
        let catalog = self.pdf.add(format!("<< /Type /Catalog /Pages {} 0 R {} >>", self.pages_id, self.catalog).into_bytes());
        let info = self
            .pdf
            .add(format!("<< /Title {} /Producer (bdf-corpus generator) /CreationDate (D:20260101000000Z) >>", lit(&self.title)).into_bytes());
        Ok(self.pdf.finish(catalog, &format!("/Info {info} 0 R {}", self.trailer)))
    }

    fn patch_parent(&mut self, page: u32, parent: u32) -> Result<()> {
        let idx = usize::try_from(page)?.checked_sub(1).context("page object number 0")?;
        let body = self.pdf.objects.get_mut(idx).ok_or_else(|| anyhow!("page object {page} missing"))?;
        let text = String::from_utf8(std::mem::take(body))?;
        *body = text.replace("@PARENT@", &parent.to_string()).into_bytes();
        Ok(())
    }
}

/// Glyph data read from a TrueType file with `skrifa`.
pub struct Face<'a> {
    data: &'a [u8],
    font: FontRef<'a>,
    pub upem: f64,
}

impl<'a> Face<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let font = FontRef::new(data).map_err(|e| anyhow!("font parse: {e}"))?;
        let upem = f64::from(font.head().map_err(|e| anyhow!("head table: {e}"))?.units_per_em());
        if upem <= 0.0 {
            bail!("font has zero units per em");
        }
        Ok(Self { data, font, upem })
    }

    pub fn glyph(&self, c: char) -> Option<u16> {
        self.font.charmap().map(c).and_then(|g| u16::try_from(g.to_u32()).ok())
    }

    /// Advance width in 1000-unit text space.
    pub fn advance(&self, gid: u16) -> f64 {
        let gm = self.font.glyph_metrics(Size::unscaled(), LocationRef::default());
        gm.advance_width(skrifa::GlyphId::new(u32::from(gid))).map_or(0.0, |w| f64::from(w) * 1000.0 / self.upem)
    }

    fn descriptor_numbers(&self) -> (f64, f64, f64, [f64; 4]) {
        let m = self.font.metrics(Size::unscaled(), LocationRef::default());
        let s = 1000.0 / self.upem;
        let bb = m.bounds.map_or([-200.0, -300.0, 1200.0, 1000.0], |b| {
            [f64::from(b.x_min) * s, f64::from(b.y_min) * s, f64::from(b.x_max) * s, f64::from(b.y_max) * s]
        });
        (f64::from(m.ascent) * s, f64::from(m.descent) * s, m.cap_height.map_or(700.0, |c| f64::from(c) * s), bb)
    }
}

/// An embedded simple (single-byte, WinAnsi) TrueType font.
pub struct SimpleFont {
    pub id: u32,
}

/// Embed a whole TrueType file as a simple font; text must stay within printable ASCII and Latin-1.
pub fn embed_simple(doc: &mut Doc, ttf: &[u8], base: &str) -> Result<SimpleFont> {
    let face = Face::new(ttf)?;
    let widths: Vec<String> = (32u32..=255)
        .map(|code| {
            let c = if (128..160).contains(&code) { None } else { char::from_u32(code) };
            num(c.and_then(|c| face.glyph(c)).map_or(0.0, |g| face.advance(g)).round())
        })
        .collect();
    let (ascent, descent, cap, bb) = face.descriptor_numbers();
    let file = doc.pdf.add_stream(&format!("/Length1 {}", face.data.len()), face.data, true)?;
    let desc = doc.pdf.add(
        format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
            num(bb[0].round()),
            num(bb[1].round()),
            num(bb[2].round()),
            num(bb[3].round()),
            num(ascent.round()),
            num(descent.round()),
            num(cap.round())
        )
        .into_bytes(),
    );
    let id = doc.pdf.add(
        format!(
            "<< /Type /Font /Subtype /TrueType /BaseFont /{base} /FirstChar 32 /LastChar 255 /Widths [{}] /Encoding /WinAnsiEncoding /FontDescriptor {desc} 0 R >>",
            widths.join(" ")
        )
        .into_bytes(),
    );
    Ok(SimpleFont { id })
}

/// An embedded Type0/CIDFontType2 (Identity-H) font that can show any Unicode text the face covers.
pub struct CidFont {
    pub id: u32,
    glyphs: BTreeMap<char, u16>,
}

impl CidFont {
    /// Hex string operand for `Tj`: two bytes per glyph. Characters the face lacks become `.notdef`.
    pub fn show(&self, s: &str) -> String {
        let mut out = String::from("<");
        for c in s.chars() {
            let _ = write!(out, "{:04X}", self.glyphs.get(&c).copied().unwrap_or(0));
        }
        out.push('>');
        out
    }
}

/// Embed `ttf` as a CID font covering exactly the characters in `text` (the `/W` array and the
/// `ToUnicode` map are limited to them, so extraction is checkable against `text`).
pub fn embed_cid(doc: &mut Doc, ttf: &[u8], base: &str, text: &str) -> Result<CidFont> {
    let face = Face::new(ttf)?;
    let mut glyphs = BTreeMap::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if let Some(g) = face.glyph(c) {
            glyphs.insert(c, g);
        } else {
            bail!("{base} has no glyph for U+{:04X}", u32::from(c));
        }
    }
    let mut by_gid: BTreeMap<u16, char> = BTreeMap::new();
    for (c, g) in &glyphs {
        by_gid.entry(*g).or_insert(*c);
    }
    let w: Vec<String> = by_gid.keys().map(|g| format!("{g} [{}]", num(face.advance(*g).round()))).collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /BDF-ToUnicode def /CMapType 2 def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for chunk in by_gid.iter().collect::<Vec<_>>().chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (g, c) in chunk {
            let mut hex = String::new();
            for u in c.encode_utf16(&mut [0u16; 2]).iter() {
                let _ = write!(hex, "{u:04X}");
            }
            let _ = writeln!(cmap, "<{g:04X}> <{hex}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    let tounicode = doc.pdf.add_stream("", cmap.as_bytes(), true)?;
    let (ascent, descent, cap, bb) = face.descriptor_numbers();
    let file = doc.pdf.add_stream(&format!("/Length1 {}", face.data.len()), face.data, true)?;
    let desc = doc.pdf.add(
        format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
            num(bb[0].round()),
            num(bb[1].round()),
            num(bb[2].round()),
            num(bb[3].round()),
            num(ascent.round()),
            num(descent.round()),
            num(cap.round())
        )
        .into_bytes(),
    );
    let cid = doc.pdf.add(
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{}] >>",
            w.join(" ")
        )
        .into_bytes(),
    );
    let id = doc.pdf.add(
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {tounicode} 0 R >>")
            .into_bytes(),
    );
    Ok(CidFont { id, glyphs })
}

/// A deterministic byte generator for image content (xorshift64*), so "scans" have noise that does
/// not compress away.
pub struct Prng(u64);

impl Prng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            for (d, s) in chunk.iter_mut().zip(v) {
                *d = s;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_document_is_deterministic_and_has_a_valid_xref() {
        let build = || {
            let mut doc = Doc::new("t");
            let f = doc.std_font("Helvetica");
            doc.add_page(
                Page::letter().with_content(b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET".to_vec()).with_resources(format!("/Font << /F1 {f} 0 R >>")),
            )
            .unwrap();
            doc.finish().unwrap()
        };
        let (a, b) = (build(), build());
        assert_eq!(a, b);
        let text = String::from_utf8_lossy(&a);
        let at: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(a[at..].starts_with(b"xref\n"));
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(lit("a(b)\\"), "(a\\(b\\)\\\\)");
        assert_eq!(utf16("A\u{3b1}"), "<FEFF0041 03B1>".replace(' ', ""));
    }

    #[test]
    fn unreserved_objects_are_an_error() {
        let mut pdf = Pdf::new("1.7");
        assert!(pdf.set(5, b"null".to_vec()).is_err());
        assert!(pdf.set(0, b"null".to_vec()).is_err());
    }
}

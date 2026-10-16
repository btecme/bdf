//! Corpus family: annotations, with and without appearance streams.

use anyhow::Result;

use super::Ctx;
use super::model::{Case, Tolerance};
use super::pdfgen::{Doc, Page, lit, num, utf16};
use super::text_cases::block;

/// Three lines of text at known positions for markup annotations to sit on.
struct Sheet {
    doc: Doc,
    font: u32,
    bold: u32,
    annots: Vec<u32>,
    /// Annotations that count towards `Expect::annotations` (not links, not popups).
    counted: usize,
    links: usize,
    content: String,
}

const LINE_X: f64 = 72.0;
const LINE_W: f64 = 260.0;

fn line_y(i: usize) -> f64 {
    700.0 - 60.0 * i as f64
}

impl Sheet {
    fn new(title: &str, tokens: &[&str]) -> Self {
        let mut doc = Doc::new(title);
        let font = doc.std_font("Helvetica");
        let bold = doc.std_font("Helvetica-Bold");
        let mut content = String::new();
        for (i, t) in tokens.iter().enumerate() {
            content.push_str(&block("F1", 14.0, LINE_X, line_y(i), 16.0, &[format!("{t} sample line of text to mark up")], &|s| lit(s)));
        }
        Self { doc, font, bold, annots: Vec::new(), counted: 0, links: 0, content }
    }

    fn add(&mut self, dict: String) -> u32 {
        let id = self.doc.pdf.add(dict.into_bytes());
        self.annots.push(id);
        self.counted += 1;
        id
    }

    fn add_link(&mut self, dict: String) -> u32 {
        let id = self.doc.pdf.add(dict.into_bytes());
        self.annots.push(id);
        self.links += 1;
        id
    }

    /// A form XObject appearance whose BBox is the annotation rectangle (identity mapping).
    fn ap(&mut self, rect: [f64; 4], resources: &str, content: &str) -> Result<u32> {
        self.doc.pdf.add_stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [{} {} {} {}] /Resources << {resources} >>",
                num(rect[0]),
                num(rect[1]),
                num(rect[2]),
                num(rect[3])
            ),
            content.as_bytes(),
            false,
        )
    }

    fn finish(mut self, id: &str, desc: &str, tokens: &[&str]) -> Result<Case> {
        let (counted, links) = (self.counted, self.links);
        let mut page =
            Page::letter().with_content(self.content.into_bytes()).with_resources(format!("/Font << /F1 {} 0 R /F2 {} 0 R >>", self.font, self.bold));
        page.annots = self.annots;
        self.doc.add_page(page)?;
        let mut case = Case::new(id, "annotations", "small", desc, self.doc.finish()?).tolerance(Tolerance::Text).pages(1);
        for t in tokens {
            case = case.text(1, t);
        }
        case.expect.annotations = Some(counted);
        case.expect.links = (links > 0).then_some(links);
        Ok(case)
    }
}

fn rect_of(i: usize) -> [f64; 4] {
    [LINE_X - 2.0, line_y(i) - 4.0, LINE_X + LINE_W, line_y(i) + 14.0]
}

fn quad(r: [f64; 4]) -> String {
    format!("[{} {} {} {} {} {} {} {}]", num(r[0]), num(r[3]), num(r[2]), num(r[3]), num(r[0]), num(r[1]), num(r[2]), num(r[1]))
}

pub fn generate(_ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    // Text markup, with appearances (Multiply blend for the highlight) and without.
    for with_ap in [true, false] {
        let id = if with_ap { "annot-markup-ap" } else { "annot-markup-no-ap" };
        let tokens = ["BDFMARK-ONE", "BDFMARK-TWO", "BDFMARK-THREE", "BDFMARK-FOUR"];
        let mut s = Sheet::new(id, &tokens);
        for (i, (sub, colour)) in
            [("Highlight", "1 0.9 0"), ("Underline", "0 0.6 0"), ("StrikeOut", "1 0 0"), ("Squiggly", "0.5 0 0.8")].iter().enumerate()
        {
            let r = rect_of(i);
            let ap = if with_ap {
                let body = match *sub {
                    "Highlight" => format!("/GS gs {colour} rg {} {} {} {} re f", num(r[0]), num(r[1]), num(r[2] - r[0]), num(r[3] - r[1])),
                    "Underline" | "Squiggly" => format!("{colour} RG 1 w {} {} m {} {} l S", num(r[0]), num(r[1] + 1.0), num(r[2]), num(r[1] + 1.0)),
                    _ => format!("{colour} RG 1 w {} {} m {} {} l S", num(r[0]), num((r[1] + r[3]) / 2.0), num(r[2]), num((r[1] + r[3]) / 2.0)),
                };
                let res = "/ExtGState << /GS << /BM /Multiply >> >>";
                format!("/AP << /N {} 0 R >>", s.ap(r, res, &body)?)
            } else {
                String::new()
            };
            s.add(format!(
                "<< /Type /Annot /Subtype /{sub} /Rect [{} {} {} {}] /QuadPoints {} /C [{}] /Contents {} /T (reviewer) /F 4 {ap} >>",
                num(r[0]),
                num(r[1]),
                num(r[2]),
                num(r[3]),
                quad(r),
                colour,
                lit(&format!("{sub} note"))
            ));
        }
        let desc = if with_ap {
            "Highlight, underline, strike-out and squiggly with appearance streams"
        } else {
            "The same four text-markup annotations with no appearance stream"
        };
        out.push(s.finish(id, desc, &tokens)?);
    }

    // A note with a popup.
    {
        let mut s = Sheet::new("annot-note-popup", &["BDFNOTE-ONE"]);
        let popup = s.doc.pdf.reserve();
        let note = s.doc.pdf.add(
            format!(
                "<< /Type /Annot /Subtype /Text /Rect [400 700 424 724] /Name /Comment /Contents {} /T (reviewer) /Open false /C [1 0.8 0] /F 28 /Popup {popup} 0 R >>",
                lit("A sticky note with a popup window")
            )
            .into_bytes(),
        );
        s.doc.pdf.set(popup, format!("<< /Type /Annot /Subtype /Popup /Rect [430 640 560 720] /Parent {note} 0 R /Open false >>").into_bytes())?;
        s.annots.push(note);
        s.annots.push(popup);
        s.counted += 1; // the note; the popup is not counted
        out.push(s.finish("annot-note-popup", "A sticky-note (Text) annotation and its popup", &["BDFNOTE-ONE"])?);
    }

    // Freehand ink with an appearance.
    {
        let mut s = Sheet::new("annot-ink", &["BDFINK-ONE"]);
        let r = [300.0, 450.0, 540.0, 650.0];
        let body = "1 0 0 RG 3 w 1 J 1 j 310 460 m 360 640 l 420 470 l 480 640 l 530 460 l S 0 0 1 RG 2 w 320 560 m 520 560 l S";
        let ap = s.ap(r, "", body)?;
        s.add(format!(
            "<< /Type /Annot /Subtype /Ink /Rect [300 450 540 650] /InkList [[310 460 360 640 420 470 480 640 530 460] [320 560 520 560]] /C [1 0 0] /BS << /W 3 >> /F 4 /AP << /N {ap} 0 R >> >>"
        ));
        out.push(s.finish("annot-ink", "Two freehand ink strokes with an appearance", &["BDFINK-ONE"])?);
    }

    // Shapes, with and without appearances.
    for with_ap in [true, false] {
        let id = if with_ap { "annot-shapes-ap" } else { "annot-shapes-no-ap" };
        let mut s = Sheet::new(id, &["BDFSHAPE-ONE"]);
        let sq = [72.0, 400.0, 200.0, 500.0];
        let ci = [230.0, 400.0, 350.0, 500.0];
        let ln = [380.0, 400.0, 540.0, 500.0];
        let sq_ap = if with_ap {
            format!("/AP << /N {} 0 R >>", s.ap(sq, "", "0.2 0.4 0.9 RG 0.9 0.95 1 rg 3 w 73.5 401.5 125 97 re B")?)
        } else {
            String::new()
        };
        let ci_ap = if with_ap {
            let k = 0.552_284_75;
            let (cx, cy, rx, ry) = (290.0, 450.0, 58.0, 48.0);
            let body = format!(
                "0.8 0.1 0.1 RG 1 0.9 0.9 rg 3 w {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c B",
                num(cx + rx),
                num(cy),
                num(cx + rx),
                num(cy + k * ry),
                num(cx + k * rx),
                num(cy + ry),
                num(cx),
                num(cy + ry),
                num(cx - k * rx),
                num(cy + ry),
                num(cx - rx),
                num(cy + k * ry),
                num(cx - rx),
                num(cy),
                num(cx - rx),
                num(cy - k * ry),
                num(cx - k * rx),
                num(cy - ry),
                num(cx),
                num(cy - ry),
                num(cx + k * rx),
                num(cy - ry),
                num(cx + rx),
                num(cy - k * ry),
                num(cx + rx),
                num(cy)
            );
            format!("/AP << /N {} 0 R >>", s.ap(ci, "", &body)?)
        } else {
            String::new()
        };
        let ln_ap = if with_ap {
            format!("/AP << /N {} 0 R >>", s.ap(ln, "", "0 0.5 0 RG 3 w 385 405 m 535 495 l S 0 0.5 0 rg 535 495 m 521 493 l 530 481 l f")?)
        } else {
            String::new()
        };
        s.add(format!("<< /Type /Annot /Subtype /Square /Rect [72 400 200 500] /C [0.2 0.4 0.9] /IC [0.9 0.95 1] /BS << /W 3 >> /F 4 {sq_ap} >>"));
        s.add(format!("<< /Type /Annot /Subtype /Circle /Rect [230 400 350 500] /C [0.8 0.1 0.1] /IC [1 0.9 0.9] /BS << /W 3 >> /F 4 {ci_ap} >>"));
        s.add(format!("<< /Type /Annot /Subtype /Line /Rect [380 400 540 500] /L [385 405 535 495] /LE [/None /ClosedArrow] /C [0 0.5 0] /BS << /W 3 >> /F 4 {ln_ap} >>"));
        s.add("<< /Type /Annot /Subtype /Polygon /Rect [72 250 250 360] /Vertices [80 260 160 350 240 260] /C [0.5 0 0.5] /IC [0.95 0.85 0.95] /BS << /W 2 >> /F 4 >>".to_string());
        s.add("<< /Type /Annot /Subtype /PolyLine /Rect [300 250 540 360] /Vertices [310 260 380 350 450 270 530 350] /C [0 0.4 0.4] /BS << /W 2 >> /F 4 >>".to_string());
        let desc = if with_ap {
            "Square, circle and line with appearances; polygon and polyline without"
        } else {
            "Square, circle, line, polygon and polyline, none with an appearance"
        };
        out.push(s.finish(id, desc, &["BDFSHAPE-ONE"])?);
    }

    // Free text with an appearance, and a custom stamp.
    {
        let mut s = Sheet::new("annot-freetext-stamp", &["BDFFREE-ONE"]);
        let ft = [72.0, 520.0, 320.0, 580.0];
        let ap = s.ap(
            ft,
            &format!("/Font << /Helv {} 0 R >>", s.font),
            "1 1 0.8 rg 72 520 248 60 re f 0 G 1 w 72.5 520.5 247 59 re S BT /Helv 14 Tf 0 g 80 555 Td (BDFFREETEXT callout text) Tj ET",
        )?;
        s.add(format!(
            "<< /Type /Annot /Subtype /FreeText /Rect [72 520 320 580] /DA (/Helv 14 Tf 0 g) /Contents {} /C [1 1 0.8] /F 4 /AP << /N {ap} 0 R >> >>",
            lit("BDFFREETEXT callout text")
        ));
        let st = [340.0, 480.0, 540.0, 560.0];
        let ap = s.ap(
            st,
            &format!("/Font << /F2 {} 0 R >>", s.bold),
            "q 0.7 0.1 0.1 RG 4 w 345 485 190 70 re S BT /F2 22 Tf 0.7 0.1 0.1 rg 358 512 Td (BDF REVIEWED) Tj ET Q",
        )?;
        s.add(format!(
            "<< /Type /Annot /Subtype /Stamp /Rect [340 480 540 560] /Name /BdfReviewed /Contents {} /F 4 /AP << /N {ap} 0 R >> >>",
            lit("Custom review stamp")
        ));
        out.push(s.finish(
            "annot-freetext-stamp",
            "A free-text callout and a custom review stamp, both with appearances",
            &["BDFFREE-ONE", "BDFFREETEXT", "BDF REVIEWED"],
        )?);
    }

    // A file attachment annotation.
    {
        let mut s = Sheet::new("annot-fileattachment", &["BDFATTACH-ONE"]);
        let data = b"BDF annotation attachment payload\n";
        let stream = s.doc.pdf.add_stream(&format!("/Type /EmbeddedFile /Params << /Size {} >>", data.len()), data, true)?;
        let spec = s.doc.pdf.add(format!("<< /Type /Filespec /F (payload.txt) /UF (payload.txt) /EF << /F {stream} 0 R >> >>").into_bytes());
        s.add(format!(
            "<< /Type /Annot /Subtype /FileAttachment /Rect [400 650 420 670] /Name /Paperclip /Contents {} /FS {spec} 0 R /F 4 >>",
            utf16("Attached payload.txt")
        ));
        let mut case =
            s.finish("annot-fileattachment", "A file-attachment annotation (no appearance; the icon is the reader's)", &["BDFATTACH-ONE"])?;
        case.expect.attachments = vec!["payload.txt".to_string()];
        out.push(case);
    }

    // Link annotations with visible borders.
    {
        let mut s = Sheet::new("annot-links", &["BDFLINK-ONE", "BDFLINK-TWO"]);
        let r0 = rect_of(0);
        let r1 = rect_of(1);
        s.add_link(format!(
            "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 2] /C [0 0 1] /A << /S /URI /URI {} >> >>",
            num(r0[0]),
            num(r0[1]),
            num(r0[2]),
            num(r0[3]),
            lit("https://example.invalid/bdf-link")
        ));
        s.add_link(format!(
            "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 2] /C [1 0 0] /A << /S /Named /N /NextPage >> >>",
            num(r1[0]),
            num(r1[1]),
            num(r1[2]),
            num(r1[3])
        ));
        out.push(s.finish("annot-links", "A URI link and a named-action link with visible borders", &["BDFLINK-ONE", "BDFLINK-TWO"])?);
    }

    // Annotation flags: hidden, no-view, print-only, no-zoom and normal.
    {
        let mut s = Sheet::new("annot-flags", &["BDFFLAG-ONE"]);
        for (i, (flag, label)) in [(2u32, "hidden"), (32, "noview"), (4, "print"), (8, "nozoom"), (4 | 128, "locked-print")].iter().enumerate() {
            let x = 72.0 + 100.0 * i as f64;
            let r = [x, 500.0, x + 80.0, 560.0];
            let body = format!("1 0.5 0 rg {} 500 80 60 re f 0 g 2 w {} 500 80 60 re S", num(x), num(x));
            let ap = s.ap(r, "", &body)?;
            s.add(format!(
                "<< /Type /Annot /Subtype /Square /Rect [{} 500 {} 560] /C [1 0.5 0] /F {flag} /Contents {} /AP << /N {ap} 0 R >> >>",
                num(x),
                num(x + 80.0),
                lit(label)
            ));
        }
        let mut case = s.finish("annot-flags", "Five squares with Hidden, NoView, Print, NoZoom and Print+Locked flags", &["BDFFLAG-ONE"])?;
        case.expect.notes = "On screen the hidden (F=2) and noview (F=32) squares are not drawn; the print, nozoom and print+locked squares are. The Print bit affects printing only".to_string();
        out.push(case);
    }
    Ok(())
}

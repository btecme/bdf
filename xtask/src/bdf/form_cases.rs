//! Corpus family: AcroForm fields. Each field exists in a variant with a hand-written appearance
//! stream and, where it matters, one without (the reader must draw it itself).

use anyhow::Result;

use super::Ctx;
use super::model::{Case, Tolerance};
use super::pdfgen::{Doc, Page, lit, num};
use super::text_cases::block;

const HELV: &str = "/Helv 12 Tf 0 g";

/// Geometry of one widget.
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn arr(self) -> String {
        format!("[{} {} {} {}]", num(self.x), num(self.y), num(self.x + self.w), num(self.y + self.h))
    }
}

/// A form XObject appearance with its own coordinate system starting at the origin.
fn appearance(doc: &mut Doc, r: Rect, helv: u32, content: &str) -> Result<u32> {
    doc.pdf.add_stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Resources << /Font << /Helv {helv} 0 R >> >>", num(r.w), num(r.h)),
        content.as_bytes(),
        false,
    )
}

fn text_ap(doc: &mut Doc, r: Rect, helv: u32, value: &str) -> Result<u32> {
    appearance(
        doc,
        r,
        helv,
        &format!("/Tx BMC q 1 1 {} {} re W n BT {HELV} 2 {} Td {} Tj ET Q EMC", num(r.w - 2.0), num(r.h - 2.0), num((r.h - 9.0) / 2.0), lit(value)),
    )
}

fn border(r: Rect) -> String {
    format!(
        "q 0.96 0.96 0.9 rg 0 0 {w} {h} re f 0 G 0.75 w 0.5 0.5 {w2} {h2} re S Q ",
        w = num(r.w),
        h = num(r.h),
        w2 = num(r.w - 1.0),
        h2 = num(r.h - 1.0)
    )
}

fn tick(r: Rect) -> String {
    format!(
        "q 0 0.4 0 RG 2 w {} {} m {} {} l {} {} l S Q",
        num(r.w * 0.2),
        num(r.h * 0.5),
        num(r.w * 0.42),
        num(r.h * 0.2),
        num(r.w * 0.82),
        num(r.h * 0.8)
    )
}

fn dot(r: Rect) -> String {
    let (cx, cy, rad) = (r.w / 2.0, r.h / 2.0, r.w * 0.28);
    let k = 0.552_284_75 * rad;
    format!(
        "q 0 g {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f Q",
        num(cx + rad),
        num(cy),
        num(cx + rad),
        num(cy + k),
        num(cx + k),
        num(cy + rad),
        num(cx),
        num(cy + rad),
        num(cx - k),
        num(cy + rad),
        num(cx - rad),
        num(cy + k),
        num(cx - rad),
        num(cy),
        num(cx - rad),
        num(cy - k),
        num(cx - k),
        num(cy - rad),
        num(cx),
        num(cy - rad),
        num(cx + k),
        num(cy - rad),
        num(cx + rad),
        num(cy - k),
        num(cx + rad),
        num(cy),
    )
}

/// What a form builder collects while a page is laid out.
struct Form {
    doc: Doc,
    helv: u32,
    widgets: Vec<u32>,
    fields: Vec<u32>,
    names: Vec<String>,
    labels: Vec<String>,
}

impl Form {
    fn new(title: &str) -> Self {
        let mut doc = Doc::new(title);
        let helv = doc.std_font("Helvetica");
        Self { doc, helv, widgets: Vec::new(), fields: Vec::new(), names: Vec::new(), labels: Vec::new() }
    }

    fn label(&mut self, y: f64, text: &str) {
        self.labels.push(block("F1", 11.0, 72.0, y + 5.0, 12.0, &[text.to_string()], &|s| lit(s)));
    }

    /// A widget that is its own field.
    fn field(&mut self, dict: &str, name: &str) -> Result<u32> {
        let id = self.doc.pdf.add(dict.to_string().into_bytes());
        self.widgets.push(id);
        self.fields.push(id);
        self.names.push(name.to_string());
        Ok(id)
    }

    fn finish(mut self, need_appearances: bool, extra_acro: &str, id: &str, desc: &str) -> Result<Case> {
        let mut c = String::new();
        c.push_str(&block("F1", 14.0, 72.0, 740.0, 16.0, &[format!("BDFFORM {id}")], &|s| lit(s)));
        for l in &self.labels {
            c.push_str(l);
        }
        let mut page = Page::letter().with_content(c.into_bytes()).with_resources(format!("/Font << /F1 {} 0 R >>", self.helv));
        page.annots = self.widgets.clone();
        self.doc.add_page(page)?;
        let fields = self.fields.iter().map(|f| format!("{f} 0 R")).collect::<Vec<_>>().join(" ");
        self.doc.catalog = format!(
            "/AcroForm << /Fields [{fields}] /DA ({HELV}) /DR << /Font << /Helv {} 0 R >> >> /NeedAppearances {need_appearances} {extra_acro} >>",
            self.helv
        );
        let mut case =
            Case::new(id, "forms", "small", desc, self.doc.finish()?).tolerance(Tolerance::Text).pages(1).text(1, &format!("BDFFORM {id}"));
        case.expect.fields = self.names;
        Ok(case)
    }
}

pub fn generate(_ctx: &Ctx, out: &mut Vec<Case>) -> Result<()> {
    // Hierarchical names with appearance streams.
    for (id, with_ap) in [("form-text-basic", true), ("form-text-no-ap", false)] {
        let mut f = Form::new(id);
        let helv = f.helv;
        let r = Rect { x: 200.0, y: 680.0, w: 250.0, h: 22.0 };
        // Parent field "person" with two kids; "person.first" and "person.last".
        let parent = f.doc.pdf.reserve();
        let mut kids = Vec::new();
        for (i, (k, v)) in [("first", "Ada"), ("last", "Lovelace")].iter().enumerate() {
            let rr = Rect { y: r.y - 40.0 * i as f64, ..r };
            let ap = if with_ap { format!("/AP << /N {} 0 R >>", text_ap(&mut f.doc, rr, helv, v)?) } else { String::new() };
            let w = f.doc.pdf.add(format!("<< /Type /Annot /Subtype /Widget /Parent {parent} 0 R /T {} /V {} /Rect {} /F 4 /DA ({HELV}) /MK << /BC [0 0 0] /BG [0.96 0.96 0.9] >> {ap} >>", lit(k), lit(v), rr.arr()).into_bytes());
            kids.push(w);
            f.widgets.push(w);
            f.names.push(format!("person.{k}"));
            f.label(rr.y, &format!("{k} name"));
        }
        f.doc.pdf.set(
            parent,
            format!("<< /FT /Tx /T (person) /Kids [{}] >>", kids.iter().map(|k| format!("{k} 0 R")).collect::<Vec<_>>().join(" ")).into_bytes(),
        )?;
        f.fields.push(parent);
        let rr = Rect { y: r.y - 120.0, ..r };
        let ap = if with_ap { format!("/AP << /N {} 0 R >>", text_ap(&mut f.doc, rr, helv, "ada@example.invalid")?) } else { String::new() };
        f.field(
            &format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T (email) /V {} /Rect {} /F 4 /DA ({HELV}) /MK << /BC [0 0 0] >> {ap} >>",
                lit("ada@example.invalid"),
                rr.arr()
            ),
            "email",
        )?;
        f.label(rr.y, "email");
        let desc = if with_ap {
            "Hierarchical text fields (person.first, person.last) and email, with appearance streams"
        } else {
            "The same fields without appearance streams and /NeedAppearances true"
        };
        out.push(f.finish(!with_ap, "", id, desc)?);
    }

    // Checkboxes and a radio group, with Yes/Off appearances.
    {
        let mut f = Form::new("form-checks-radios");
        let helv = f.helv;
        let r = Rect { x: 200.0, y: 680.0, w: 18.0, h: 18.0 };
        for (i, (name, on)) in [("agree", true), ("newsletter", false)].iter().enumerate() {
            let rr = Rect { y: r.y - 36.0 * i as f64, ..r };
            let yes = appearance(&mut f.doc, rr, helv, &format!("{}{}", border(rr), tick(rr)))?;
            let off = appearance(&mut f.doc, rr, helv, &border(rr))?;
            let state = if *on { "Yes" } else { "Off" };
            f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Btn /T {} /V /{state} /AS /{state} /Rect {} /F 4 /AP << /N << /Yes {yes} 0 R /Off {off} 0 R >> >> >>", lit(name), rr.arr()), name)?;
            f.label(rr.y, name);
        }
        // Radio group "size": Small, Medium (selected), Large. Flags: radio (bit 16) + no-toggle-to-off (bit 15).
        let parent = f.doc.pdf.reserve();
        let mut kids = Vec::new();
        for (i, opt) in ["Small", "Medium", "Large"].iter().enumerate() {
            let rr = Rect { y: 560.0 - 36.0 * i as f64, ..r };
            let sel = *opt == "Medium";
            let on = appearance(&mut f.doc, rr, helv, &format!("{}{}", border(rr), dot(rr)))?;
            let off = appearance(&mut f.doc, rr, helv, &border(rr))?;
            let w = f.doc.pdf.add(format!("<< /Type /Annot /Subtype /Widget /Parent {parent} 0 R /Rect {} /F 4 /AS /{} /AP << /N << /{opt} {on} 0 R /Off {off} 0 R >> >> >>", rr.arr(), if sel { opt } else { &"Off" }).into_bytes());
            kids.push(w);
            f.widgets.push(w);
            f.label(rr.y, &format!("size: {opt}"));
        }
        f.doc.pdf.set(
            parent,
            format!(
                "<< /FT /Btn /Ff 49152 /T (size) /V /Medium /Kids [{}] >>",
                kids.iter().map(|k| format!("{k} 0 R")).collect::<Vec<_>>().join(" ")
            )
            .into_bytes(),
        )?;
        f.fields.push(parent);
        f.names.push("size".into());
        out.push(f.finish(false, "", "form-checks-radios", "Two checkboxes (one checked) and a three-button radio group with Medium selected")?);
    }

    // Combo box and list box.
    {
        let mut f = Form::new("form-choices");
        let helv = f.helv;
        let combo = Rect { x: 200.0, y: 680.0, w: 160.0, h: 22.0 };
        let ap = text_ap(&mut f.doc, combo, helv, "Green")?;
        f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (colour) /V (Green) /Opt [(Red) (Green) (Blue)] /Rect {} /F 4 /DA ({HELV}) /MK << /BC [0 0 0] >> /AP << /N {ap} 0 R >> >>", combo.arr()), "colour")?;
        f.label(combo.y, "colour (combo)");
        let list = Rect { x: 200.0, y: 560.0, w: 160.0, h: 70.0 };
        let content = format!("{}q BT {HELV} 3 52 Td 14 TL (North) Tj T* (East) Tj T* (South) Tj T* (West) Tj ET Q", border(list));
        let lap = appearance(&mut f.doc, list, helv, &content)?;
        f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Ch /T (direction) /V (East) /Opt [(North) (East) (South) (West)] /Rect {} /F 4 /DA ({HELV}) /AP << /N {lap} 0 R >> >>", list.arr()), "direction")?;
        f.label(list.y + 40.0, "direction (list)");
        out.push(f.finish(false, "", "form-choices", "A combo box (Green) and a list box (East) with option arrays")?);
    }

    // Multi-line text and a comb field.
    {
        let mut f = Form::new("form-multiline-comb");
        let helv = f.helv;
        let ml = Rect { x: 200.0, y: 560.0, w: 260.0, h: 110.0 };
        let text = "BDFMULTI line one wraps across the field and continues with more words";
        let content = format!(
            "{}q BT {HELV} 3 94 Td 14 TL (BDFMULTI line one wraps across the) Tj T* (field and continues with more) Tj T* (words) Tj ET Q",
            border(ml)
        );
        let ap = appearance(&mut f.doc, ml, helv, &content)?;
        f.field(
            &format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 4096 /T (comments) /V {} /Rect {} /F 4 /DA ({HELV}) /AP << /N {ap} 0 R >> >>",
                lit(text),
                ml.arr()
            ),
            "comments",
        )?;
        f.label(ml.y + 90.0, "comments (multiline)");
        let comb = Rect { x: 200.0, y: 500.0, w: 160.0, h: 24.0 };
        let mut content = border(comb);
        for (i, ch) in "12345".chars().enumerate() {
            content.push_str(&format!("q BT {HELV} {} 7 Td ({ch}) Tj ET Q ", num(5.0 + 20.0 * i as f64)));
        }
        let ap = appearance(&mut f.doc, comb, helv, &content)?;
        f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 16777216 /MaxLen 8 /T (code) /V (12345) /Rect {} /F 4 /DA ({HELV}) /AP << /N {ap} 0 R >> >>", comb.arr()), "code")?;
        f.label(comb.y, "code (comb, 8 cells)");
        out.push(f.finish(false, "", "form-multiline-comb", "A multi-line text field and an 8-cell comb field")?);
    }

    // Read-only and required flags.
    {
        let mut f = Form::new("form-flags");
        let helv = f.helv;
        let r = Rect { x: 200.0, y: 680.0, w: 200.0, h: 22.0 };
        for (i, (name, ff, value)) in
            [("locked", 1u32, "read only value"), ("needed", 2, "required value"), ("normal", 0, "editable value")].iter().enumerate()
        {
            let rr = Rect { y: r.y - 40.0 * i as f64, ..r };
            let ap = text_ap(&mut f.doc, rr, helv, value)?;
            f.field(
                &format!(
                    "<< /Type /Annot /Subtype /Widget /FT /Tx /Ff {ff} /T {} /V {} /Rect {} /F 4 /DA ({HELV}) /AP << /N {ap} 0 R >> >>",
                    lit(name),
                    lit(value),
                    rr.arr()
                ),
                name,
            )?;
            f.label(rr.y, &format!("{name} (Ff={ff})"));
        }
        out.push(f.finish(false, "", "form-flags", "Read-only, required and ordinary text fields")?);
    }

    // Calculation and format scripts: stored values must be shown; scripts must not run by default.
    {
        let mut f = Form::new("form-calculated");
        let helv = f.helv;
        let r = Rect { x: 200.0, y: 680.0, w: 120.0, h: 22.0 };
        let mut ids = Vec::new();
        for (i, (name, value)) in [("a", "2"), ("b", "3"), ("total", "5")].iter().enumerate() {
            let rr = Rect { y: r.y - 40.0 * i as f64, ..r };
            let ap = text_ap(&mut f.doc, rr, helv, value)?;
            let aa = if *name == "total" {
                format!("/AA << /C << /S /JavaScript /JS {} >> >>", lit("event.value = this.getField('a').value + this.getField('b').value;"))
            } else {
                String::new()
            };
            ids.push(f.field(
                &format!(
                    "<< /Type /Annot /Subtype /Widget /FT /Tx /T {} /V {} /Rect {} /F 4 /DA ({HELV}) /AP << /N {ap} 0 R >> {aa} >>",
                    lit(name),
                    lit(value),
                    rr.arr()
                ),
                name,
            )?);
            f.label(rr.y, name);
        }
        let co = ids.last().map_or(String::new(), |t| format!("/CO [{t} 0 R]"));
        let mut case =
            f.finish(false, &co, "form-calculated", "Fields a, b and a stored total with a calculation script (scripts are off by default)")?;
        case.expect.notes = "The stored value 5 must be shown. A reader that runs the calculation must produce the same 5; one that does not must not claim it recalculated".to_string();
        out.push(case);
    }

    // An unsigned signature field and a push button with a submit action (never executed).
    {
        let mut f = Form::new("form-sig-button");
        let helv = f.helv;
        let sig = Rect { x: 200.0, y: 640.0, w: 200.0, h: 60.0 };
        f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Sig /T (signature) /Rect {} /F 4 /MK << /BC [0 0 0] >> >>", sig.arr()), "signature")?;
        f.label(sig.y + 25.0, "signature (unsigned)");
        let btn = Rect { x: 200.0, y: 560.0, w: 120.0, h: 28.0 };
        let content = format!("q 0.8 0.85 0.95 rg 0 0 120 28 re f 0 G 1 w 0.5 0.5 119 27 re S BT {HELV} 28 9 Td (Submit) Tj ET Q");
        let ap = appearance(&mut f.doc, btn, helv, &content)?;
        f.field(&format!("<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (submit) /Rect {} /F 4 /AP << /N {ap} 0 R >> /A << /S /SubmitForm /F << /FS /URL /F {} >> >> >>", btn.arr(), lit("https://example.invalid/submit")), "submit")?;
        f.label(btn.y + 8.0, "submit (push button)");
        out.push(f.finish(false, "", "form-sig-button", "An empty signature field and a push button with a SubmitForm action")?);
    }
    Ok(())
}

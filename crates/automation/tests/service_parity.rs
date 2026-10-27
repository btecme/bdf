//! The automation tools and the application service agree on what a page number is, what a rotation does and what is
//! said when a request is wrong (BUILD-007: "retain existing page-number/coordinate conventions and errors through all
//! adapters"). The tools stay the authority until the adapters move onto the service; this test keeps the two from
//! drifting in the meantime.

use std::sync::Arc;

use pdfcraft_app_service::{
    ActorContext, DocumentKey, ErrorCode, OpenSource, Operation, OperationRequest, Outcome, PageSelection, RequestContext, Service,
};
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// A PDF with `n` 200×300 pt pages reading "Page 1", "Page 2", …
fn fixture(n: usize) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i).into_bytes());
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()).into_bytes());
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn tool(a: &mut Automation, name: &str, args: Value) -> Result<Value, ToolError> {
    a.call(name, &args).map(|mut content| match content.remove(0) {
        Content::Json(v) => v,
        other => panic!("{name}: expected JSON, got {other:?}"),
    })
}

fn tool_pages(a: &mut Automation, doc: u64) -> Vec<(u64, f64, f64, u64)> {
    let info = tool(a, "doc_info", json!({ "doc": doc })).expect("doc_info");
    info["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|p| {
            (
                p["page"].as_u64().expect("page"),
                p["width"].as_f64().expect("width"),
                p["height"].as_f64().expect("height"),
                p["rotation"].as_u64().expect("rotation"),
            )
        })
        .collect()
}

fn service_with(pages: usize) -> (Service, DocumentKey, u64) {
    let mut service = Service::new();
    let source = OpenSource::Bytes { name: "a.pdf".into(), bytes: Arc::new(fixture(pages)) };
    let done = service
        .execute(
            &RequestContext::new(1, ActorContext::local_user().expect("the current user")),
            OperationRequest::new(None, Operation::Open { source, password: None }),
        )
        .expect("opens");
    let Outcome::Opened { document, .. } = done.outcome else { panic!("opened") };
    (service, document, done.version.expect("version").content_revision)
}

fn service_pages(service: &Service, key: DocumentKey) -> Vec<(u64, f64, f64, u64)> {
    service
        .document(&ActorContext::local_user().expect("the current user"), key)
        .expect("open")
        .info()
        .pages
        .iter()
        .map(|p| (p.number as u64, f64::from(p.width), f64::from(p.height), u64::from(p.rotation)))
        .collect()
}

fn tool_automation(pages: usize) -> (Automation, u64, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("a.pdf"), fixture(pages)).expect("writes the fixture");
    let mut a = Automation::new().with_root(dir.path()).expect("a root");
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" })).expect("opens")["doc"].as_u64().expect("doc");
    (a, doc, dir)
}

#[test]
fn both_surfaces_number_pages_the_same_way() {
    let (mut a, doc, _dir) = tool_automation(3);
    let (service, key, _) = service_with(3);
    assert_eq!(tool_pages(&mut a, doc), service_pages(&service, key), "numbers from 1, sizes in displayed points, rotation in degrees");
}

#[test]
fn a_valid_rotation_does_the_same_thing_on_both() {
    for (pages, degrees) in [(vec![1], 90), (vec![2, 3], 180), (vec![3], -90), (vec![1, 2, 3], 270), (vec![1], 360), (vec![2], 0)] {
        let (mut a, doc, _dir) = tool_automation(3);
        let (mut service, key, revision) = service_with(3);
        tool(&mut a, "page_rotate", json!({ "doc": doc, "pages": pages, "degrees": degrees })).expect("tool rotates");
        service
            .execute(
                &RequestContext::new(2, ActorContext::local_user().expect("the current user")),
                OperationRequest::new(
                    Some(key),
                    Operation::Rotate { expected_revision: revision, pages: PageSelection::Numbers(pages.clone()), degrees },
                ),
            )
            .expect("service rotates");
        assert_eq!(tool_pages(&mut a, doc), service_pages(&service, key), "pages {pages:?} by {degrees}");
    }
}

#[test]
fn a_wrong_rotation_is_refused_with_the_same_words() {
    let cases: Vec<(&str, Vec<i64>, i64)> = vec![
        ("degrees", vec![1], 45),
        ("page zero", vec![0], 90),
        ("negative page", vec![-2], 90),
        ("page past the end", vec![1, 4], 90),
        ("empty list", Vec::new(), 90),
    ];
    for (what, pages, degrees) in cases {
        let (mut a, doc, _dir) = tool_automation(3);
        let (mut service, key, revision) = service_with(3);
        let ToolError::InvalidArgs(from_tool) =
            tool(&mut a, "page_rotate", json!({ "doc": doc, "pages": pages, "degrees": degrees })).expect_err(what)
        else {
            panic!("{what}: the tool should call it an invalid argument");
        };
        let from_service = service
            .execute(
                &RequestContext::new(2, ActorContext::local_user().expect("the current user")),
                OperationRequest::new(
                    Some(key),
                    Operation::Rotate { expected_revision: revision, pages: PageSelection::Numbers(pages.clone()), degrees },
                ),
            )
            .expect_err(what);
        assert_eq!(from_service.code, ErrorCode::InvalidArgument, "{what}");
        assert_eq!(from_service.message, from_tool, "{what}: the same sentence");
    }
}

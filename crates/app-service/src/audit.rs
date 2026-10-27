//! The audit log (BUILD-008): who did what to which document, in what state, with what result, and nothing else.
//!
//! A record holds the actor (human or client, the client's name and grant), the operation, the request number, the
//! document's number within the session, its content revision before and after, the outcome, and the effects outside
//! memory (a file name and its size). **It never holds a document's content, a secret, or a full path.** A file effect is
//! the file's name only; the document is its number, not its title.
//!
//! [`FileAudit`] keeps the log as JSON lines in a private folder (only the current user can read it, checked before anything
//! is written), in segments of bounded size with a bounded number of segments: the oldest is deleted when a new one starts,
//! so the log cannot fill the disk. Each line carries a short hash of the line before it and of itself, so an accidental
//! change (a truncated write, a damaged disk, a hand edit) is found by [`verify`] instead of being read as history. The
//! chain is not a signature: someone who can write the folder can rewrite all of it, and the guarantee is against accident.
//!
//! If a record cannot be written, the service stops serving clients until it can (`AuditUnavailable`): a client's action
//! that cannot be recorded does not happen. The person at the keyboard is not locked out of their own documents by a full
//! disk.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::actors::PrincipalKind;

/// Largest segment, and most segments kept: the log occupies at most 8 x 256 KiB.
pub const MAX_SEGMENT_BYTES: u64 = 256 * 1024;
pub const MAX_SEGMENTS: usize = 8;
const MAX_EFFECTS: usize = 8;
const MAX_NAME_CHARS: usize = 255;

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok,
    /// Refused by policy; the reason's stable spelling.
    Denied(String),
    /// Failed for another reason; the error code's stable spelling.
    Failed(String),
}

/// A change outside memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEffect {
    /// `file_written` or `document_closed`.
    pub kind: String,
    /// The file's name, never its path.
    pub file: Option<String>,
    pub bytes: Option<u64>,
}

/// What the service reports about one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    /// Seconds since the Unix epoch.
    pub at: i64,
    pub actor: PrincipalKind,
    pub client: Option<String>,
    pub grant: Option<String>,
    pub request_id: u64,
    pub operation: String,
    /// The document's number within the session.
    pub document: Option<u64>,
    pub revision_before: Option<u64>,
    pub revision_after: Option<u64>,
    pub outcome: AuditOutcome,
    pub effects: Vec<AuditEffect>,
    pub warnings: usize,
}

/// An entry as recorded, with its place in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRecord {
    pub seq: u64,
    pub entry: AuditEntry,
}

/// The log could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditError(pub String);

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the audit log could not be written: {}", self.0)
    }
}

impl std::error::Error for AuditError {}

/// Where records go.
pub trait AuditSink: Send {
    /// Record `entry`, returning it with its sequence number.
    fn record(&mut self, entry: AuditEntry) -> Result<AuditRecord, AuditError>;

    /// The most recent records this sink keeps in memory (none, for a sink that writes only to disk).
    fn recent(&self) -> Vec<AuditRecord> {
        Vec::new()
    }
}

/// The last records, in memory, bounded. The default sink of a service; readable through any clone of the handle.
#[derive(Clone)]
pub struct MemoryAudit {
    state: Arc<Mutex<Memory>>,
}

struct Memory {
    capacity: usize,
    next_seq: u64,
    records: VecDeque<AuditRecord>,
}

impl MemoryAudit {
    pub fn new(capacity: usize) -> Self {
        Self { state: Arc::new(Mutex::new(Memory { capacity: capacity.max(1), next_seq: 1, records: VecDeque::new() })) }
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).records.iter().cloned().collect()
    }
}

impl AuditSink for MemoryAudit {
    fn record(&mut self, entry: AuditEntry) -> Result<AuditRecord, AuditError> {
        let mut memory = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let record = AuditRecord { seq: memory.next_seq, entry: bounded(entry) };
        memory.next_seq = memory.next_seq.saturating_add(1);
        memory.records.push_back(record.clone());
        while memory.records.len() > memory.capacity {
            memory.records.pop_front();
        }
        Ok(record)
    }

    fn recent(&self) -> Vec<AuditRecord> {
        self.records()
    }
}

/// An entry with its free-text parts cut to a safe size.
fn bounded(mut entry: AuditEntry) -> AuditEntry {
    let cut = |s: &mut String| {
        if s.chars().count() > MAX_NAME_CHARS {
            *s = s.chars().take(MAX_NAME_CHARS).collect();
        }
    };
    cut(&mut entry.operation);
    entry.effects.truncate(MAX_EFFECTS);
    for effect in &mut entry.effects {
        cut(&mut effect.kind);
        if let Some(file) = &mut effect.file {
            cut(file);
        }
    }
    match &mut entry.outcome {
        AuditOutcome::Denied(why) | AuditOutcome::Failed(why) => cut(why),
        AuditOutcome::Ok => {}
    }
    entry
}

/// The log on disk: JSON lines in a private folder, bounded, chained.
pub struct FileAudit {
    dir: PathBuf,
    next_seq: u64,
    last_sum: String,
    segment: Option<Segment>,
    /// The existing log failed [`verify`] when it was opened; this run's records start a new chain.
    chain_was_broken: bool,
}

struct Segment {
    index: u32,
    bytes: u64,
    file: File,
}

impl FileAudit {
    /// Open the log in `dir`, creating the folder if needed. The existing log is verified; if it is damaged the damage is
    /// kept (see [`FileAudit::chain_was_broken`]) and new records start a new chain in a new segment.
    pub fn open(dir: &Path) -> Result<Self, AuditError> {
        std::fs::create_dir_all(dir).map_err(|e| AuditError(e.to_string()))?;
        let report = verify(dir).map_err(|e| AuditError(e.to_string()))?;
        let broken = report.first_bad.is_some();
        let last_sum = if broken { "restart".to_owned() } else { report.last_sum.clone().unwrap_or_else(|| "start".to_owned()) };
        let mut audit =
            Self { dir: dir.to_path_buf(), next_seq: report.last_seq.saturating_add(1), last_sum, segment: None, chain_was_broken: broken };
        if broken {
            // Never append to a segment that is already damaged.
            audit.start_segment(report.last_segment.saturating_add(1))?;
        } else if let Some(index) = report.last_segment_index() {
            audit.reopen_segment(index)?;
        }
        Ok(audit)
    }

    pub fn chain_was_broken(&self) -> bool {
        self.chain_was_broken
    }

    fn segment_path(&self, index: u32) -> PathBuf {
        self.dir.join(format!("audit-{index:06}.jsonl"))
    }

    fn start_segment(&mut self, index: u32) -> Result<(), AuditError> {
        let path = self.segment_path(index);
        // Created empty and shown private before any record is written into it.
        pdfcraft_platform::private_files::write_private_file(&path, b"", pdfcraft_platform::staging::StagingName::SuffixThenTag)
            .map_err(|e| AuditError(format!("{e}")))?;
        let file = OpenOptions::new().append(true).open(&path).map_err(|e| AuditError(e.to_string()))?;
        self.segment = Some(Segment { index, bytes: 0, file });
        self.prune();
        Ok(())
    }

    fn reopen_segment(&mut self, index: u32) -> Result<(), AuditError> {
        let path = self.segment_path(index);
        pdfcraft_platform::private_files::verify_private(&path).map_err(|e| AuditError(format!("not private: {e}")))?;
        let bytes = std::fs::metadata(&path).map_err(|e| AuditError(e.to_string()))?.len();
        let file = OpenOptions::new().append(true).open(&path).map_err(|e| AuditError(e.to_string()))?;
        self.segment = Some(Segment { index, bytes, file });
        Ok(())
    }

    /// Delete the oldest segments beyond [`MAX_SEGMENTS`].
    fn prune(&self) {
        let mut found = segments(&self.dir).unwrap_or_default();
        while found.len() > MAX_SEGMENTS {
            let oldest = found.remove(0);
            let _ = std::fs::remove_file(self.segment_path(oldest));
        }
    }
}

impl AuditSink for FileAudit {
    fn record(&mut self, entry: AuditEntry) -> Result<AuditRecord, AuditError> {
        let record = AuditRecord { seq: self.next_seq, entry: bounded(entry) };
        let line = encode(&record, &self.last_sum);
        let sum = line_sum(&line).ok_or_else(|| AuditError("a record could not be encoded".into()))?;
        let mut text = line;
        text.push('\n');
        let needed = u64::try_from(text.len()).unwrap_or(u64::MAX);
        let roll = match &self.segment {
            None => Some(1),
            Some(s) if s.bytes.saturating_add(needed) > MAX_SEGMENT_BYTES => Some(s.index.saturating_add(1)),
            Some(_) => None,
        };
        if let Some(index) = roll {
            self.start_segment(index)?;
        }
        let segment = self.segment.as_mut().ok_or_else(|| AuditError("no segment".into()))?;
        segment.file.write_all(text.as_bytes()).and_then(|()| segment.file.sync_data()).map_err(|e| AuditError(e.to_string()))?;
        segment.bytes = segment.bytes.saturating_add(needed);
        self.next_seq = self.next_seq.saturating_add(1);
        self.last_sum = sum;
        Ok(record)
    }
}

/// The JSON object of a record, without its chain fields (`prev`, `sum`).
fn body(record: &AuditRecord) -> Map<String, Value> {
    let e = &record.entry;
    let (outcome, reason) = match &e.outcome {
        AuditOutcome::Ok => ("ok", None),
        AuditOutcome::Denied(why) => ("denied", Some(why.as_str())),
        AuditOutcome::Failed(why) => ("failed", Some(why.as_str())),
    };
    let effects: Vec<Value> = e.effects.iter().map(|x| json!({ "kind": x.kind, "file": x.file, "bytes": x.bytes })).collect();
    let value = json!({
        "seq": record.seq,
        "at": e.at,
        "actor": e.actor.as_str(),
        "client": e.client,
        "grant": e.grant,
        "request": e.request_id,
        "op": e.operation,
        "doc": e.document,
        "rev_before": e.revision_before,
        "rev_after": e.revision_after,
        "outcome": outcome,
        "reason": reason,
        "effects": effects,
        "warnings": e.warnings,
    });
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The text of the line a record is written as: its fields, the previous line's sum, and its own sum. The keys are sorted
/// (serde_json's map is ordered), so the text is the same wherever it is produced.
fn encode(record: &AuditRecord, prev: &str) -> String {
    let mut map = body(record);
    map.insert("prev".into(), json!(prev));
    let without_sum = serde_json::to_string(&map).unwrap_or_default();
    map.insert("sum".into(), json!(sum_of(&without_sum)));
    serde_json::to_string(&map).unwrap_or_default()
}

fn sum_of(text_without_sum: &str) -> String {
    let digest = Sha256::digest(text_without_sum.as_bytes());
    crate::grants::hex(digest.get(..8).unwrap_or(&[]))
}

/// The `sum` a line carries, if the line is a record whose sum is right.
fn line_sum(line: &str) -> Option<String> {
    let mut map: Map<String, Value> = serde_json::from_str(line).ok()?;
    let claimed = map.remove("sum")?.as_str()?.to_owned();
    let actual = sum_of(&serde_json::to_string(&map).ok()?);
    (claimed == actual).then_some(claimed)
}

/// The segment numbers in `dir`, ascending.
fn segments(dir: &Path) -> io::Result<Vec<u32>> {
    let mut found: Vec<u32> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let digits = name.strip_prefix("audit-")?.strip_suffix(".jsonl")?;
            (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then(|| digits.parse::<u32>().ok()).flatten()
        })
        .collect();
    found.sort_unstable();
    Ok(found)
}

/// The last `count` lines of the log in `dir` (each a JSON record, newest last), reading the newest segments first. The
/// log holds no content, secret or full path, so its lines can be shown as they are. An empty or missing log is no lines.
pub fn recent_lines(dir: &Path, count: usize) -> io::Result<Vec<String>> {
    let found = match segments(dir) {
        Ok(found) => found,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut lines: Vec<String> = Vec::new();
    for index in found.iter().rev() {
        let path = dir.join(format!("audit-{index:06}.jsonl"));
        let (bytes, _) = pdfcraft_platform::file_identity::read_regular_with_version(&path, MAX_SEGMENT_BYTES * 2)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut segment: Vec<String> = text.lines().map(str::to_owned).collect();
        segment.append(&mut lines);
        lines = segment;
        if lines.len() >= count {
            break;
        }
    }
    let skip = lines.len().saturating_sub(count);
    Ok(lines.split_off(skip))
}

/// What [`verify`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub segments: Vec<u32>,
    pub records: u64,
    pub last_seq: u64,
    pub last_sum: Option<String>,
    pub last_segment: u32,
    /// The first place the chain does not hold: the segment number and the line number (from 1) in it.
    pub first_bad: Option<(u32, usize)>,
}

impl VerifyReport {
    fn last_segment_index(&self) -> Option<u32> {
        self.segments.last().copied()
    }
}

/// Read every segment in `dir` and check the chain: each line's sum, each line's link to the one before, and the sequence
/// numbers. The first line of the oldest segment starts the chain wherever it points (earlier segments were deleted).
pub fn verify(dir: &Path) -> io::Result<VerifyReport> {
    let mut report = VerifyReport { segments: segments(dir)?, ..VerifyReport::default() };
    let mut prev: Option<String> = None;
    let mut seq: Option<u64> = None;
    for &index in &report.segments.clone() {
        report.last_segment = index;
        let path = dir.join(format!("audit-{index:06}.jsonl"));
        // A segment is bounded; reading it is bounded too (twice the limit leaves room for the last line).
        let (bytes, _) = pdfcraft_platform::file_identity::read_regular_with_version(&path, MAX_SEGMENT_BYTES * 2)?;
        let text = String::from_utf8_lossy(&bytes);
        for (number, line) in text.lines().enumerate() {
            let line_number = number.saturating_add(1);
            let Some(sum) = line_sum(line) else {
                report.first_bad.get_or_insert((index, line_number));
                continue;
            };
            let map: Option<Map<String, Value>> = serde_json::from_str(line).ok();
            let line_prev = map.as_ref().and_then(|m| m.get("prev")).and_then(Value::as_str).map(str::to_owned);
            let line_seq = map.as_ref().and_then(|m| m.get("seq")).and_then(Value::as_u64);
            let links = match (&prev, &line_prev) {
                (None, _) => true,
                (Some(before), Some(claimed)) => before == claimed,
                _ => false,
            };
            let in_order = match (seq, line_seq) {
                (None, Some(_)) => true,
                (Some(before), Some(now)) => now == before.saturating_add(1),
                _ => false,
            };
            if !(links && in_order) {
                report.first_bad.get_or_insert((index, line_number));
            }
            report.records = report.records.saturating_add(1);
            prev = Some(sum.clone());
            seq = line_seq.or(seq);
            report.last_sum = Some(sum);
        }
    }
    report.last_seq = seq.unwrap_or(0);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(request: u64) -> AuditEntry {
        AuditEntry {
            at: 1_700_000_000,
            actor: PrincipalKind::Agent,
            client: Some("claude".into()),
            grant: Some("g-0123456789abcdef".into()),
            request_id: request,
            operation: "page.rotate".into(),
            document: Some(1),
            revision_before: Some(3),
            revision_after: Some(4),
            outcome: AuditOutcome::Ok,
            effects: vec![AuditEffect { kind: "file_written".into(), file: Some("out.pdf".into()), bytes: Some(1234) }],
            warnings: 0,
        }
    }

    fn dir(test: &str) -> PathBuf {
        crate::testutil::private_dir(&format!("audit-{test}")).join("log")
    }

    #[test]
    fn memory_audit_is_bounded_and_numbers_its_records() {
        let mut log = MemoryAudit::new(3);
        for n in 0..5 {
            log.record(entry(n)).expect("records");
        }
        let kept: Vec<u64> = log.records().iter().map(|r| r.seq).collect();
        assert_eq!(kept, [3, 4, 5], "the oldest are dropped, the numbers keep counting");
    }

    #[test]
    fn a_file_log_chains_its_records_and_verifies() {
        let d = dir("chain");
        let mut log = FileAudit::open(&d).expect("opens");
        for n in 0..5 {
            log.record(entry(n)).expect("records");
        }
        let report = verify(&d).expect("verifies");
        assert_eq!((report.records, report.last_seq, report.first_bad), (5, 5, None));
        // A new process continues the chain and the numbering.
        let mut again = FileAudit::open(&d).expect("reopens");
        assert!(!again.chain_was_broken());
        again.record(entry(5)).expect("records");
        let report = verify(&d).expect("verifies");
        assert_eq!((report.records, report.last_seq, report.first_bad), (6, 6, None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn accidental_damage_is_found_and_never_appended_to() {
        let d = dir("damage");
        let mut log = FileAudit::open(&d).expect("opens");
        for n in 0..4 {
            log.record(entry(n)).expect("records");
        }
        let path = d.join("audit-000001.jsonl");
        let text = std::fs::read_to_string(&path).expect("reads");
        // Edit a record in place (a changed number, the sort of thing a damaged disk or a hand edit does).
        let edited = text.replacen("\"rev_after\":4", "\"rev_after\":9", 1);
        assert_ne!(text, edited);
        std::fs::write(&path, &edited).expect("writes");
        assert_eq!(verify(&d).expect("verifies").first_bad, Some((1, 1)));
        // A removed line breaks the sequence and the link.
        let lines: Vec<&str> = text.lines().collect();
        std::fs::write(&path, format!("{}\n{}\n{}\n", lines[0], lines[1], lines[3])).expect("writes");
        assert_eq!(verify(&d).expect("verifies").first_bad, Some((1, 3)));
        // Truncated in the middle of a line.
        std::fs::write(&path, &text[..text.len() - 20]).expect("writes");
        assert!(verify(&d).expect("verifies").first_bad.is_some());
        // Opening a damaged log says so, keeps the damaged segment as it was, and starts a new chain in a new segment.
        let damaged = std::fs::read(&path).expect("reads");
        let mut reopened = FileAudit::open(&d).expect("opens");
        assert!(reopened.chain_was_broken());
        reopened.record(entry(9)).expect("records");
        assert_eq!(std::fs::read(&path).expect("reads"), damaged, "the damaged segment was not appended to");
        assert!(d.join("audit-000002.jsonl").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_log_is_bounded_and_the_chain_survives_pruning() {
        let d = dir("bounded");
        let mut log = FileAudit::open(&d).expect("opens");
        let mut big = entry(0);
        big.effects =
            (0..8).map(|i| AuditEffect { kind: "file_written".into(), file: Some(format!("{i}{}", "x".repeat(250))), bytes: Some(i) }).collect();
        // Each record is about 2.3 KiB, so a segment holds ~110; write enough for more than eight segments.
        for n in 0..1100 {
            big.request_id = n;
            log.record(big.clone()).expect("records");
        }
        let found = segments(&d).expect("lists");
        assert!(found.len() <= MAX_SEGMENTS, "{found:?}");
        let total: u64 = found.iter().map(|i| std::fs::metadata(d.join(format!("audit-{i:06}.jsonl"))).expect("metadata").len()).sum();
        assert!(total <= MAX_SEGMENTS as u64 * MAX_SEGMENT_BYTES, "{total}");
        let report = verify(&d).expect("verifies");
        assert_eq!(report.first_bad, None, "the oldest segments were deleted and the rest still verifies");
        assert_eq!(report.last_seq, 1100);
        assert!(found.first().is_some_and(|first| *first > 1), "the oldest were pruned: {found:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_record_holds_no_path_content_or_secret() {
        let d = dir("redaction");
        let mut log = FileAudit::open(&d).expect("opens");
        let mut e = entry(1);
        e.effects[0].file = Some("report.pdf".into());
        log.record(e).expect("records");
        let text = std::fs::read_to_string(d.join("audit-000001.jsonl")).expect("reads");
        assert!(text.contains("report.pdf"));
        for forbidden in ["\\\\", "C:", "/home", "secret", "password", "bdfg_"] {
            assert!(!text.contains(forbidden), "{forbidden}: {text}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn long_names_are_cut() {
        let mut log = MemoryAudit::new(4);
        let mut e = entry(1);
        e.operation = "o".repeat(10_000);
        e.effects = (0..50).map(|_| AuditEffect { kind: "k".into(), file: Some("f".repeat(10_000)), bytes: None }).collect();
        let record = log.record(e).expect("records");
        assert_eq!(record.entry.operation.chars().count(), MAX_NAME_CHARS);
        assert_eq!(record.entry.effects.len(), MAX_EFFECTS);
        assert!(record.entry.effects.iter().all(|x| x.file.as_ref().is_some_and(|f| f.chars().count() == MAX_NAME_CHARS)));
    }
}

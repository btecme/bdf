//! Page text for the surfaces that read or search it: extraction with one renderer per worker thread, and a bounded,
//! session-wide cache. Active results own their Arcs independently of the cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
pub use pdfcraft_render::{PageText, TextGlyph};

use crate::DocId;

const BYTE_BUDGET: usize = 64 * 1024 * 1024;
// Also bound bookkeeping for documents with thousands of empty pages.
const PAGE_BUDGET: usize = 4096;
type Key = (DocId, usize);

/// Page text by 0-based page, or why that page could not be read.
pub type PageTexts = Vec<(usize, Result<Arc<PageText>, String>)>;

/// Extract the text of `pages` (0-based) with one renderer per worker thread.
pub fn extract_pages(bytes: &Arc<Vec<u8>>, password: Option<Arc<str>>, pages: &[usize]) -> PageTexts {
    let extract = |r: &mut PageRenderer, p: usize| {
        let out = r.render(RenderRequest { page: p, kind: RequestKind::Text, tile: None, scale: 1.0, tag: 0, preview: false });
        match out.error {
            Some(e) => Err(e),
            None => Ok(out.text.unwrap_or_default()),
        }
    };
    let config = RenderConfig { password, ..Default::default() };
    let workers = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).min(8) };
    // Small jobs aren't worth a second parse of the document.
    if workers == 1 || pages.len() < 8 {
        let mut r = PageRenderer::new(bytes.clone(), config);
        return pages.iter().map(|p| (*p, extract(&mut r, *p))).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: PageTexts = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| {
                    let mut r = PageRenderer::new(bytes.clone(), config.clone());
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(p) = pages.get(i) else { break };
                        done.push((*p, extract(&mut r, *p)));
                    }
                    done
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    out.sort_by_key(|(p, _)| *p);
    out
}

struct Cached {
    version: Weak<Vec<u8>>,
    text: Arc<PageText>,
    bytes: usize,
    stamp: u64,
}

pub struct TextCache {
    pages: HashMap<Key, Cached>,
    order: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
    budget: usize,
}

impl Default for TextCache {
    fn default() -> Self {
        Self::new(BYTE_BUDGET)
    }
}

impl TextCache {
    pub fn new(budget: usize) -> Self {
        Self { pages: HashMap::new(), order: BTreeMap::new(), clock: 0, bytes: 0, budget }
    }

    pub fn prepare(&mut self, id: DocId, bytes: &Arc<Vec<u8>>) {
        let version = Arc::downgrade(bytes);
        // At most PAGE_BUDGET entries, independent of the document's page count. Weak pointers
        // distinguish revisions without keeping an obsolete PDF byte buffer alive.
        let stale: Vec<_> = self.pages.iter().filter(|(key, page)| key.0 == id && !page.version.ptr_eq(&version)).map(|(key, _)| *key).collect();
        for key in stale {
            self.remove_page(key);
        }
    }

    pub fn remove(&mut self, id: &DocId) {
        let keys: Vec<_> = self.pages.keys().filter(|key| key.0 == *id).copied().collect();
        for key in keys {
            self.remove_page(key);
        }
    }

    fn remove_page(&mut self, key: Key) {
        if let Some(page) = self.pages.remove(&key) {
            self.order.remove(&page.stamp);
            self.bytes -= page.bytes;
        }
    }

    fn stamp(&mut self) -> u64 {
        if self.clock == u64::MAX {
            self.pages.clear();
            self.order.clear();
            self.bytes = 0;
            self.clock = 0;
        }
        self.clock += 1;
        self.clock
    }

    pub fn get(&mut self, id: DocId, page: usize) -> Option<Arc<PageText>> {
        let stamp = self.stamp();
        let entry = self.pages.get_mut(&(id, page))?;
        self.order.remove(&entry.stamp);
        entry.stamp = stamp;
        self.order.insert(stamp, (id, page));
        Some(entry.text.clone())
    }

    pub fn insert(&mut self, id: DocId, page: usize, version: &Arc<Vec<u8>>, text: Arc<PageText>) {
        let bytes = retained_bytes(&text);
        self.remove_page((id, page));
        // An oversized page is usable by this call, but never retained by the cache.
        if bytes > self.budget {
            return;
        }
        while self.bytes > self.budget - bytes || self.pages.len() >= PAGE_BUDGET {
            let Some((_, key)) = self.order.first_key_value() else { break };
            self.remove_page(*key);
        }
        let stamp = self.stamp();
        self.pages.insert((id, page), Cached { version: Arc::downgrade(version), text, bytes, stamp });
        self.order.insert(stamp, (id, page));
        self.bytes += bytes;
    }

    /// How many pages are held.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// The estimated bytes held, never more than the budget.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

// Estimate owned allocations, including spare capacity; bookkeeping has its own page-count cap.
fn retained_bytes(text: &PageText) -> usize {
    let arrays = std::mem::size_of::<PageText>()
        .saturating_add(2 * std::mem::size_of::<usize>())
        .saturating_add(text.glyphs.capacity().saturating_mul(std::mem::size_of::<TextGlyph>()))
        .saturating_add(text.line_of.capacity().saturating_mul(std::mem::size_of::<u32>()))
        .saturating_add(text.space_before.capacity().saturating_mul(std::mem::size_of::<bool>()));
    text.glyphs.iter().fold(arrays, |sum, glyph| sum.saturating_add(glyph.text.capacity()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_counts_owned_capacity_and_lru_is_shared_across_documents() {
        let mut text = PageText::default();
        text.glyphs.reserve(32);
        text.line_of.reserve(48);
        text.space_before.reserve(64);
        let mut word = String::with_capacity(1000);
        word.push('a');
        text.glyphs.push(TextGlyph { text: word, rect: [0.0; 4], quad: [[0.0; 2]; 4], direction: [1.0, 0.0] });
        let cost = retained_bytes(&text);
        assert!(cost >= 32 * std::mem::size_of::<TextGlyph>() + 48 * 4 + 64 + 1000);
        let text = Arc::new(text);
        let version = Arc::new(vec![1]);
        let mut cache = TextCache::new(cost * 2);
        cache.insert(DocId(1), 0, &version, text.clone());
        cache.insert(DocId(2), 0, &version, text.clone());
        assert!(Arc::ptr_eq(&cache.get(DocId(1), 0).unwrap(), &text));
        cache.insert(DocId(3), 0, &version, text.clone());
        assert!(cache.get(DocId(2), 0).is_none(), "least recently used page in another document is evicted");
        assert_eq!(cache.bytes, 2 * cost);
        cache.insert(DocId(1), 0, &version, text.clone());
        assert_eq!(cache.bytes, 2 * cost, "replacement does not double-count");
        let next_version = Arc::new(vec![1]);
        cache.prepare(DocId(1), &next_version);
        assert!(cache.get(DocId(1), 0).is_none(), "equal bytes in a new Arc are a new version");
        assert_eq!(cache.bytes, cost);
        cache.remove(&DocId(3));
        assert_eq!((cache.bytes, cache.len(), cache.order.len()), (0, 0, 0));
        assert_eq!(Arc::strong_count(&version), 1, "the cache never retains a PDF byte buffer");
    }

    #[test]
    fn oversized_pages_bypass_cache_and_empty_pages_have_a_bookkeeping_limit() {
        let text = Arc::new(PageText::default());
        let version = Arc::new(vec![]);
        let cost = retained_bytes(&text);
        let mut tiny = TextCache::new(cost - 1);
        tiny.insert(DocId(1), 0, &version, text.clone());
        assert_eq!((tiny.bytes, tiny.len()), (0, 0));
        let mut cache = TextCache::default();
        for page in 0..PAGE_BUDGET + 50 {
            cache.insert(DocId(1), page, &version, text.clone());
        }
        assert_eq!(cache.len(), PAGE_BUDGET);
        assert!(cache.get(DocId(1), 0).is_none());
        assert!(cache.get(DocId(1), PAGE_BUDGET + 49).is_some());
        assert_eq!(cache.bytes, PAGE_BUDGET * cost);
        cache.clock = u64::MAX;
        cache.insert(DocId(2), 0, &version, text);
        assert_eq!((cache.bytes, cache.len()), (cost, 1), "recency counter rollover safely clears old state");
    }
}

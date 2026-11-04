//! A page as a PNG: one renderer per open document, kept while the document's displayed bytes and reading colours stay
//! as they were, and the PNG encoder that the application service and the automation tools share.

use std::collections::HashMap;
use std::sync::Arc;

use pdfcraft_render::{PageRenderer, ReadingColors, RenderConfig, RenderRequest, RequestKind};

use crate::{DocId, Session};

/// A rendered page.
pub struct PagePng {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

struct Raster {
    display: Arc<Vec<u8>>,
    colors: ReadingColors,
    renderer: PageRenderer,
}

/// The renderers of a session's documents. An edit gives a document new displayed bytes, and the next render builds a
/// fresh renderer from them; nothing rendered before the edit is ever served after it.
#[derive(Default)]
pub struct PageRasters {
    renderers: HashMap<DocId, Raster>,
}

impl PageRasters {
    /// Render 0-based `page` of `id` at `scale` device pixels per point (the renderer reduces a scale that would pass its
    /// size and pixel limits) and encode it. `Err` says why not, in words fit to show.
    pub fn render_png(&mut self, session: &Session, id: DocId, page: usize, scale: f32) -> Result<PagePng, String> {
        let doc = session.get(id).ok_or_else(|| "no such document".to_owned())?;
        let colors = session.reading_colors(id).unwrap_or_default();
        let fresh = || {
            let config = RenderConfig { password: doc.password.as_deref().map(Arc::from), reading_colors: colors, ..Default::default() };
            Raster { display: doc.display.clone(), colors, renderer: PageRenderer::new(doc.display.clone(), config) }
        };
        let entry = self.renderers.entry(id).or_insert_with(fresh);
        if !Arc::ptr_eq(&entry.display, &doc.display) || entry.colors != colors {
            *entry = fresh();
        }
        let out = entry.renderer.render(RenderRequest { page, kind: RequestKind::Pixels, tile: None, scale, tag: 0, preview: false });
        if let Some(error) = out.error {
            return Err(format!("page {} could not be rendered: {error}", page.saturating_add(1)));
        }
        let png = encode_png(out.width, out.height, &out.rgba)?;
        Ok(PagePng { width: out.width, height: out.height, png })
    }

    /// Forget the renderer of a closed document.
    pub fn remove(&mut self, id: DocId) {
        self.renderers.remove(&id);
    }

    /// How many documents have a renderer.
    pub fn len(&self) -> usize {
        self.renderers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.renderers.is_empty()
    }
}

/// A PNG of premultiplied RGBA8 pixels, `width` by `height`.
pub fn encode_png(width: u32, height: u32, premultiplied: &[u8]) -> Result<Vec<u8>, String> {
    let expected = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4));
    if expected != u64::try_from(premultiplied.len()).ok() {
        return Err("the pixels do not match the size of the image".to_owned());
    }
    let mut rgba = premultiplied.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgba).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

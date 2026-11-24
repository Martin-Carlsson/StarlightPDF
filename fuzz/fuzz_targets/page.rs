//! A file around a target's data, rendered small and across three bands.

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::rendering::RenderOptions;

/// A stream object of `data` with `entries` beside its Length.
pub(crate) fn stream(entries: &str, data: &[u8]) -> Vec<u8> {
    [
        format!("<< {entries} /Length {} >>\nstream\n", data.len()).as_bytes(),
        data,
        b"\nendstream",
    ]
    .concat()
}

/// Renders a page of `content` with `resources` at 32 pixels, and for one input in 16 also a strip of it a pixel
/// wide and 2100 tall, which takes three bands; `objects` are numbered from 5. Returns the clauses each render
/// warned of.
pub(crate) fn render(content: &[u8], resources: &str, objects: Vec<Vec<u8>>) -> Vec<Vec<String>> {
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << {resources} >> /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream("", content),
    ];
    all.extend(objects);
    let strip = all.len().saturating_add(1);
    all.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /CropBox [50 0 50.05 100] /Resources << {resources} >> /Contents 4 0 R >>"
        )
        .into_bytes(),
    );
    if let Some(pages) = all.get_mut(1) {
        *pages = format!("<< /Type /Pages /Kids [3 0 R {strip} 0 R] /Count 2 >>").into_bytes();
    }
    let mut file = b"%PDF-1.7\n".to_vec();
    let mut table = b"0000000000 65535 f \n".to_vec();
    for (object, number) in all.iter().zip(1_u32..) {
        table.extend_from_slice(format!("{:010} 00000 n \n", file.len()).as_bytes());
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend_from_slice(object);
        file.extend_from_slice(b"\nendobj\n");
    }
    let size = all.len().saturating_add(1);
    let start = file.len();
    file.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    file.extend(table);
    file.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n").as_bytes(),
    );
    let pages = if file.len().is_multiple_of(16) { 2 } else { 1 };
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(file);
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1_u64 << 20_u32);
    let mut rendered = Vec::new();
    let Ok(document) = Source::from_bytes(&source).and_then(|made| {
        Document::open(
            &made,
            Some(&ResourcePolicy::new(&limits)?),
            &OpenOptions::default(),
        )
    }) else {
        return rendered;
    };
    let mut seen = 0;
    for (index, fit) in [(0, 32), (1, 2100)].into_iter().take(pages) {
        let mut chosen = PageOptions::default();
        chosen.index = index;
        let mut options = RenderOptions::default();
        options.fit = Some(fit);
        drop(document.page(&chosen).and_then(|shown| {
            shown.render(
                &Sink::in_memory(&MemorySinkOptions::default())?,
                None,
                &options,
            )
        }));
        // The document keeps every render's warnings.
        let mut clauses = Vec::new();
        if let Ok(warned) = document.warnings() {
            while let Ok(Some(found)) = warned.next() {
                clauses.push(found.clause);
            }
        }
        let fresh = clauses.split_off(seen.min(clauses.len()));
        seen = seen.saturating_add(fresh.len());
        rendered.push(fresh);
    }
    rendered
}

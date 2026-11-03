//! SVG pages → one multi-page PDF through `svg2pdf` (each page converted to an
//! XObject chunk, then assembled with `pdf-writer`). Text uses the embedded
//! DejaVu Sans font (Bitstream Vera licence, see assets/fonts/LICENSE_DEJAVU),
//! so the PDF is the same natively and in the browser.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};
use svg2pdf::usvg;

use crate::{Error, Result};

static FONT: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");

fn options() -> &'static usvg::Options<'static> {
    static OPT: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPT.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(FONT.to_vec());
        db.set_sans_serif_family("DejaVu Sans");
        let mut o = usvg::Options { font_family: "DejaVu Sans".into(), ..Default::default() };
        o.fontdb = Arc::new(db);
        o
    })
}

/// Converts SVG pages (sizes in mm) to a PDF.
pub fn pdf_from_svgs(pages: &[String]) -> Result<Vec<u8>> {
    let mut alloc = Ref::new(1);
    let catalog = alloc.bump();
    let tree_id = alloc.bump();
    let mut pdf = Pdf::new();
    let mut page_ids = Vec::new();
    let mut chunks = Vec::new();
    for (i, svg) in pages.iter().enumerate() {
        let tree = usvg::Tree::from_str(svg, options()).map_err(|e| Error::Invalid(format!("page {}: {e}", i + 1)))?;
        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default()).map_err(|e| Error::Invalid(format!("page {}: {e}", i + 1)))?;
        let mut map = HashMap::new();
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_ref = map[&svg_ref];
        // usvg sizes are CSS px at 96 dpi; PDF points are 1/72 in.
        let size = tree.size();
        let r = |v: f32| (v * 72.0 / 96.0 * 100.0).round() / 100.0;
        let (wpt, hpt) = (r(size.width()), r(size.height()));
        let page_id = alloc.bump();
        let content_id = alloc.bump();
        let name = format!("S{i}");
        {
            let mut page = pdf.page(page_id);
            page.media_box(Rect::new(0.0, 0.0, wpt, hpt));
            page.parent(tree_id);
            page.contents(content_id);
            let mut res = page.resources();
            res.x_objects().pair(Name(name.as_bytes()), svg_ref);
            res.finish();
            page.finish();
        }
        let mut content = Content::new();
        content.transform([wpt, 0.0, 0.0, hpt, 0.0, 0.0]).x_object(Name(name.as_bytes()));
        pdf.stream(content_id, &content.finish());
        page_ids.push(page_id);
        chunks.push(chunk);
    }
    pdf.catalog(catalog).pages(tree_id);
    pdf.pages(tree_id).kids(page_ids.iter().copied()).count(page_ids.len() as i32);
    for c in &chunks {
        pdf.extend(c);
    }
    Ok(pdf.finish())
}

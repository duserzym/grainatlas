//! SVG pages → one multi-page PDF through `svg2pdf` (each page converted to an
//! XObject chunk, then assembled with `pdf-writer`). Text uses the embedded
//! DejaVu Sans font (Bitstream Vera licence, see assets/fonts/LICENSE_DEJAVU),
//! so the PDF is the same natively and in the browser.

use std::collections::{HashMap, HashSet};

mod objects;
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
        let mut o = usvg::Options {
            font_family: "DejaVu Sans".into(),
            ..Default::default()
        };
        o.fontdb = Arc::new(db);
        o
    })
}

/// Incremental PDF assembler. One push is a cancellation/progress boundary.
/// Generated root-level grayscale PNG images reuse their IDAT streams: no pixel
/// decode, RGB expansion or second compression. Vector layers preserve SVG order.
pub struct PdfBuilder {
    alloc: Ref,
    catalog: Ref,
    tree: Ref,
    pdf: Pdf,
    pages: Vec<Ref>,
    objects: objects::Objects,
    images: HashMap<(i32, i32, String), Ref>,
    glyphs: HashSet<(usvg::fontdb::ID, u16)>,
    seed: String,
    clusters: HashSet<String>,
    seed_ready: bool,
    prepared: usize,
    attachment: Option<(String, Vec<u8>)>,
}
impl Default for PdfBuilder {
    fn default() -> Self {
        Self::new()
    }
}
impl PdfBuilder {
    pub fn new() -> Self {
        let mut alloc = Ref::new(1);
        let catalog = alloc.bump();
        let tree = alloc.bump();
        Self {
            alloc,
            catalog,
            tree,
            pdf: Pdf::new(),
            pages: Vec::new(),
            objects: objects::Objects::default(),
            images: HashMap::new(),
            glyphs: HashSet::new(),
            seed: String::new(),
            clusters: HashSet::new(),
            seed_ready: false,
            prepared: 0,
            attachment: None,
        }
    }
    pub fn len(&self) -> usize {
        self.pages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
    /// Preflight all pages before push: every layer uses the same glyph subset
    /// and CID assignment, so its font objects can be shared across the document.
    pub fn prepare(&mut self, svg: &str) -> Result<()> {
        let doc = roxmltree::Document::parse(svg).map_err(|e| Error::Invalid(e.to_string()))?;
        let opening = &svg[..svg.find('>').unwrap() + 1];
        let vector = without_images(svg, doc.root_element());
        let tree = usvg::Tree::from_str(&format!("{opening}{vector}</svg>"), options())
            .map_err(|e| Error::Invalid(e.to_string()))?;
        self.font_samples(tree.root());
        self.prepared += 1;
        Ok(())
    }
    pub fn prepared(&self) -> usize {
        self.prepared
    }
    fn font_samples(&mut self, group: &usvg::Group) {
        for node in group.children() {
            match node {
                usvg::Node::Text(text) => {
                    self.clusters.extend(
                        text.layouted()
                            .iter()
                            .flat_map(|s| &s.positioned_glyphs)
                            .map(|g| g.text.clone()),
                    );
                    let glyphs: Vec<_> = text
                        .layouted()
                        .iter()
                        .flat_map(|s| &s.positioned_glyphs)
                        .map(|g| (g.font, g.id.0))
                        .collect();
                    if glyphs.iter().any(|g| !self.glyphs.contains(g)) {
                        for chunk in text.chunks() {
                            let text = chunk
                                .text()
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;");
                            self.seed
                                .push_str(&format!("<text font-size=\"1\">{text}</text>"));
                        }
                        self.glyphs.extend(glyphs);
                    }
                }
                usvg::Node::Group(group) => self.font_samples(group),
                _ => (),
            }
        }
    }
    fn compact_seed(&mut self) -> Result<()> {
        if self.seed_ready {
            return Ok(());
        }
        // Most sheets need only a short alphabet of shaping clusters. Verify
        // the actual glyph ids, retaining the full-context samples for scripts
        // whose glyph selection depends on surrounding text.
        let mut clusters: Vec<_> = self.clusters.iter().map(String::as_str).collect();
        clusters.sort_unstable();
        let text = clusters
            .join(" ")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let seed = format!("<text font-size=\"1\">{text}</text>");
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{seed}</svg>"#
        );
        let tree =
            usvg::Tree::from_str(&svg, options()).map_err(|e| Error::Invalid(e.to_string()))?;
        fn glyphs(group: &usvg::Group, out: &mut HashSet<(usvg::fontdb::ID, u16)>) {
            for node in group.children() {
                match node {
                    usvg::Node::Text(t) => out.extend(
                        t.layouted()
                            .iter()
                            .flat_map(|s| &s.positioned_glyphs)
                            .map(|g| (g.font, g.id.0)),
                    ),
                    usvg::Node::Group(g) => glyphs(g, out),
                    _ => (),
                }
            }
        }
        let mut covered = HashSet::new();
        glyphs(tree.root(), &mut covered);
        if self.glyphs.is_subset(&covered) {
            self.seed = seed;
        }
        self.seed_ready = true;
        Ok(())
    }
    pub fn attach(&mut self, name: &str, bytes: Vec<u8>) {
        self.attachment = Some((name.into(), bytes));
    }
    pub fn push(&mut self, svg: &str) -> Result<()> {
        self.compact_seed()?;
        let doc = roxmltree::Document::parse(svg).map_err(|e| Error::Invalid(e.to_string()))?;
        let root = doc.root_element();
        let mm = |key| {
            root.attribute(key)
                .and_then(|v| v.trim_end_matches("mm").parse::<f32>().ok())
        };
        let (wmm, hmm) = (
            mm("width").ok_or_else(|| Error::Invalid("missing page width".into()))?,
            mm("height").ok_or_else(|| Error::Invalid("missing page height".into()))?,
        );
        let r = |v: f32| (v * 72.0 / 25.4 * 100.0).round() / 100.0;
        let (wpt, hpt) = (r(wmm), r(hmm));
        let opening = &svg[..svg.find('>').unwrap() + 1];
        let defs: String = root
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "defs")
            .map(|n| &svg[n.range()])
            .collect();
        let mut layers = Vec::<(Ref, [f32; 6])>::new();
        let mut vector = String::new();
        // Our piece wrapper carries one affine matrix. Keep direct PNG reuse
        // (and layer order) for its images instead of decoding them in svg2pdf.
        let mut nodes = Vec::new();
        for node in root.children().filter(|n| n.is_element()) {
            if node.attribute("data-piece").is_some() {
                if let Some(matrix) = node.attribute("transform").and_then(piece_matrix) {
                    nodes.extend(
                        node.children()
                            .filter(|n| n.is_element())
                            .map(|child| (child, Some(matrix))),
                    );
                    continue;
                }
            }
            nodes.push((node, None));
        }
        for (node, transform) in nodes {
            if node.tag_name().name() == "defs" && transform.is_none() {
                continue;
            }
            let png = if node.tag_name().name() == "image"
                && node.attribute("preserveAspectRatio") == Some("none")
            {
                node.attribute(("http://www.w3.org/1999/xlink", "href"))
                    .or_else(|| node.attribute("href"))
                    .and_then(|v| v.strip_prefix("data:image/png;base64,"))
                    .and_then(png_stream)
            } else {
                None
            };
            if let Some((w, h, data)) = png {
                if !vector.is_empty() {
                    layers.push((
                        self.vector(&format!("{opening}{defs}{vector}</svg>"))?,
                        [wpt, 0.0, 0.0, hpt, 0.0, 0.0],
                    ));
                    vector.clear();
                }
                let key = (w, h, crate::util::sha256_hex(&data));
                let id = if let Some(&id) = self.images.get(&key) {
                    id
                } else {
                    let id = self.alloc.bump();
                    let mut image = self.pdf.image_xobject(id, &data);
                    image.width(w).height(h).bits_per_component(8);
                    image.color_space().device_gray();
                    image.filter(pdf_writer::Filter::FlateDecode);
                    image
                        .decode_parms()
                        .predictor(pdf_writer::types::Predictor::PngOptimum)
                        .colors(1)
                        .bits_per_component(8)
                        .columns(w);
                    image.finish();
                    self.images.insert(key, id);
                    id
                };
                let attr = |key| {
                    node.attribute(key)
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(0.0)
                };
                let (x, y, w, h) = (attr("x"), attr("y"), attr("width"), attr("height"));
                layers.push((
                    id,
                    image_matrix(
                        transform.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                        [x, y, w, h],
                        [wmm, hmm],
                        [wpt, hpt],
                    ),
                ));
            } else {
                if let Some(m) = transform {
                    vector.push_str(&format!(
                        "<g transform=\"matrix({} {} {} {} {} {})\">",
                        m[0], m[1], m[2], m[3], m[4], m[5]
                    ));
                    vector.push_str(&svg[node.range()]);
                    vector.push_str("</g>");
                } else {
                    vector.push_str(&svg[node.range()]);
                }
            }
        }
        if !vector.is_empty() {
            layers.push((
                self.vector(&format!("{opening}{defs}{vector}</svg>"))?,
                [wpt, 0.0, 0.0, hpt, 0.0, 0.0],
            ));
        }
        let page_id = self.alloc.bump();
        let content_id = self.alloc.bump();
        let mut content = Content::new();
        {
            let mut page = self.pdf.page(page_id);
            page.media_box(Rect::new(0.0, 0.0, wpt, hpt));
            page.parent(self.tree);
            page.contents(content_id);
            let mut resources = page.resources();
            let mut objects = resources.x_objects();
            for (i, (id, matrix)) in layers.iter().enumerate() {
                let name = format!("L{i}");
                objects.pair(Name(name.as_bytes()), *id);
                content
                    .save_state()
                    .transform(*matrix)
                    .x_object(Name(name.as_bytes()))
                    .restore_state();
            }
        }
        self.pdf.stream(content_id, &content.finish());
        self.pages.push(page_id);
        Ok(())
    }
    fn vector(&mut self, svg: &str) -> Result<Ref> {
        // Hidden samples participate in font subsetting, but never paint. Put
        // them before and after the layer for stable CID and Unicode mappings.
        let at = svg.find('>').unwrap() + 1;
        let seed = format!("<g visibility=\"hidden\">{}</g>", self.seed);
        let svg = format!(
            "{}{}{}{}{}",
            &svg[..at],
            seed,
            &svg[at..svg.len() - 6],
            seed,
            "</svg>"
        );
        let tree =
            usvg::Tree::from_str(&svg, options()).map_err(|e| Error::Invalid(e.to_string()))?;
        let (chunk, id) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default())
            .map_err(|e| Error::Invalid(e.to_string()))?;
        self.objects
            .import(&mut self.pdf, &mut self.alloc, &chunk, id)
    }
    pub fn finish(mut self) -> Vec<u8> {
        let attachment = self.attachment.take().map(|(name, bytes)| {
            let data = self.alloc.bump();
            let spec = self.alloc.bump();
            self.pdf
                .embedded_file(data, &bytes)
                .subtype(Name(b"application/json"))
                .params()
                .size(bytes.len() as i32);
            self.pdf
                .file_spec(spec)
                .path(pdf_writer::Str(name.as_bytes()))
                .unic_file(pdf_writer::TextStr(&name))
                .embedded_file_with_unicode(data);
            (name, spec)
        });
        let mut catalog = self.pdf.catalog(self.catalog);
        catalog.pages(self.tree);
        if let Some((name, spec)) = &attachment {
            catalog
                .names()
                .embedded_files()
                .names()
                .insert(pdf_writer::Str(name.as_bytes()), *spec);
        }
        catalog.finish();
        self.pdf
            .pages(self.tree)
            .kids(self.pages.iter().copied())
            .count(self.pages.len() as i32);
        self.pdf.finish()
    }
}
fn png_stream(encoded: &str) -> Option<(i32, i32, Vec<u8>)> {
    let mut bytes = Vec::with_capacity(encoded.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in encoded.bytes().take_while(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits) as u8);
        }
    }
    if bytes.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut at = 8;
    let mut size = None;
    let mut data = Vec::new();
    while at + 12 <= bytes.len() {
        let n = u32::from_be_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
        let chunk = bytes.get(at + 8..at + 8 + n)?;
        match &bytes[at + 4..at + 8] {
            b"IHDR" => {
                if chunk.len() != 13 || chunk[8..] != [8, 0, 0, 0, 0] {
                    return None;
                }
                size = Some((
                    i32::from_be_bytes(chunk[..4].try_into().ok()?),
                    i32::from_be_bytes(chunk[4..8].try_into().ok()?),
                ));
            }
            b"IDAT" => data.extend_from_slice(chunk),
            _ => (),
        }
        at += 12 + n;
    }
    let (w, h) = size?;
    if w <= 0 || h <= 0 || data.is_empty() {
        return None;
    }
    Some((w, h, data))
}
/// Converts SVG pages (sizes in mm) to a PDF, retaining image pixels exactly.
pub fn pdf_from_svgs(pages: &[String]) -> Result<Vec<u8>> {
    let mut builder = PdfBuilder::new();
    for page in pages {
        builder.prepare(page)?;
    }
    for page in pages {
        builder.push(page)?;
    }
    Ok(builder.finish())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_idat_is_reused_without_reencoding() {
        let px: Vec<_> = (0..120).map(|i| (i * 13) as u8).collect();
        let png = crate::image::encode_png_gray(12, 10, &px);
        let (w, h, idat) = png_stream(&crate::layout::base64(&png)).unwrap();
        assert_eq!((w, h), (12, 10));
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20mm" height="20mm" viewBox="0 0 20 20"><rect width="20" height="20" fill="white"/><image x="1" y="2" width="12" height="10" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/><path d="M1 2h12" stroke="black"/></svg>"#,
            crate::layout::base64(&png)
        );
        let pdf = pdf_from_svgs(&[svg.clone(), svg]).unwrap();
        assert!(pdf.windows(idat.len()).any(|v| v == idat));
        assert!(pdf.starts_with(b"%PDF"));
        assert_eq!(
            pdf.windows(b"/Subtype /Image".len())
                .filter(|v| *v == b"/Subtype /Image")
                .count(),
            1
        );
    }
    #[test]
    fn all_pages_share_one_font_and_registration_is_an_embedded_file() {
        let pages: Vec<_> = ["office 10 µm", "affinity 29 Ω", "Grain 47", "Échantillon 83", "عينة 52"].iter().map(|text| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><text x="2" y="10" font-size="4">{text}</text></svg>"#)).collect();
        let mut pdf = PdfBuilder::new();
        for page in &pages {
            pdf.prepare(page).unwrap();
        }
        for page in &pages {
            pdf.push(page).unwrap();
        }
        let json = br#"{"spots":[{"id":"registered"}]}"#;
        pdf.attach("print_registration.json", json.to_vec());
        let bytes = pdf.finish();
        let count = |key: &[u8]| bytes.windows(key.len()).filter(|v| *v == key).count();
        assert_eq!(count(b"/Subtype /Type0"), 1);
        assert_eq!(count(b"/FontFile2"), 1);
        assert_eq!(count(b"/EmbeddedFiles"), 1);
        assert_eq!(count(json), 1);
    }
}

fn piece_matrix(value: &str) -> Option<[f32; 6]> {
    let values: Vec<_> = value
        .strip_prefix("matrix(")?
        .strip_suffix(')')?
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    values.try_into().ok()
}
/// Same affine as the SVG, converting downwards sheet mm to upwards PDF pt.
fn image_matrix(m: [f32; 6], r: [f32; 4], mm: [f32; 2], pt: [f32; 2]) -> [f32; 6] {
    let [x, y, w, h] = r;
    let sx = pt[0] / mm[0];
    let sy = pt[1] / mm[1];
    [
        m[0] * w * sx,
        -m[1] * w * sy,
        -m[2] * h * sx,
        m[3] * h * sy,
        (m[0] * x + m[2] * (y + h) + m[4]) * sx,
        (mm[1] - m[1] * x - m[3] * (y + h) - m[5]) * sy,
    ]
}
fn without_images(svg: &str, root: roxmltree::Node) -> String {
    let mut out = String::new();
    let mut at = root.range().start;
    for node in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "image")
    {
        let r = node.range();
        out.push_str(&svg[at..r.start]);
        at = r.end;
    }
    out.push_str(&svg[at..root.range().end]);
    // Caller supplies the root opening and closing tags.
    out[out.find('>').unwrap() + 1..out.len() - 6].into()
}

#[cfg(test)]
mod arrange_tests {
    use super::*;
    #[test]
    fn rotated_rescaled_png_pdf_geometry_equals_screen_and_reuses_pixels() {
        use crate::layout::sheets::arrange::point;
        let (sin, cos) = 23.125_f64.to_radians().sin_cos();
        let m = [
            cos * 0.73,
            sin * 0.73,
            -sin * 0.73,
            cos * 0.73,
            83.12345,
            91.65432,
        ];
        let r = [12.345, 17.654, 50.123, 28.456];
        let mm = [431.8, 279.4];
        let pt = [1224.0, 792.0];
        let pdf = image_matrix(m.map(|v| v as f32), r, mm, pt);
        for uv in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.3, 0.8]] {
            let screen = point(
                m,
                [
                    (r[0] + uv[0] * r[2]) as f64,
                    (r[1] + (1.0 - uv[1]) * r[3]) as f64,
                ],
            );
            let from_pdf = [
                (pdf[0] * uv[0] + pdf[2] * uv[1] + pdf[4]) * mm[0] / pt[0],
                mm[1] - (pdf[1] * uv[0] + pdf[3] * uv[1] + pdf[5]) * mm[1] / pt[1],
            ];
            for d in 0..2 {
                assert!((screen[d] - from_pdf[d] as f64).abs() * 8.0 < 0.01);
            }
        }
        let png = crate::image::encode_png_gray(12, 10, &[128; 120]);
        let (_, _, idat) = png_stream(&crate::layout::base64(&png)).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="431.8mm" height="279.4mm" viewBox="0 0 431.8 279.4"><g data-piece="test" transform="matrix({} {} {} {} {} {})"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/><text x="10" y="10" font-size="3">100 µm</text></g></svg>"#,
            m[0],
            m[1],
            m[2],
            m[3],
            m[4],
            m[5],
            r[0],
            r[1],
            r[2],
            r[3],
            crate::layout::base64(&png)
        );
        let bytes = pdf_from_svgs(&[svg]).unwrap();
        assert!(bytes.windows(idat.len()).any(|b| b == idat));
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("/Subtype /Image").count(), 1);
        let matrices: Vec<Vec<f32>> = text
            .lines()
            .filter(|line| line.ends_with(" cm"))
            .filter_map(|line| {
                line.trim_end_matches(" cm")
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()
                    .ok()
            })
            .collect();
        assert!(
            matrices.iter().any(
                |mat| mat.len() == 6 && mat.iter().zip(pdf).all(|(a, b)| (a - b).abs() < 0.001)
            ),
            "PDF paints the same affine matrix"
        );
    }
}

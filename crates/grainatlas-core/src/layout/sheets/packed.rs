//! Deterministic MaxRects placement of whole strip blocks, without rotation or scaling.
use super::SheetBlock;

const GAP: f64 = 2.5;
const EPS: f64 = 1e-7;

pub(super) fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0].max(b[0]) < a[2].min(b[2]) - EPS && a[1].max(b[1]) < a[3].min(b[3]) - EPS
}

fn contains(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[0] + EPS && a[1] <= b[1] + EPS && a[2] >= b[2] - EPS && a[3] >= b[3] - EPS
}

struct Page { free: Vec<[f64; 4]> }
impl Page {
    fn new(b: [f64; 4]) -> Self { Self { free: vec![[b[0], b[1], b[2] + GAP, b[3] + GAP]] } }
    fn take(&mut self, r: [f64; 4]) {
        let r = [r[0], r[1], r[2] + GAP, r[3] + GAP];
        let mut split = Vec::new();
        for f in self.free.drain(..) {
            if !overlaps(f, r) { split.push(f); continue; }
            if r[0] > f[0] + EPS { split.push([f[0], f[1], r[0], f[3]]); }
            if r[1] > f[1] + EPS { split.push([f[0], f[1], f[2], r[1]]); }
            if r[2] < f[2] - EPS { split.push([r[2], f[1], f[2], f[3]]); }
            if r[3] < f[3] - EPS { split.push([f[0], r[3], f[2], f[3]]); }
        }
        // MaxRects free rectangles overlap each other; discard contained ones only.
        self.free = split.iter().enumerate().filter(|(i, a)| !split.iter().enumerate().any(|(j, b)| i != &j && contains(*b, **a) && (!contains(**a, *b) || j < *i))).map(|(_, r)| *r).collect();
    }
    fn position(&self, size: [f64; 2]) -> Option<[f64; 4]> {
        self.free.iter().filter(|r| r[2] - r[0] + EPS >= size[0] + GAP && r[3] - r[1] + EPS >= size[1] + GAP)
            .min_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])))
            .map(|r| [r[0], r[1], r[0] + size[0], r[1] + size[1]])
    }
}

/// Pack sizes in mount order around rectangles already taken on 1-based sheets.
/// `bodies[0]` is page 1; `bodies[1]` applies to every later page (sheet mm).
/// Returned blocks retain input order. The 2.5 mm gap is between blocks only.
/// Four trials use mount order without/with back-fill, tallest first, widest first;
/// the first trial wins ties. Oversize blocks reserve a sheet of their own and
/// extend beyond its body so the registration bounds audit can report them.
pub fn pack_rectangles(sizes: &[[f64; 2]], bodies: [[f64; 4]; 2], taken: &[SheetBlock]) -> Vec<SheetBlock> {
    let trial = |order: &[usize], backfill: bool| {
        let mut pages: Vec<Page> = (0..taken.iter().map(|r| r.sheet).max().unwrap_or(0)).map(|i| Page::new(bodies[usize::from(i > 0)])).collect();
        for r in taken { pages[r.sheet - 1].take(r.rect_mm); }
        let mut placed = vec![SheetBlock { sheet: 0, rect_mm: [0.0; 4] }; sizes.len()];
        let mut cursor = 0;
        for &i in order {
            let size = sizes[i];
            let start = if backfill { 0 } else { cursor };
            let mut found = (start..pages.len()).find_map(|pi| pages[pi].position(size).map(|r| (pi, r)));
            if found.is_none() {
                let mut pi = pages.len();
                let b = bodies[usize::from(pi > 0)];
                // A block may fit the compact body but not the first body.
                let fits = |b: [f64; 4]| size[0] <= b[2] - b[0] + EPS && size[1] <= b[3] - b[1] + EPS;
                if pi == 0 && !fits(b) && fits(bodies[1]) { pages.push(Page::new(b)); pi += 1; }
                let b = bodies[usize::from(pi > 0)];
                let mut page = Page::new(b);
                let r = [b[0], b[1], b[0] + size[0], b[1] + size[1]];
                if !fits(b) { page.free.clear(); }
                pages.push(page);
                found = Some((pi, r));
            }
            let (pi, r) = found.expect("new page always places the block");
            pages[pi].take(r);
            cursor = pi;
            placed[i] = SheetBlock { sheet: pi + 1, rect_mm: r };
        }
        (pages.len(), placed)
    };
    let mount: Vec<_> = (0..sizes.len()).collect();
    let mut best = trial(&mount, false);
    for mode in 1..4 {
        let mut order = mount.clone();
        if mode >= 2 {
            let d = usize::from(mode == 2);
            order.sort_by(|&a, &b| sizes[b][d].total_cmp(&sizes[a][d]).then(a.cmp(&b)));
        }
        let candidate = trial(&order, true);
        if candidate.0 < best.0 { best = candidate; }
    }
    best.1
}

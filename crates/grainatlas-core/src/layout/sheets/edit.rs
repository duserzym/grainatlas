//! Ordered, pixel-free strip edits. Keys survive placement; split siblings retain a
//! parent snapshot for exact joins. Child counters increase within their parent's
//! area/hash family, in edit order, and are never reused during one replay.
use super::*;
use std::collections::{BTreeSet, BTreeMap};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StripEdit {
    /// An absent origin chooses the first free place on the zero-based sheet.
    Move { strip: StripKey, sheet: usize, #[serde(default)] origin_mm: Option<[f64; 2]> },
    AddSheet { at: usize },
    RemoveSheet { sheet: usize },
    MoveSheet { from: usize, to: usize },
    Crop { band: StripKey, mosaic_rect_um: [f64; 4] },
    Split { band: StripKey, axis: Axis, at_um: f64 },
    Join { band_a: StripKey, band_b: StripKey },
    Separate { strip: StripKey },
    Label { strip: StripKey, text: String },
    Hide { strip: StripKey },
    Show { strip: StripKey },
    Repack {},
    Arrange {},
    Transform { pieces: Vec<arrange::PiecePlacement> },
    AutoArrange { strips: Vec<StripKey> },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Axis { X, Y }
/// Resolved scale when the edits were made; zero means unknown in older edit lists.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintEdits { #[serde(default = "legacy_version", deserialize_with = "print_version", skip_serializing_if = "is_legacy_version")] pub version: u32, pub params: SheetParams, #[serde(default)] pub um_per_mm: f64, pub edits: Vec<StripEdit> }
fn legacy_version() -> u32 { 1 }
fn is_legacy_version(v: &u32) -> bool { *v == 1 }
fn print_version<'de,D:serde::Deserializer<'de>>(d:D)->Result<u32,D::Error> {
    let v=u32::deserialize(d)?;
    if v==1 || v==2 {Ok(v)} else {Err(serde::de::Error::custom("unsupported print arrangement version"))}
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DroppedEdit { pub index: usize, pub reason: String }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind { Overlap, OutsideBody, GrainCut, PrintedTwice }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Problem { pub kind: ProblemKind, pub strips: Vec<StripKey>, pub message: String }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LeftOff { #[serde(flatten)] pub grain: StrayGrain, pub reason: String }
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LeftOffSummary { pub cropped: usize, pub hidden: usize }
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EditSummary {
    pub moves: usize, pub crops: usize, pub splits: usize, pub joins: usize,
    pub separations: usize, pub label_changes: usize, pub hidden_strips: usize,
    pub sheets_added: usize, pub sheets_removed: usize, pub sheets_reordered: usize, pub repacks: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EditedPlan {
    pub plan: SheetPlan, pub applied: usize, pub dropped: Vec<DroppedEdit>,
    pub problems: Vec<Problem>, pub left_off: Vec<LeftOff>, pub summary: EditSummary,
}

/// One source of block geometry for rendering, packing, snapping and validation.
/// A shared-label lines strip reserves its divider to the left without moving pixels.
pub fn strip_block(s: &SheetStrip) -> SheetBlock {
    if s.transform.is_some() {
        let size = arrange::size(s);
        return SheetBlock { sheet:s.sheet+1,rect_mm:[s.origin_mm[0],s.origin_mm[1],s.origin_mm[0]+size[0],s.origin_mm[1]+size[1]] };
    }
    SheetBlock { sheet: s.sheet + 1, rect_mm: [s.origin_mm[0] - if s.labelled { 0.0 } else { PIECE_GAP_MM / 2.0 }, s.origin_mm[1], s.origin_mm[0] + s.size_mm[0], s.origin_mm[1] + s.size_mm[1]] }
}
pub(super) fn note_height(s: &SheetStrip) -> f64 {
    if s.neighbours.is_empty() { 0.0 } else { NOTE_MM }
}
pub(super) fn measure(s: &mut SheetStrip, k: f64) {
    let mut width: f64 = 0.0;
    let mut height: f64 = 0.0;
    for b in &s.bands {
        let end = b.offset_mm + (b.mosaic_rect_um[2] - b.mosaic_rect_um[0]) / k;
        width = width.max(end);
        height = height.max((b.mosaic_rect_um[3] - b.mosaic_rect_um[1]) / k);
    }
    s.size_mm = [width, LABEL_MM + height + note_height(s)];
    block_extent(s, k);
}
pub(super) fn block_extent(s: &mut SheetStrip, k: f64) {
    if s.labelled { s.size_mm[0] = s.size_mm[0].max(20.0).max(text_w(&s.sample, 3.2, true) + 6.0 + text_w(&s.label, 2.5, false)); }
    for (i, b) in s.bands.iter().enumerate() {
        if let Some(gap) = b.gap_after_um {
            let mark = text_w("//", 2.4, false).max(text_w(&format!("{:.1} mm", gap / 1000.0), 1.7, false));
            s.size_mm[0] = s.size_mm[0].max(b.offset_mm + (b.mosaic_rect_um[2] - b.mosaic_rect_um[0]) / k + PIECE_GAP_MM / 2.0 + if i + 1 == s.bands.len() && s.shares_gap_with.is_some() { 0.0 } else { mark / 2.0 });
        }
    }
}
fn clear_shared_gaps(plan: &mut SheetPlan, keys: &BTreeSet<StripKey>) {
    for s in &mut plan.strips {
        if s.shares_gap_with.as_ref().is_some_and(|next| keys.contains(&s.key) || keys.contains(next)) {
            s.shares_gap_with = None; measure(s, plan.um_per_mm);
        }
    }
}
fn relabel(s: &mut SheetStrip, k: f64) { s.shares_gap_with = None; s.labelled = true; measure(s, k); }
fn reflow(s: &mut SheetStrip, k: f64) {
    let mut x = 0.0;
    for b in &mut s.bands { b.offset_mm = x; x += (b.mosaic_rect_um[2] - b.mosaic_rect_um[0]) / k + PIECE_GAP_MM; }
    measure(s, k);
}
fn body(plan: &SheetPlan, i: usize) -> SheetBody {
    let m = plan.params.margin_mm;
    let header = plan.cut_stats.header_mm[usize::from(i > 0)];
    SheetBody { sheet: i + 1, header_mm: header, body_rect_mm: [m, m + header + 1.0, plan.paper_mm[0] - m, plan.paper_mm[1] - m - FOOTER_MM] }
}
fn bodies(plan: &mut SheetPlan, n: usize) { plan.sheet_bodies = (0..n).map(|i| body(plan, i)).collect(); }
fn locate(plan: &SheetPlan, key: &StripKey) -> Result<(usize, usize), String> {
    plan.strips.iter().enumerate().find_map(|(i, s)| s.bands.iter().position(|b| &b.key == key).map(|j| (i, j))).ok_or_else(missing)
}
fn strip(plan: &SheetPlan, key: &StripKey) -> Result<usize, String> { plan.strips.iter().position(|s| &s.key == key).ok_or_else(missing) }
fn missing() -> String { "strip no longer exists (grains or context changed)".into() }
fn piece<'g>(b: &SheetBand, gs: &'g [Grain]) -> Piece<'g> {
    Piece { grains: b.grains.iter().filter_map(|id| gs.iter().find(|g| g.id == *id)).collect(), x: b.offset_mm, rect: b.mosaic_rect_um, core: b.core_um, cuts: b.cuts.clone(), cut_sources: b.cut_sources.clone(), gap_after_um: b.gap_after_um }
}
fn clipped(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}
fn crosses(rect: [f64; 4], box_um: [f64; 4]) -> bool { intersects(rect, box_um) && !contains(rect, box_um) }

struct SplitRecord { strip_key: StripKey, children: [StripKey; 2], parent: SheetBand, size: [f64; 2], offsets: Vec<(StripKey, f64)> }
struct State {
    touched: BTreeSet<StripKey>, edited: BTreeSet<StripKey>,
    original: BTreeMap<StripKey, Vec<[f64; 4]>>, splits: Vec<SplitRecord>,
    // Preserve the automatic plan's resolved samples, including area aliases.
    samples: BTreeMap<(String, uuid::Uuid), String>,
    counters: BTreeMap<(String, String), u32>, reasons: BTreeMap<(String, uuid::Uuid), String>,
}
impl State {
    fn same_sample(&self, area: &str, id: uuid::Uuid, sample: &str) -> bool {
        self.samples.get(&(area.to_string(), id)).is_some_and(|s| s == sample)
    }
    fn touch(&mut self, s: &SheetStrip) { self.touched.insert(s.key.clone()); self.touched.extend(s.bands.iter().map(|b| b.key.clone())); }
    fn child(&mut self, key: &StripKey) -> Result<StripKey, String> {
        let n = self.counters.entry((key.area.clone(), key.grains_hash.clone())).or_insert(key.split);
        *n = n.checked_add(1).ok_or("split key limit reached")?;
        Ok(StripKey { split: *n, ..key.clone() })
    }
}

/// Apply only to a freshly made automatic plan. Every refused edit leaves it unchanged.
pub fn apply_edits(plan: &SheetPlan, edits: &[StripEdit], areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>) -> EditedPlan {
    let mut plan = plan.clone();
    let mut state = State { touched: BTreeSet::new(), edited: BTreeSet::new(), original: BTreeMap::new(), splits: Vec::new(), samples: BTreeMap::new(), counters: BTreeMap::new(), reasons: BTreeMap::new() };
    for s in &plan.strips {
        let gs: Vec<_> = grains[&s.area].iter().filter(|g| plan.targets[&s.area].contains(&g.id)).collect();
        for b in &s.bands {
            for id in &b.grains { state.samples.insert((s.area.clone(), *id), s.sample.clone()); }
            state.original.insert(b.key.clone(), b.repeated_um.clone().unwrap_or_else(|| piece(b, &grains[&s.area]).repeated(&gs, plan.params.pad_um)));
            let count = state.counters.entry((b.key.area.clone(), b.key.grains_hash.clone())).or_default();
            *count = (*count).max(b.key.split);
        }
    }
    let mut summary = EditSummary::default();
    let mut applied = 0;
    let mut dropped = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        // The first edit uses compact geometry even for first-free placement and
        // Repack. A refused edit restores the byte-identical automatic Lines plan.
        let automatic = plan.lines.is_some().then(|| plan.clone());
        if automatic.is_some() {
            plan.cut_stats.header_mm = compact::heights(&plan);
            let count = plan.sheet_bodies.len(); bodies(&mut plan, count);
            for s in &mut plan.strips { measure(s, plan.um_per_mm); }
        }
        match apply_one(&mut plan, edit, areas, grains, &mut state, &mut summary) {
            Err(reason) => {
                if let Some(original) = automatic { plan = original; }
                dropped.push(DroppedEdit { index, reason });
            },
            Ok(()) => {
                applied += 1;
                clear_shared_gaps(&mut plan, &state.touched);
                plan.lines = None;
                plan.edit_summary = Some(summary.clone());
                reconcile(&mut plan, grains, &mut state);
                refresh_notes(&mut plan);
                plan.cut_stats.header_mm = compact::heights(&plan);
                let count = plan.sheet_bodies.len(); bodies(&mut plan, count);
            }
        }
    }
    // Validation uses the renderer's exact mapping and shared audit, without pixels.
    let r = render_plan_with_images(&plan, areas, grains, &Ownership::default(), &|_, _| (Vec::new(), "kept".into()));
    EditedPlan { left_off: plan.left_off.clone(), plan, applied, dropped, problems: r.problems, summary }
}

fn apply_one(plan: &mut SheetPlan, edit: &StripEdit, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>, state: &mut State, summary: &mut EditSummary) -> Result<(), String> {
    let k = plan.um_per_mm;
    let n = plan.sheet_bodies.len();
    match edit {
        StripEdit::Arrange {} => {
            // A packed run may contain several image bands separated by a gap.
            // Each becomes its own paper object, retaining continuation context.
            let runs=plan.strips.iter().filter(|s|!s.hidden && s.bands.len()>1).map(|s|s.key.clone()).collect::<Vec<_>>();
            for strip in runs { apply_one(plan,&StripEdit::Separate{strip},areas,grains,state,summary)?; }
            let keys = plan.strips.iter().filter(|s| !s.hidden).map(|s|s.key.clone()).collect::<Vec<_>>();
            auto_arrange(plan,&keys)?;
            summary.repacks += 1;
        }
        StripEdit::AutoArrange { strips } => { auto_arrange(plan,strips)?; summary.repacks += 1; }
        StripEdit::Transform { pieces } => {
            if pieces.is_empty() { return Err("select a piece".into()); }
            let mut candidate = plan.clone();
            let mut seen = BTreeSet::new();
            for p in pieces {
                if !seen.insert(p.strip.clone()) || p.sheet>=n || !p.scale.is_finite() || p.scale<=0.0 || !p.rotation_deg.is_finite() || p.origin_mm.iter().any(|v| !v.is_finite()) { return Err("invalid piece transform".into()); }
                let i = strip(&candidate,&p.strip)?;
                let s = &mut candidate.strips[i];
                s.sheet=p.sheet; s.origin_mm=p.origin_mm;
                s.transform=Some(arrange::PieceTransform {scale:p.scale,rotation_deg:p.rotation_deg.rem_euclid(360.0)});
                s.shares_gap_with=None; s.labelled=true;
            }
            refresh_notes(&mut candidate);
            candidate.cut_stats.header_mm=compact::heights(&candidate); bodies(&mut candidate,n);
            if candidate.strips.iter().filter(|s|!s.hidden && !seen.contains(&s.key)).any(|s|s.origin_mm[1]<candidate.sheet_bodies[s.sheet].body_rect_mm[1]-1e-7) {return Err("Shared scale key needs more header space; move the other pieces down first".into());}
            if candidate.strips.iter().filter(|s|seen.contains(&s.key)).any(|s|s.bands.iter().any(|b|arrange::band_transform(s,b,k).paper_to_mount.iter().any(|v|!v.is_finite()))) { return Err("invalid piece transform".into()); }
            // Clamp a selection as one rigid group, preserving its relative positions.
            for sheet in pieces.iter().map(|p|p.sheet).collect::<BTreeSet<_>>() {
                let body=candidate.sheet_bodies[sheet].body_rect_mm;
                let blocks:Vec<_>=candidate.strips.iter().filter(|s|s.sheet==sheet && seen.contains(&s.key)).map(|s|strip_block(s).rect_mm).collect();
                let r=[blocks.iter().map(|r|r[0]).fold(f64::INFINITY,f64::min),blocks.iter().map(|r|r[1]).fold(f64::INFINITY,f64::min),blocks.iter().map(|r|r[2]).fold(f64::NEG_INFINITY,f64::max),blocks.iter().map(|r|r[3]).fold(f64::NEG_INFINITY,f64::max)];
                if r[2]-r[0]>body[2]-body[0]+1e-7 || r[3]-r[1]>body[3]-body[1]+1e-7 { return Err("piece group does not fit below the header".into()); }
                let delta=[(body[0]-r[0]).max((body[2]-r[2]).min(0.0)),(body[1]-r[1]).max((body[3]-r[3]).min(0.0))];
                for s in candidate.strips.iter_mut().filter(|s|s.sheet==sheet && seen.contains(&s.key)) { for d in 0..2 {s.origin_mm[d]+=delta[d];} }
            }
            // Refuse the whole group atomically. Bounding boxes intentionally keep
            // clearance around rotated corners and labels as well as their pixels.
            for a in candidate.strips.iter().filter(|s|seen.contains(&s.key) && !s.hidden) {
                for b in candidate.strips.iter().filter(|s|!s.hidden && s.sheet==a.sheet && s.key!=a.key) {
                    if packed::overlaps(strip_block(a).rect_mm,strip_block(b).rect_mm) { return Err("Pieces overlap; drop refused".into()); }
                }
            }
            for s in candidate.strips.iter().filter(|s|seen.contains(&s.key)) { state.touch(s); }
            *plan=candidate; summary.moves += pieces.len();
        }
        StripEdit::Move { strip: key, sheet, origin_mm } => {
            let i = strip(plan, key)?;
            if *sheet >= n || origin_mm.is_some_and(|xy| xy.iter().any(|v| !v.is_finite())) { return Err("invalid sheet or origin".into()); }
            // Measure the independent strip and both sides of any broken shared gap
            // before packing. A refused thumbnail drop leaves the plan unchanged.
            let mut moved = plan.clone();
            clear_shared_gaps(&mut moved, &BTreeSet::from([key.clone()]));
            relabel(&mut moved.strips[i], k);
            moved.strips[i].sheet = *sheet;
            if moved.lines.is_some() { for s in &mut moved.strips { measure(s, k); } }
            refresh_notes(&mut moved);
            let origin = if let Some(xy) = origin_mm { *xy } else {
                let b = moved.sheet_bodies[*sheet].body_rect_mm;
                let taken: Vec<_> = moved.strips.iter().enumerate().filter(|(j, s)| *j != i && !s.hidden && s.sheet == *sheet).map(|(_, s)| SheetBlock { sheet: 1, ..strip_block(s) }).collect();
                let block = packed::pack_rectangles(&[moved.strips[i].size_mm], [b, b], &taken).remove(0);
                if block.sheet != 1 || !contains(b, block.rect_mm) { return Err(format!("no room on sheet {}", sheet + 1)); }
                [block.rect_mm[0], block.rect_mm[1]]
            };
            let s = &mut moved.strips[i]; state.touch(s); s.origin_mm = origin;
            *plan = moved; summary.moves += 1;
        }
        StripEdit::AddSheet { at } => {
            if *at > n { return Err("invalid sheet".into()); }
            for s in &mut plan.strips { if s.sheet >= *at { s.sheet += 1; } }
            bodies(plan, n + 1); summary.sheets_added += 1;
        }
        StripEdit::RemoveSheet { sheet } => {
            if *sheet >= n { return Err("invalid sheet".into()); }
            if plan.strips.iter().any(|s| s.sheet == *sheet && !s.hidden) { return Err("sheet is not empty".into()); }
            if n == 1 { return Err("cannot remove the last sheet".into()); }
            for s in &mut plan.strips { if s.sheet > *sheet { s.sheet -= 1; } else if s.sheet == *sheet { s.sheet = (*sheet).min(n - 2); } }
            bodies(plan, n - 1); summary.sheets_removed += 1;
        }
        StripEdit::MoveSheet { from, to } => {
            if *from >= n || *to >= n { return Err("invalid sheet".into()); }
            for s in &mut plan.strips {
                s.sheet = if s.sheet == *from { *to } else if from < to && s.sheet > *from && s.sheet <= *to { s.sheet - 1 } else if from > to && s.sheet >= *to && s.sheet < *from { s.sheet + 1 } else { s.sheet };
            }
            bodies(plan, n); summary.sheets_reordered += 1;
        }
        StripEdit::Crop { band, mosaic_rect_um } => {
            let (i, j) = locate(plan, band)?;
            let a = areas.iter().find(|a| a.rec.id == band.area).ok_or_else(missing)?;
            let mut rect = *mosaic_rect_um;
            if rect.iter().any(|v| !v.is_finite()) { return Err("invalid crop rectangle".into()); }
            for d in 0..2 { let limit = [a.img.w, a.img.h][d] as f64 * a.img.um_per_px; rect[d] = rect[d].clamp(0.0, limit); rect[d + 2] = rect[d + 2].clamp(0.0, limit); }
            if rect[0] >= rect[2] || rect[1] >= rect[3] { return Err("empty crop rectangle".into()); }
            for g in &grains[&band.area] { if plan.targets[&band.area].contains(&g.id) && crosses(rect, g.bbox_um) { return Err(format!("would cut grain {}", g.number)); } }
            let s = &mut plan.strips[i]; state.touch(s);
            s.bands[j].mosaic_rect_um = rect;
            state.edited.insert(band.clone());
            for id in &s.bands[j].grains { state.reasons.insert((s.area.clone(), *id), "cropped".into()); }
            reflow(s, k); summary.crops += 1;
        }
        StripEdit::Split { band, axis, at_um } => {
            let (i, j) = locate(plan, band)?;
            let a = areas.iter().find(|a| a.rec.id == band.area).ok_or_else(missing)?;
            let d = usize::from(*axis == Axis::Y);
            if !at_um.is_finite() { return Err("invalid split coordinate".into()); }
            let gs: Vec<_> = grains[&band.area].iter().filter(|g| plan.targets[&band.area].contains(&g.id)).collect();
            let b = &plan.strips[i].bands[j];
            for g in &gs { if g.bbox_um[d] < *at_um && *at_um < g.bbox_um[d + 2] && (g.bbox_um[1-d] < b.mosaic_rect_um[3-d] && g.bbox_um[3-d] > b.mosaic_rect_um[1-d]) { return Err(format!("would cut grain {}", g.number)); } }
            let r = crate::segment::filter::reference_size(&gs.iter().map(|g| (*g).clone()).collect::<Vec<_>>());
            let c = if plan.params.context_um > 0.0 { plan.params.context_um } else { (3.0 * r).max(plan.params.gap_break_um) };
            let overlap = if plan.params.overlap_um > 0.0 { plan.params.overlap_um } else { r.max(c / 3.0) };
            let cut = CutContext { r, c, overlap, pad: plan.params.pad_um, min_group: plan.params.min_group, optional_min_group: 1, source: if d == 0 { "width" } else { "height_line" }, max_repeated_fraction: plan.params.max_repeated_fraction, grains: &gs };
            let q = piece(b, &grains[&band.area]);
            let (left, right) = split_at(&q, d, *at_um, &cut, &a.img).ok_or("split outside band core")?;
            if left.grains.is_empty() || right.grains.is_empty() { return Err("split would leave a side with no grain".into()); }
            let parent = b.clone();
            if state.counters.get(&(band.area.clone(), band.grains_hash.clone())).copied().unwrap_or(band.split) > u32::MAX - 2 { return Err("split key limit reached".into()); }
            let keys = [state.child(band)?, state.child(band)?];
            let children: Vec<_> = [left, right].into_iter().zip(&keys).map(|(q, key)| SheetBand { key: key.clone(), offset_mm: 0.0, grains: q.grains.iter().map(|g| g.id).collect(), mosaic_rect_um: q.rect, core_um: q.core, cuts: q.cuts, cut_sources: q.cut_sources, gap_after_um: None, repeated_um: None }).collect();
            for b in &children {
                let mut original: Vec<_> = state.original[band].iter().filter_map(|r| clipped(*r, b.mosaic_rect_um)).collect();
                original.extend(piece(b, &grains[&band.area]).repeated(&gs, plan.params.pad_um));
                state.original.insert(b.key.clone(), original); state.edited.insert(b.key.clone()); state.touched.insert(b.key.clone());
            }
            let s = &mut plan.strips[i]; state.touch(s);
            state.splits.push(SplitRecord { strip_key: s.key.clone(), children: keys, parent, size: s.size_mm, offsets: s.bands.iter().map(|b| (b.key.clone(), b.offset_mm)).collect() });
            s.bands.splice(j..j+1, children); reflow(s, k); summary.splits += 1;
        }
        StripEdit::Join { band_a, band_b } => {
            let (i, a) = locate(plan, band_a)?; let (ib, b) = locate(plan, band_b)?;
            if plan.strips[i].area != plan.strips[ib].area || plan.strips[i].sample != plan.strips[ib].sample { return Err("bands belong to different areas or samples".into()); }
            let pos = state.splits.iter().position(|r| (r.children[0] == *band_a && r.children[1] == *band_b) || (r.children[1] == *band_a && r.children[0] == *band_b)).ok_or("bands are not direct split siblings")?;
            let record = state.splits.remove(pos);
            let parent_key = record.parent.key.clone();
            let parent_ids: BTreeSet<_> = record.parent.grains.iter().copied().collect();
            if i == ib {
                let s = &mut plan.strips[i]; state.touch(s);
                let lo = a.min(b); let hi = a.max(b);
                s.bands.remove(hi); s.bands[lo] = record.parent;
                for band in &mut s.bands { if let Some((_, x)) = record.offsets.iter().find(|(key, _)| key == &band.key) { band.offset_mm = *x; } }
                s.size_mm = record.size; block_extent(s, k);
            } else {
                // Separated siblings join at the first child's image origin. Other
                // bands and strips keep their places; notes now refer to the parent.
                let (first, fj) = locate(plan, &record.children[0])?;
                let (second, sj) = locate(plan, &record.children[1])?;
                state.touch(&plan.strips[first]); state.touch(&plan.strips[second]);
                let x = plan.strips[first].origin_mm[0] + plan.strips[first].bands[fj].offset_mm;
                let mut parent = record.parent; parent.offset_mm = 0.0;
                plan.strips[first].bands[fj] = parent;
                if plan.strips[first].bands.len() == 1 {
                    plan.strips[first].key = record.strip_key.clone(); plan.strips[first].origin_mm[0] = x;
                }
                plan.strips[second].bands.remove(sj);
                let parent_key = plan.strips[first].key.clone();
                if plan.strips[second].bands.is_empty() { plan.strips.remove(second); }
                for s in &mut plan.strips {
                    for neighbour in &mut s.neighbours { if record.children.contains(neighbour) { *neighbour = parent_key.clone(); } }
                    s.neighbours.retain(|key| key != &s.key); s.neighbours.dedup();
                }
                if let Some(s) = plan.strips.iter_mut().find(|s| s.key == parent_key) { measure(s, k); }
            }
            // Restoring the parent restores its owners, including any temporarily
            // adopted elsewhere while a child was cropped or hidden.
            let (pi, pj) = locate(plan, &parent_key)?;
            for (si, s) in plan.strips.iter_mut().enumerate() {
                if s.area == band_a.area { for (bi, b) in s.bands.iter_mut().enumerate() { if si != pi || bi != pj { b.grains.retain(|id| !parent_ids.contains(id)); } } }
            }
            state.edited.remove(band_a); state.edited.remove(band_b); state.edited.insert(parent_key); state.touch(&plan.strips[pi]); summary.joins += 1;
        }
        StripEdit::Separate { strip: key } => {
            let i = strip(plan, key)?;
            let old = plan.strips[i].clone(); state.touch(&old);
            let keys: Vec<_> = old.bands.iter().map(|b| b.key.clone()).collect();
            let mut pieces = Vec::new();
            for (j, mut b) in old.bands.clone().into_iter().enumerate() {
                let x = old.origin_mm[0] + b.offset_mm; b.offset_mm = 0.0; b.gap_after_um = None;
                let mut s = SheetStrip { key: b.key.clone(), bands: vec![b], origin_mm: [x, old.origin_mm[1]], neighbours: keys.iter().enumerate().filter(|(n, _)| n.abs_diff(j) == 1).map(|(_, k)| k.clone()).collect(), ..old.clone() };
                relabel(&mut s, k); state.touch(&s); pieces.push(s);
            }
            plan.strips.splice(i..i+1, pieces); summary.separations += 1;
        }
        StripEdit::Label { strip: key, text } => {
            let i = strip(plan, key)?; let s = &mut plan.strips[i]; state.touch(s); s.label = text.clone(); relabel(s, k); summary.label_changes += 1;
        }
        StripEdit::Hide { strip: key } | StripEdit::Show { strip: key } => {
            let i = strip(plan, key)?;
            let hide = matches!(edit, StripEdit::Hide { .. });
            state.touch(&plan.strips[i]);
            if hide { for b in &plan.strips[i].bands { for id in &b.grains { state.reasons.insert((key.area.clone(), *id), "hidden".into()); } } }
            else {
                let ids: BTreeSet<_> = plan.strips[i].bands.iter().flat_map(|b| &b.grains).copied().collect();
                for (j, s) in plan.strips.iter_mut().enumerate() { if j != i && s.area == key.area { for b in &mut s.bands { b.grains.retain(|id| !ids.contains(id)); } } }
            }
            plan.strips[i].hidden = hide; summary.hidden_strips += usize::from(hide);
        }
        StripEdit::Repack {} => {
            let pinned = |s: &SheetStrip| state.touched.contains(&s.key) || s.bands.iter().any(|b| state.touched.contains(&b.key));
            let indices: Vec<_> = plan.strips.iter().enumerate().filter(|(_, s)| !s.hidden && !pinned(s)).map(|(i, _)| i).collect();
            clear_shared_gaps(plan, &indices.iter().map(|&i| plan.strips[i].key.clone()).collect());
            let taken: Vec<_> = plan.strips.iter().filter(|s| !s.hidden && pinned(s)).map(strip_block).collect();
            for &i in &indices { relabel(&mut plan.strips[i], k); }
            let sizes: Vec<_> = indices.iter().map(|&i| plan.strips[i].size_mm).collect();
            let placed = packed::pack_rectangles(&sizes, [body(plan, 0).body_rect_mm, body(plan, 1).body_rect_mm], &taken);
            let count = placed.iter().chain(&taken).map(|b| b.sheet).max().unwrap_or(1).max(1);
            for (&i, block) in indices.iter().zip(placed) { plan.strips[i].sheet = block.sheet - 1; plan.strips[i].origin_mm = [block.rect_mm[0], block.rect_mm[1]]; }
            for s in &mut plan.strips { if s.hidden { s.sheet = s.sheet.min(count - 1); } }
            bodies(plan, count); summary.repacks += 1;
        }
    }
    Ok(())
}

fn reconcile(plan: &mut SheetPlan, grains: &BTreeMap<String, Vec<Grain>>, state: &mut State) {
    let mut owned = BTreeSet::new();
    for s in &mut plan.strips {
        if s.hidden { continue; }
        for b in &mut s.bands {
            b.grains.retain(|id| state.same_sample(&s.area, *id, &s.sample) && grains[&s.area].iter().find(|g| g.id == *id).is_some_and(|g| contains(b.mosaic_rect_um, g.bbox_um)) && owned.insert((s.area.clone(), *id)));
        }
    }
    plan.left_off.clear();
    for (area, ids) in &plan.targets {
        for id in ids {
            if owned.contains(&(area.clone(), *id)) { continue; }
            let Some(g) = grains.get(area).and_then(|gs| gs.iter().find(|g| g.id == *id)) else { continue; };
            if let Some(b) = plan.strips.iter_mut().filter(|s| !s.hidden && &s.area == area && state.same_sample(area, *id, &s.sample)).flat_map(|s| &mut s.bands).find(|b| contains(b.mosaic_rect_um, g.bbox_um)) {
                b.grains.push(*id); owned.insert((area.clone(), *id));
            } else {
                plan.left_off.push(LeftOff { grain: StrayGrain { area: area.clone(), id: *id, number: g.number, label: g.label.clone(), eq_diameter_um: g.eq_diameter_um, centroid_um: g.centroid_um, bbox_um: g.bbox_um }, reason: state.reasons.get(&(area.clone(), *id)).cloned().unwrap_or_else(|| "cropped".into()) });
            }
        }
    }
    let cores: Vec<_> = plan.strips.iter().filter(|s| !s.hidden).flat_map(|s| s.bands.iter().map(|b| (s.area.clone(), b.key.clone(), b.core_um))).collect();
    for s in &mut plan.strips {
        for b in &mut s.bands {
            let base = state.original.get(&b.key).cloned().unwrap_or_default();
            let repeated = if state.edited.contains(&b.key) {
                let mut strips: Vec<_> = base.into_iter().filter_map(|r| clipped(r, b.mosaic_rect_um)).collect();
                for extra in cores.iter().filter(|(a, key, _)| a == &s.area && key != &b.key).filter_map(|(_, _, r)| clipped(*r, b.mosaic_rect_um)) {
                    let before = rect_union_area(&strips); strips.push(extra);
                    if rect_union_area(&strips) <= before + 1e-7 { strips.pop(); }
                }
                strips
            } else { base };
            let gs: Vec<_> = grains[&s.area].iter().filter(|g| plan.targets[&s.area].contains(&g.id)).collect();
            b.repeated_um = (piece(b, &grains[&s.area]).repeated(&gs, plan.params.pad_um) != repeated).then_some(repeated);
        }
    }
}

/// Ignore display/header parameters. Report every cutting or placement difference by name.
pub fn replay(plan: &SheetPlan, stored: &PrintEdits, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>) -> EditedPlan {
    if stored.version != 1 && stored.version != 2 {
        let mut result = apply_edits(plan,&[],areas,grains);
        result.dropped = stored.edits.iter().enumerate().map(|(index,_)|DroppedEdit {index,reason:"unsupported print arrangement version".into()}).collect();
        return result;
    }
    let old = serde_json::to_value(&stored.params).expect("parameters serialize");
    let new = serde_json::to_value(&plan.params).expect("parameters serialize");
    let fields = ["paper", "landscape", "margin_mm", "um_per_mm", "target_grain_mm", "pad_um", "gap_break_um", "context_um", "min_group", "min_cluster_grains", "overlap_um", "min_cut_grains", "max_repeated_fraction", "min_grain_um", "max_grain_um", "packing"];
    let changed: Vec<_> = fields.into_iter().filter(|key| old[key] != new[key]).collect();
    let scale_changed = stored.um_per_mm > 0.0 && (stored.um_per_mm - plan.um_per_mm).abs() > 1e-9 * stored.um_per_mm.abs().max(plan.um_per_mm.abs());
    let reason = if scale_changed { Some(format!("parameters changed (scale {} → {} µm/mm)", stored.um_per_mm, plan.um_per_mm)) }
        else if !changed.is_empty() { Some(format!("parameters changed ({})", changed.join(", "))) } else { None };
    if let Some(reason) = reason {
        let mut result = apply_edits(plan, &[], areas, grains);
        result.dropped = stored.edits.iter().enumerate().map(|(index, _)| DroppedEdit { index, reason: reason.clone() }).collect(); result
    } else { apply_edits(plan, &stored.edits, areas, grains) }
}

/// Problems are counted with the audit's image and block granularity, including both
/// when a strip's image and its block violate the same boundary or overlap.
pub(super) fn problems(plan: &SheetPlan, reg: &PrintRegistration, printed: &[Vec<&Grain>], areas: &[SheetArea]) -> Vec<Problem> {
    let mut out = Vec::new();
    let strips: Vec<_> = plan.strips.iter().filter(|s| !s.hidden).collect();
    let mut ordered = strips.clone();
    ordered.sort_by(|a, b| a.sheet.cmp(&b.sheet).then_with(|| if plan.lines.is_some() { std::cmp::Ordering::Equal } else { a.origin_mm[1].total_cmp(&b.origin_mm[1]).then(a.origin_mm[0].total_cmp(&b.origin_mm[0])) }));
    let band_keys: Vec<_> = ordered.iter().flat_map(|s| s.bands.iter().map(|_| s.key.clone())).collect();
    let owner = |b: &BandRecord| reg.bands.iter().position(|a| std::ptr::eq(a, b)).and_then(|i| band_keys.get(i)).cloned();
    let mut push = |kind, keys: Vec<StripKey>, message: &str| out.push(Problem { kind, strips: keys, message: message.into() });
    for (i, a) in reg.bands.iter().enumerate() {
        let keys: Vec<_> = owner(a).into_iter().collect();
        let body = reg.sheet_bodies.iter().find(|s| s.sheet == a.sheet).map_or(reg.body_rect_mm, |s| s.body_rect_mm);
        if !contains(body, a.sheet_rect_mm) || a.sheet_rect_mm.iter().any(|v| !v.is_finite()) { push(ProblemKind::OutsideBody, keys.clone(), "band outside sheet body"); }
        for b in &reg.bands[i+1..] { if a.sheet == b.sheet && packed::overlaps(a.sheet_rect_mm, b.sheet_rect_mm) { push(ProblemKind::Overlap, keys.iter().cloned().chain(owner(b)).collect(), "band images overlap"); } }
    }
    if plan.lines.is_none() {
        for (i, s) in strips.iter().enumerate() {
            let block = strip_block(s);
            let b = reg.sheet_bodies.iter().find(|b| b.sheet == block.sheet).map_or(reg.body_rect_mm, |b| b.body_rect_mm);
            if !contains(b, block.rect_mm) || block.rect_mm.iter().any(|v| !v.is_finite()) { push(ProblemKind::OutsideBody, vec![s.key.clone()], "strip outside sheet body"); }
            if covers_header(reg, &block) { push(ProblemKind::Overlap, vec![s.key.clone()], "strip covers sheet header"); }
            for other in &strips[i+1..] { if s.sheet == other.sheet && packed::overlaps(block.rect_mm, strip_block(other).rect_mm) { push(ProblemKind::Overlap, vec![s.key.clone(), other.key.clone()], "strip blocks overlap"); } }
        }
    }
    if plan.lines.is_some() {
        let blocks = legacy_blocks(reg, areas);
        for (i, a) in blocks.iter().enumerate() { for b in &blocks[i+1..] {
            if a.sheet == b.sheet && packed::overlaps(a.rect_mm, b.rect_mm) {
                let keys = strips.iter().filter(|s| s.sheet + 1 == a.sheet && (packed::overlaps(strip_block(s).rect_mm, a.rect_mm) || packed::overlaps(strip_block(s).rect_mm, b.rect_mm))).map(|s| s.key.clone()).collect();
                push(ProblemKind::Overlap, keys, "strip blocks overlap");
            }
        } }
    }
    for (ai, gs) in printed.iter().enumerate() {
        for g in gs {
            let owners: Vec<_> = reg.bands.iter().filter(|b| b.area == areas[ai].rec.id && b.grains.iter().any(|own| own.id == g.id)).collect();
            let keys: Vec<_> = owners.iter().filter_map(|b| owner(b)).collect();
            if (owners.is_empty() && !reg.left_off.iter().any(|off| off.grain.id == g.id && off.grain.area == areas[ai].rec.id)) || owners.iter().any(|b| !contains(b.mosaic_rect_um, g.bbox_um)) { push(ProblemKind::GrainCut, keys.clone(), "target grain has no whole owner band"); }
            if owners.len() > 1 { push(ProblemKind::PrintedTwice, keys, "target grain owned more than once"); }
        }
    }
    out
}

pub(super) fn covers_header(reg: &PrintRegistration, block: &SheetBlock) -> bool {
    let top = reg.sheet_bodies.iter().find(|b| b.sheet == block.sheet).map_or(reg.body_rect_mm[1], |b| b.body_rect_mm[1]);
    packed::overlaps(block.rect_mm, [0.0, 0.0, reg.paper_mm[0], top - 1.0])
}

/// Snap a block edge to its body or a 2.5 mm gap from another shown block.
/// The tolerance is in paper mm; callers bypass this helper while Alt is held.
pub fn snap_origin(plan: &SheetPlan, key: &StripKey, sheet: usize, origin: [f64; 2], tolerance_mm: f64) -> Option<[f64; 2]> {
    let s = plan.strips.iter().find(|s| &s.key == key)?;
    let mut moved = s.clone(); moved.origin_mm = origin;
    let r = strip_block(&moved).rect_mm;
    let b = plan.sheet_bodies.get(sheet)?.body_rect_mm;
    let mut result = origin;
    for d in 0..2 {
        let mut shifts = vec![b[d] - r[d], b[d+2] - r[d+2]];
        for other in plan.strips.iter().filter(|s| !s.hidden && s.sheet == sheet && &s.key != key) {
            let rect = strip_block(other).rect_mm;
            shifts.extend([rect[d] - 2.5 - r[d+2], rect[d+2] + 2.5 - r[d]]);
        }
        if let Some(shift) = shifts.into_iter().filter(|s| s.abs() <= tolerance_mm).min_by(|a, b| a.abs().total_cmp(&b.abs())) { result[d] += shift; }
    }
    Some(result)
}

pub(super) fn continuation_note(plan: &SheetPlan, strip: &SheetStrip) -> String {
    strip.neighbours.iter().filter_map(|key| plan.strips.iter().find(|s| &s.key == key && !s.hidden)).map(|s| format!("continues on sheet {}", s.sheet + 1)).collect::<Vec<_>>().join(" · ")
}
fn refresh_notes(plan: &mut SheetPlan) {
    let widths: Vec<_> = plan.strips.iter().map(|s| text_w(&continuation_note(plan,s),1.9,false)).collect();
    for (s, width) in plan.strips.iter_mut().zip(widths) {
        if !s.neighbours.is_empty() || s.transform.is_some() {
            let reserved=if s.transform.is_some() {s.size_mm[0]} else {0.0};
            measure(s, plan.um_per_mm); s.size_mm[0] = s.size_mm[0].max(width).max(reserved);
            if s.transform.is_some() {arrange::references(s,plan.um_per_mm,&plan.params.spot_sizes_um);}
        }
    }
}

/// Same MaxRects algorithm and ordering as Automatic, with fixed pieces reserved.
fn auto_arrange(plan: &mut SheetPlan, keys: &[StripKey]) -> Result<(),String> {
    let mut next = plan.clone();
    let mut indices = Vec::new();
    for key in keys { let i=strip(&next,key)?; if !next.strips[i].hidden && !indices.contains(&i) { indices.push(i); } }
    for &i in &indices {
        let s=&mut next.strips[i]; s.transform=Some(arrange::PieceTransform::default());
        s.shares_gap_with=None; s.labelled=true;
    }
    clear_shared_gaps(&mut next,&keys.iter().cloned().collect());
    refresh_notes(&mut next);
    // Packing can allocate one new page per piece. Reserve the widest page
    // number now, then keep that width during subsequent moves and resizes.
    let page_limit=next.sheet_bodies.len()+next.strips.len();
    for &i in &indices {
        let s=&mut next.strips[i];
        let note=vec![format!("continues on sheet {page_limit}");s.neighbours.len()].join(" · ");
        s.size_mm[0]=s.size_mm[0].max(text_w(&note,1.9,false));
    }
    next.cut_stats.header_mm=compact::heights(&next);
    let taken:Vec<_>=next.strips.iter().enumerate().filter(|(i,s)|!s.hidden && !indices.contains(i)).map(|(_,s)|strip_block(s)).collect();
    if taken.iter().any(|b|b.rect_mm[1]<body(&next,b.sheet-1).body_rect_mm[1]-1e-7) {return Err("Shared scale key needs more header space; move the other pieces down first".into());}
    let sizes:Vec<_>=indices.iter().map(|&i|next.strips[i].size_mm).collect();
    let placed=packed::pack_rectangles(&sizes,[body(&next,0).body_rect_mm,body(&next,1).body_rect_mm],&taken);
    let count=placed.iter().chain(&taken).map(|b|b.sheet).max().unwrap_or(1).max(1);
    for (&i,b) in indices.iter().zip(placed) { next.strips[i].sheet=b.sheet-1; next.strips[i].origin_mm=[b.rect_mm[0],b.rect_mm[1]]; }
    bodies(&mut next,count);
    for s in &mut next.strips { s.sheet=s.sheet.min(count-1); }
    *plan=next; Ok(())
}

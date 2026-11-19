//! Frozen pre-export reference from origin/main bfa87ee9c621.
//! Test-only; do not share helpers with the production blend.
//! See LICENSE-photocraft-MIT for the upstream copyright and permission notice.
//! Sparse seam grids, preserving photocraft-algo 81bd973's DP arithmetic and tie order.
//! Adapted from panorama/blend.rs (MIT OR Apache-2.0), by its contributors.
//! Only whole covered coarse cells are stored; canvas holes have no luma allocation.
pub(super) struct Grid {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub stride: usize,
    pub px: Vec<f32>,
}
impl Grid {
    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        (self.y..self.y + self.h)
            .flat_map(move |y| (self.x..self.x + self.w).map(move |x| y * self.stride + x))
    }
}
impl std::ops::Index<usize> for Grid {
    type Output = f32;
    fn index(&self, i: usize) -> &f32 {
        let (x, y) = (i % self.stride, i / self.stride);
        if x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h {
            &self.px[(y - self.y) * self.w + x - self.x]
        } else {
            &0.0
        }
    }
}
pub(super) fn seam_labels(
    w: usize,
    h: usize,
    luma: &[Grid],
    order: &[usize],
    ctl: &crate::Ctl,
) -> crate::Result<Vec<i32>> {
    let mut label = vec![-1i32; w * h];
    for &img in order {
        ctl.report("seams", img as f32 / order.len().max(1) as f32);
        ctl.check()?;
        let cov = &luma[img];
        let mut overlap = vec![false; w * h];
        let (mut sx, mut sy, mut no) = (0.0f64, 0.0f64, 0usize);
        let (mut nx, mut ny, mut nn) = (0.0f64, 0.0f64, 0usize);
        for i in cov.indices() {
            nx += (i % w) as f64;
            ny += (i / w) as f64;
            nn += 1;
            if label[i] >= 0 {
                overlap[i] = true;
                sx += (i % w) as f64;
                sy += (i / w) as f64;
                no += 1;
            } else {
                label[i] = img as i32;
            }
        }
        if no == 0 {
            continue;
        }
        let (cx, cy) = (nx / nn as f64, ny / nn as f64);
        let contained = nn == no;
        let (ox, oy) = if contained {
            // Wholly inside the composite (#5745: two photos in one document, the second moved
            // by Auto-Align so the canvas crops it to the part over the first). Its side of the
            // seam is the one it lies towards from the composite's centre; an image centred on
            // the composite has no side, and the earlier images keep it.
            let (mut px, mut py, mut pn) = (0.0f64, 0.0f64, 0usize);
            for (i, l) in label.iter().enumerate() {
                if *l >= 0 {
                    px += (i % w) as f64;
                    py += (i / w) as f64;
                    pn += 1;
                }
            }
            let (px, py) = (px / pn.max(1) as f64, py / pn.max(1) as f64);
            if (cx - px).abs() < 0.5 && (cy - py).abs() < 0.5 {
                continue;
            }
            (px, py)
        } else {
            (sx / no as f64, sy / no as f64)
        };
        let vertical = (cx - ox).abs() >= (cy - oy).abs();
        // Image on the increasing side along the seam's cross axis?
        let positive = if vertical { cx > ox } else { cy > oy };
        // A contained image has no pixels of its own, so a seam on its outer edge would leave it
        // nothing: break cost ties away from its side (towards larger k when it lies on the
        // decreasing side) so it keeps its side of the overlap.
        let prefer_high = contained && !positive;
        let cost = |i: usize| -> f32 { 0.002 + (luma[img][i] - luma[label[i] as usize][i]).abs() };
        let (lines, len) = if vertical { (h, w) } else { (w, h) };
        let (line_start, line_end, cross_start, cross_end) = if vertical {
            (cov.y, cov.y + cov.h, cov.x, cov.x + cov.w)
        } else {
            (cov.x, cov.x + cov.w, cov.y, cov.y + cov.h)
        };
        let idx = |line: usize, k: usize| if vertical { line * w + k } else { k * w + line };
        // DP over lines (rows for a vertical seam), within the overlap.
        let mut acc = vec![f32::INFINITY; w * h];
        let mut from = vec![usize::MAX; w * h];
        let mut prev_line: Option<usize> = None;
        for line in line_start..line_end {
            let any = (cross_start..cross_end).any(|k| overlap[idx(line, k)]);
            if !any {
                continue;
            }
            for k in cross_start..cross_end {
                let i = idx(line, k);
                if !overlap[i] {
                    continue;
                }
                let mut best = (0.0f32, usize::MAX);
                if let Some(pl) = prev_line.filter(|pl| *pl + 1 == line) {
                    best = (f32::INFINITY, usize::MAX);
                    let dks = if prefer_high {
                        [1i64, 0, -1]
                    } else {
                        [-1i64, 0, 1]
                    };
                    for dk in dks {
                        let kk = k as i64 + dk;
                        if kk < 0 || kk >= len as i64 {
                            continue;
                        }
                        let j = idx(pl, kk as usize);
                        if acc[j] < best.0 {
                            best = (acc[j], kk as usize);
                        }
                    }
                    if best.1 == usize::MAX {
                        best = (0.0, usize::MAX);
                    }
                }
                acc[i] = best.0 + cost(i);
                from[i] = best.1;
            }
            prev_line = Some(line);
        }
        // Backtrack from the cheapest end; lines not reached keep their own best cell.
        let mut seam = vec![usize::MAX; lines];
        let mut line = line_end;
        while line > line_start {
            line -= 1;
            if seam[line] != usize::MAX {
                continue;
            }
            let Some(k) = (cross_start..cross_end)
                .filter(|&k| overlap[idx(line, k)])
                .min_by(|&a, &b| {
                    acc[idx(line, a)]
                        .total_cmp(&acc[idx(line, b)])
                        .then(if prefer_high { b.cmp(&a) } else { a.cmp(&b) })
                })
            else {
                continue;
            };
            let (mut l, mut k) = (line, k);
            loop {
                seam[l] = k;
                let f = from[idx(l, k)];
                if f == usize::MAX || l == 0 {
                    break;
                }
                l -= 1;
                if seam[l] != usize::MAX {
                    break;
                }
                k = f;
            }
        }
        for line in line_start..line_end {
            let s = seam[line];
            if s == usize::MAX {
                continue;
            }
            let on_side = |k: usize| if positive { k > s } else { k < s };
            // A contained image has no other cells, so where the overlap has nothing on its side
            // of the seam (a line one cell wide) it takes the seam cell rather than nothing.
            let take_seam =
                contained && !(cross_start..cross_end).any(|k| overlap[idx(line, k)] && on_side(k));
            for k in cross_start..cross_end {
                let i = idx(line, k);
                if overlap[i] && (on_side(k) || (take_seam && k == s)) {
                    label[i] = img as i32;
                }
            }
        }
    }
    ctl.report("seams", 1.0);
    Ok(label)
}

//! Binary morphology (square structuring elements, separable running counts),
//! hole filling, connected components and the Euclidean distance transform.

/// Dilation with a (2r+1) square.
pub fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    box_any(m, w, h, r, true)
}

/// Erosion with a (2r+1) square.
pub fn erode(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let inv: Vec<bool> = m.iter().map(|v| !v).collect();
    box_any(&inv, w, h, r, true).into_iter().map(|v| !v).collect()
}

pub fn open(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    dilate(&erode(m, w, h, r), w, h, r)
}

pub fn close(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    erode(&dilate(m, w, h, r), w, h, r)
}

fn box_any(m: &[bool], w: usize, h: usize, r: usize, _any: bool) -> Vec<bool> {
    if r == 0 {
        return m.to_vec();
    }
    // Horizontal pass: count of set pixels in the window.
    let mut tmp = vec![false; w * h];
    crate::util::par_rows(&mut tmp, w, |y, row| {
        let src = &m[y * w..(y + 1) * w];
        let mut c: i64 = 0;
        for &v in src.iter().take(r.min(w)) {
            c += v as i64;
        }
        for x in 0..w {
            if x + r < w {
                c += src[x + r] as i64;
            }
            if x > r {
                c -= src[x - r - 1] as i64;
            }
            row[x] = c > 0;
        }
    });
    // Vertical pass on columns.
    let mut out = vec![false; w * h];
    let mut cnt = vec![0i64; w];
    for y in 0..r.min(h) {
        for x in 0..w {
            cnt[x] += tmp[y * w + x] as i64;
        }
    }
    for y in 0..h {
        if y + r < h {
            for x in 0..w {
                cnt[x] += tmp[(y + r) * w + x] as i64;
            }
        }
        if y > r {
            for x in 0..w {
                cnt[x] -= tmp[(y - r - 1) * w + x] as i64;
            }
        }
        for x in 0..w {
            out[y * w + x] = cnt[x] > 0;
        }
    }
    out
}

/// Fills holes: background regions not connected to the border become foreground.
pub fn fill_holes(m: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut outside = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        for y in [0, h - 1] {
            let i = y * w + x;
            if !m[i] && !outside[i] {
                outside[i] = true;
                stack.push(i);
            }
        }
    }
    for y in 0..h {
        for x in [0, w - 1] {
            let i = y * w + x;
            if !m[i] && !outside[i] {
                outside[i] = true;
                stack.push(i);
            }
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let mut push = |j: usize| {
            if !m[j] && !outside[j] {
                outside[j] = true;
                stack.push(j);
            }
        };
        if x > 0 {
            push(i - 1);
        }
        if x + 1 < w {
            push(i + 1);
        }
        if y > 0 {
            push(i - w);
        }
        if y + 1 < h {
            push(i + w);
        }
    }
    outside.iter().map(|o| !o).collect()
}

/// 4-connected components: labels (0 = background, 1..=n) and pixel counts per label (index 0 unused).
pub fn components(m: &[bool], w: usize, h: usize) -> (Vec<u32>, Vec<usize>) {
    let mut lab = vec![0u32; w * h];
    let mut counts = vec![0usize];
    let mut stack = Vec::new();
    for s in 0..w * h {
        if !m[s] || lab[s] != 0 {
            continue;
        }
        let l = counts.len() as u32;
        counts.push(0);
        lab[s] = l;
        stack.push(s);
        while let Some(i) = stack.pop() {
            counts[l as usize] += 1;
            let (x, y) = (i % w, i / w);
            let nb = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in nb.into_iter().flatten() {
                if m[j] && lab[j] == 0 {
                    lab[j] = l;
                    stack.push(j);
                }
            }
        }
    }
    (lab, counts)
}

/// Exact Euclidean distance transform (Felzenszwalb and Huttenlocher, 2012): distance of each
/// foreground pixel to the nearest background pixel.
pub fn distance(m: &[bool], w: usize, h: usize) -> Vec<f32> {
    const INF: f32 = 1e20;
    let mut f = vec![0f32; w * h];
    for i in 0..w * h {
        f[i] = if m[i] { INF } else { 0.0 };
    }
    // Columns.
    let mut col = vec![0f32; h];
    let mut out = vec![0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = f[y * w + x];
        }
        dt1(&col, &mut out);
        for y in 0..h {
            f[y * w + x] = out[y];
        }
    }
    // Rows.
    crate::util::par_rows(&mut f, w, |_, row| {
        let src = row.to_vec();
        let mut o = vec![0f32; w];
        dt1(&src, &mut o);
        for (r, v) in row.iter_mut().zip(o) {
            *r = v.sqrt();
        }
    });
    f
}

fn dt1(f: &[f32], d: &mut [f32]) {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0f32; n + 1];
    let mut k = 0usize;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0 and s <= -inf cannot happen; fallthrough guard.
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for (q, d) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        *d = (q as f32 - p as f32) * (q as f32 - p as f32) + f[p];
    }
}

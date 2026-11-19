//! Compact headers for packed and edited placement. Legacy Lines drawing stays in sheets.rs.
use super::*;

/// All coordinates are relative to the header's top-left. Shared by measurement
/// and drawing, so wrapped acquisition values cannot consume the body.
struct HeaderText {
    x: f64,
    y: f64,
    size: f64,
    bold: bool,
    text: String,
}

pub(super) fn ellipsis(text: &str, width: f64, size: f64, bold: bool) -> String {
    if text_w(text, size, bold) <= width {
        return text.to_string();
    }
    let count = (width / text_w("x", size, bold)).floor().max(1.0) as usize;
    text.chars()
        .take(count.saturating_sub(1))
        .chain(['…'])
        .collect()
}

fn wrap_header(text: &str, width: f64, size: f64) -> Vec<String> {
    let count = (width / text_w("x", size, false)).floor().max(1.0) as usize;
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        if !lines.last().unwrap().is_empty() {
            if lines.last().unwrap().chars().count() + 1 + word.chars().count() <= count {
                lines.last_mut().unwrap().push(' ');
            } else {
                lines.push(String::new());
            }
        }
        for c in word.chars() {
            if lines.last().unwrap().chars().count() == count {
                lines.push(String::new());
            }
            lines.last_mut().unwrap().push(c);
        }
    }
    lines
}

fn packed_grid(kv: &[(&str, String)], width: f64) -> (Vec<HeaderText>, f64) {
    let mut out = Vec::new();
    let mut y = 11.4;
    for row in 0..5 {
        let indices: Vec<_> = if row < 3 {
            vec![2 * row, 2 * row + 1]
        } else {
            vec![row + 3]
        };
        let mut height = 1;
        for (col, i) in indices.into_iter().enumerate() {
            let x = col as f64 * width / 2.0;
            let available = if i < 6 {
                width / 2.0 - 18.0
            } else {
                width - 16.0
            };
            let lines = wrap_header(&kv[i].1, available, 2.0);
            height = height.max(lines.len());
            out.push(HeaderText {
                x,
                y,
                size: 1.8,
                bold: false,
                text: kv[i].0.to_uppercase(),
            });
            out.extend(
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(line, text)| HeaderText {
                        x: x + 16.0,
                        y: y + line as f64 * 2.6,
                        size: 2.0,
                        bold: false,
                        text,
                    }),
            );
        }
        y += height as f64 * 2.6;
    }
    (out, y)
}

struct HeaderKey {
    circles: Vec<(f64, f64, f64, f64)>,
    text: Vec<HeaderText>,
    height: f64,
}
fn packed_key(p: &SheetParams, k: f64, width: f64) -> HeaderKey {
    let (sizes, off) = key_spots(&p.spot_sizes_um, k);
    let mut key = HeaderKey {
        circles: Vec::new(),
        text: vec![HeaderText {
            x: 0.0,
            y: 2.0,
            size: 1.8,
            bold: false,
            text: if p.spot_sizes_um.is_empty() { "Spot sizes are shown with each piece".into() } else { "LASER SPOT Ø (µm)".into() },
        }],
        height: 2.5,
    };
    let (mut x, mut y, mut row_h) = (44.0, 0.0, 0.0_f64);
    for d in sizes {
        let r = d / k / 2.0;
        if x + 2.0 * r > width {
            x = 0.0;
            y += row_h.max(3.0) + 1.0;
            row_h = 0.0;
        }
        key.circles.push((x + r, y + r, r, d));
        row_h = row_h.max(2.0 * r + 2.4);
        key.height = key.height.max(y + row_h);
        x += 2.0 * r + 4.0;
    }
    if !off.is_empty() {
        let note = format!(
            "{} µm left off: too small to draw or tell apart at this scale",
            off.iter()
                .map(|d| crate::layout::fmt_num(*d))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for text in wrap_header(&note, width, 1.8) {
            key.text.push(HeaderText {
                x: 0.0,
                y: key.height + 2.3,
                size: 1.8,
                bold: false,
                text,
            });
            key.height += 2.7;
        }
    }
    key
}

/// Preserve sheet-1 title and counts, including long text. Extra baselines move
/// both the grid and the map, and are included in the placement header height.
fn first_heading(
    p: &SheetParams,
    meta: &SheetMeta,
    page: crate::layout::Paper,
    stats: &SheetStats,
) -> (Vec<HeaderText>, f64) {
    let width = page.w_mm - 2.0 * p.margin_mm;
    let title = if p.title.is_empty() {
        &meta.package
    } else {
        &p.title
    };
    let title_lines = wrap_header(title, (width - map_width(page) - 6.0) * 0.58 / 0.66, 4.0);
    let title_extra = (title_lines.len() - 1) as f64 * 4.8;
    let mut text: Vec<_> = title_lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| HeaderText {
            x: 0.0,
            y: 4.0 + i as f64 * 4.8,
            size: 4.0,
            bold: true,
            text,
        })
        .collect();
    let counts = format!(
        "{} grains · {} bands · {} duplicates left to their owner area{}{}{}",
        stats.grains,
        stats.bands,
        stats.duplicates,
        if stats.strays > 0 {
            format!(" · {} strays left off", stats.strays)
        } else {
            String::new()
        },
        if stats.isolated > 0 {
            format!(" · {} isolated", stats.isolated)
        } else {
            String::new()
        },
        if p.printed.is_empty() {
            String::new()
        } else {
            format!(" · printed {}", p.printed)
        }
    );
    let count_lines = wrap_header(&counts, width, 1.9);
    let extra = title_extra + (count_lines.len() - 1) as f64 * 2.5;
    text.extend(
        count_lines
            .into_iter()
            .enumerate()
            .map(|(i, text)| HeaderText {
                x: 0.0,
                y: 7.8 + title_extra + i as f64 * 2.5,
                size: 1.9,
                bold: false,
                text,
            }),
    );
    (text, extra)
}
pub(super) fn map_top(
    p: &SheetParams,
    meta: &SheetMeta,
    page: crate::layout::Paper,
    stats: &SheetStats,
) -> f64 {
    10.0 + first_heading(p, meta, page, stats).1
}

pub(super) fn heights(plan: &SheetPlan) -> [f64; 2] {
    let page = crate::layout::Paper {
        w_mm: plan.paper_mm[0],
        h_mm: plan.paper_mm[1],
    };
    // Compact maps abbreviate and limit labels instead of growing with the mount.
    let width = page.w_mm - 2.0 * plan.params.margin_mm - map_width(page) - 6.0;
    let (kv, _) = header_values(
        &plan.params,
        &plan.meta,
        plan.um_per_mm,
        &scale_note(plan.auto_scale.as_ref(), plan.um_per_mm),
    );
    let (_, mut grid_end) = packed_grid(&kv, width);
    // Counts may change after edits. Reserve their longest possible form so a
    // newly isolated target cannot make a wrapped stats line invade the grid.
    let mut counts = plan.cut_stats.clone();
    counts.bands = plan.strips.iter().map(|s| s.bands.len()).sum();
    counts.isolated = counts.grains;
    let (_, extra) = first_heading(&plan.params, &plan.meta, page, &counts);
    let mut key = packed_key(&plan.params, plan.um_per_mm, width);
    if plan.strips.iter().any(|s|s.transform.is_some()) {
        let scales:Vec<_>=plan.strips.iter().filter(|s|!s.hidden).map(|s|plan.um_per_mm/s.transform.unwrap_or_default().scale).collect();
        let mixed=scales.first().is_some_and(|first|scales.iter().any(|v|(v-first).abs()>1e-7));
        let k=if mixed {plan.um_per_mm} else {scales.first().copied().unwrap_or(plan.um_per_mm)};
        let params=SheetParams {spot_sizes_um:if mixed {Vec::new()} else {key_spots(&plan.params.spot_sizes_um,k).0},..plan.params.clone()};
        let note=if mixed {"Per-piece scale; see each scale bar".into()} else {scale_note(None,k)};
        let (values,_)=header_values(&params,&plan.meta,k,&note);
        grid_end=grid_end.max(packed_grid(&values,width).1);
        key.height=key.height.max(packed_key(&params,k,width).height);
    }
    [
        (grid_end + extra + 5.2 + key.height + 0.8).max(PACKED_FIRST_HEADER_MM),
        (15.0 + key.height + 0.8).max(PACKED_LATER_HEADER_MM),
    ]
}
pub(super) fn map_width(page: crate::layout::Paper) -> f64 {
    (page.w_mm * 0.27).clamp(60.0, 100.0)
}

fn header_text(s: &mut String, m: f64, t: &HeaderText) {
    let weight = if t.bold { "bold" } else { "normal" };
    let _ = write!(
        s,
        r#"<text x="{:.3}" y="{:.3}" font-size="{}" font-weight="{weight}" fill="{INK}">{}</text>"#,
        m + t.x,
        m + t.y,
        t.size,
        esc(&t.text)
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    s: &mut String,
    p: &SheetParams,
    meta: &SheetMeta,
    page: crate::layout::Paper,
    sheet: usize,
    sheets: usize,
    here: &[String],
    areas: &[SheetArea],
    printed: &[Vec<&Grain>],
    ownership: &Ownership,
    stats: &SheetStats,
    k: f64,
    header_mm: f64,
    kv: &[(&str, String)],
    processing: &str,
    bands: &[(usize, String, [f64; 4])],
) {
    let m = p.margin_mm;
    let width = page.w_mm - 2.0 * m - map_width(page) - 6.0;
    let _ = write!(s, r#"<g data-header="compact">"#);
    let title = if p.title.is_empty() {
        &meta.package
    } else {
        &p.title
    };
    if sheet > 1 {
        header_text(
            s,
            m,
            &HeaderText {
                x: 0.0,
                y: 4.0,
                size: 4.0,
                bold: true,
                text: ellipsis(title, width, 4.0, true),
            },
        );
    }
    let number = format!("Sheet {sheet} / {sheets}");
    header_text(
        s,
        m,
        &HeaderText {
            x: page.w_mm - 2.0 * m - text_w(&number, 3.0, true),
            y: 4.0,
            size: 3.0,
            bold: true,
            text: number,
        },
    );
    let on_y = if sheet == 1 {
        let (heading, extra) = first_heading(p, meta, page, stats);
        for text in heading {
            header_text(s, m, &text);
        }
        let (grid, end) = packed_grid(kv, width);
        for mut t in grid {
            t.y += extra;
            header_text(s, m, &t);
        }
        end + extra + 0.8
    } else {
        header_text(
            s,
            m,
            &HeaderText {
                x: 0.0,
                y: 8.0,
                size: 2.2,
                bold: false,
                text: ellipsis(processing, width, 2.2, false),
            },
        );
        11.4
    };
    header_text(
        s,
        m,
        &HeaderText {
            x: 0.0,
            y: on_y,
            size: 1.8,
            bold: false,
            text: "ON THIS SHEET".into(),
        },
    );
    // Only areas actually placed here, at most two lines. Reserve the suffix
    // before accepting an entry, so even an oversized single name stays bounded.
    let entries: Vec<_> = here
        .iter()
        .map(|sn| {
            let ar: Vec<_> = areas
                .iter()
                .enumerate()
                .filter(|(ai, _)| bands.iter().any(|(j, sample, _)| j == ai && sample == sn))
                .map(|(_, a)| area_short(a.rec))
                .collect();
            let ng: usize = areas
                .iter()
                .enumerate()
                .map(|(ai, a)| {
                    printed[ai]
                        .iter()
                        .filter(|g| {
                            grain_sample(g, ownership.sample_of(&a.rec.id).unwrap_or(a.img.sample))
                                == sn
                        })
                        .count()
                })
                .sum();
            format!("{sn} · {ng} grains · area {}", ar.join(" + "))
        })
        .collect();
    let mut lines: Vec<String> = Vec::new();
    let mut accepted = String::new();
    for (i, entry) in entries.iter().enumerate() {
        let candidate = if accepted.is_empty() {
            entry.clone()
        } else {
            format!("{accepted} · {entry}")
        };
        let wrapped = wrap_header(&candidate, width - 24.0, 2.0);
        let remaining = entries.len() - i - 1;
        let suffix = if remaining > 0 {
            format!(" · + {remaining} more")
        } else {
            String::new()
        };
        // Reserve the suffix before accepting an entry. Names and areas that
        // fit are complete; omitted entries are counted rather than clipped.
        if wrapped.len() <= 2
            && text_w(&(wrapped.last().unwrap().clone() + &suffix), 2.0, false) <= width - 24.0
        {
            accepted = candidate;
            lines = wrapped;
        } else {
            let more = entries.len() - i;
            if let Some(last) = lines.last_mut() {
                last.push_str(&format!(" · + {more} more"));
            } else {
                lines.push(format!("+ {more} more"));
            }
            break;
        }
    }
    for (i, text) in lines.into_iter().enumerate() {
        header_text(
            s,
            m,
            &HeaderText {
                x: 24.0,
                y: on_y + i as f64 * 2.6,
                size: 2.0,
                bold: false,
                text,
            },
        );
    }
    let key = packed_key(p, k, width);
    let key_top = header_mm - key.height - 0.8;
    for mut t in key.text {
        t.y += key_top;
        header_text(s, m, &t);
    }
    for (x, y, r, d) in key.circles {
        let _ = write!(
            s,
            r#"<circle cx="{:.3}" cy="{:.3}" r="{r:.3}" fill="none" stroke="{INK}" stroke-width="0.2"/>"#,
            m + x,
            m + key_top + y
        );
        let label = crate::layout::fmt_num(d);
        header_text(
            s,
            m,
            &HeaderText {
                x: x - text_w(&label, 1.8, false) / 2.0,
                y: key_top + y + r + 2.0,
                size: 1.8,
                bold: false,
                text: label,
            },
        );
    }
    let _ = write!(s, "</g>");
}

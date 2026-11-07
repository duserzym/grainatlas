//! Read-only consistency checks of the same full project record used by Save.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    project::Project,
    segment::Grain,
    spots::{Spot, SpotKind, format_name, grain_at},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub code: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
    pub spot_ids: Vec<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain_id: Option<Uuid>,
    pub message: String,
    /// Only near_miss findings offer an explicit rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn finding(code: &str, spots: &[&Spot], message: String) -> Finding {
    let area_id = spots
        .first()
        .map(|s| s.area_id.clone())
        .filter(|a| spots.iter().all(|s| &s.area_id == a));
    Finding {
        code: code.into(),
        severity: "warning".into(),
        area_id,
        spot_ids: spots.iter().map(|s| s.id).collect(),
        grain_id: None,
        message,
        label: None,
    }
}
fn names(spots: &[&Spot]) -> String {
    let mut text = spots
        .iter()
        .take(6)
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if spots.len() > 6 {
        text.push_str(&format!(", … ({} more)", spots.len() - 6));
    }
    text
}

fn tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(['-', '_', ' '])
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
/// Case and separators are equivalent. Otherwise the trailing number must agree and
/// all of the spot's sample tokens must occur in the acquisition label.
pub fn label_matches(spot: &str, label: &str) -> bool {
    let a = tokens(spot);
    let b = tokens(label);
    if a == b {
        return true;
    }
    let number = |v: &Vec<String>| v.last().and_then(|s| s.parse::<u64>().ok());
    number(&a).is_some()
        && number(&a) == number(&b)
        && a.len() > 1
        && a[..a.len() - 1]
            .iter()
            .all(|t| b[..b.len() - 1].contains(t))
}

/// Unambiguous suggestions among unmatched names only, unique in both directions.
pub fn match_labels(spots: &[Spot], labels: &[String]) -> Vec<Finding> {
    let labels: BTreeSet<&str> = labels.iter().map(String::as_str).collect();
    let missing: Vec<&Spot> = spots
        .iter()
        .filter(|s| !labels.contains(s.name.as_str()))
        .collect();
    let unused: Vec<&str> = labels
        .into_iter()
        .filter(|l| !spots.iter().any(|s| s.name == *l))
        .collect();
    let mut out = Vec::new();
    let mut missing_groups: BTreeMap<&str, Vec<&Spot>> = BTreeMap::new();
    for s in &missing {
        missing_groups.entry(&s.area_id).or_default().push(s);
    }
    for (area, ss) in missing_groups {
        out.push(finding(
            "no_label",
            &ss,
            format!(
                "{area}: {} spots without an acquisition label ({})",
                ss.len(),
                names(&ss)
            ),
        ));
    }
    // Labels have no area metadata. Associate a unique sample/area when possible;
    // otherwise group by the instrument's sample prefix (before its run number).
    let mut unused_groups: BTreeMap<(Option<String>, String), Vec<&str>> = BTreeMap::new();
    for l in &unused {
        let ts = tokens(l);
        let prefix = &ts[..ts.len().saturating_sub(1)];
        let candidates: BTreeSet<_> = spots
            .iter()
            .filter(|s| {
                let sample = tokens(&s.sample);
                !sample.is_empty() && sample.iter().all(|t| prefix.contains(t))
            })
            .map(|s| (Some(s.area_id.clone()), s.sample.clone()))
            .collect();
        let key = if candidates.len() == 1 {
            candidates.into_iter().next().unwrap()
        } else {
            (None, prefix.join("-"))
        };
        unused_groups.entry(key).or_default().push(l);
    }
    for ((area, sample), ls) in unused_groups {
        let mut f = finding(
            "no_spot",
            &[],
            format!(
                "{sample}: {} acquisition labels without a spot ({}{})",
                ls.len(),
                ls.iter().take(6).copied().collect::<Vec<_>>().join(", "),
                if ls.len() > 6 {
                    format!(", … ({} more)", ls.len() - 6)
                } else {
                    String::new()
                }
            ),
        );
        f.area_id = area;
        out.push(f);
    }
    for s in &missing {
        let candidates: Vec<_> = unused
            .iter()
            .filter(|l| label_matches(&s.name, l))
            .collect();
        if candidates.len() != 1 {
            continue;
        }
        let label = *candidates[0];
        if missing
            .iter()
            .filter(|other| label_matches(&other.name, label))
            .count()
            != 1
        {
            continue;
        }
        let mut f = finding("near_miss", &[s], format!("{} → {label}", s.name));
        f.label = Some(label.into());
        out.push(f);
    }
    out
}

fn boundary_distance(g: &Grain, x: f64, y: f64) -> f64 {
    let mut best = f64::INFINITY;
    for (a, b) in g
        .polygon_um
        .iter()
        .zip(g.polygon_um.iter().cycle().skip(1))
        .take(g.polygon_um.len())
    {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let t = if dx * dx + dy * dy == 0.0 {
            0.0
        } else {
            ((x - a[0]) * dx + (y - a[1]) * dy) / (dx * dx + dy * dy)
        }
        .clamp(0.0, 1.0);
        best = best.min((x - a[0] - t * dx).hypot(y - a[1] - t * dy));
    }
    best
}

/// `None` means no label source was loaded; `Some([])` means a loaded, empty list.
/// Never modifies the project, grains, spots, names or labels.
pub fn check_project(
    project: &Project,
    grains: &BTreeMap<String, Vec<Grain>>,
    spots: &[Spot],
    labels: Option<&[String]>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut patterns: BTreeMap<&str, Vec<&Spot>> = BTreeMap::new();
    for s in spots {
        let gs = grains
            .get(&s.area_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let stored = s
            .grain_id
            .and_then(|id| gs.iter().find(|g| g.id == id && g.is_grain()));
        let stale = s.grain_id.is_some() && stored.is_none();
        let mut points = vec![(s.x_um, s.y_um)];
        if s.kind != SpotKind::Spot
            && let (Some(x), Some(y)) = (s.x2_um, s.y2_um)
        {
            points.push((x, y));
        }
        let hits: Vec<_> = points.iter().map(|&(x, y)| grain_at(gs, x, y)).collect();
        if stale || hits.iter().any(Option::is_none) {
            out.push(finding("off_grain", &[s], format!("{}: {}", s.name, if stale { "stored grain no longer exists (deleted, split, merged or marked as artifact)" } else { "centre or end point is outside every current grain" })));
        }
        if stored.is_some_and(|g| Some(g.number) != s.grain_number)
            || hits
                .iter()
                .flatten()
                .any(|g| Some(g.id) != s.grain_id || Some(g.number) != s.grain_number)
        {
            let mut f = finding(
                "grain_mismatch",
                &[s],
                format!(
                    "{}: grain id or number differs from current grain {}; stored number {}",
                    s.name,
                    hits.iter()
                        .flatten()
                        .copied()
                        .chain(stored.filter(|_| hits.iter().all(Option::is_none)))
                        .map(|g| g.number.to_string())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join(", "),
                    s.grain_number
                        .map_or_else(|| "none".into(), |n| n.to_string())
                ),
            );
            f.grain_id = hits
                .iter()
                .flatten()
                .next()
                .map(|g| g.id)
                .or_else(|| stored.map(|g| g.id));
            out.push(f);
        }
        let mut overhang: f64 = 0.0;
        for (&(x, y), hit) in points.iter().zip(&hits) {
            if let Some(g) = hit {
                overhang = overhang.max(s.diameter_um / 2.0 - boundary_distance(g, x, y));
            }
        }
        if overhang > 0.0 {
            let mut f = finding(
                "ring_outside",
                &[s],
                format!("{}: ring overhangs the grain by {overhang:.2} µm", s.name),
            );
            f.grain_id = hits.iter().flatten().next().map(|g| g.id);
            out.push(f);
        }
        if let Some(area) = project.areas.iter().find(|a| a.id == s.area_id) {
            let sample = if area.sample.is_empty() {
                &area.name
            } else {
                &area.sample
            };
            if &s.sample != sample {
                out.push(finding(
                    "sample_mismatch",
                    &[s],
                    format!(
                        "{}: sample {:?}, area sample {:?}",
                        s.name, s.sample, sample
                    ),
                ));
            }
        }
        let expected = format_name(
            project.settings.pattern_for(&s.area_id),
            &s.sample,
            s.grain_number,
            s.n,
        );
        if expected != s.name {
            patterns.entry(&s.area_id).or_default().push(s);
        }
    }
    for (area_id, ss) in patterns {
        let area = project.areas.iter().find(|a| a.id == area_id);
        let title = area.map_or(area_id, |a| a.name.as_str());
        let pattern = project.settings.pattern_for(area_id);
        let s = ss[0];
        let expected = format_name(pattern, &s.sample, s.grain_number, s.n);
        out.push(finding("pattern_mismatch", &ss, format!(
            "{title}: {} names differ from the pattern {pattern} (e.g. {}, pattern gives {expected})",
            ss.len(), s.name
        )));
    }
    let mut exact: BTreeMap<&str, Vec<&Spot>> = BTreeMap::new();
    let mut folded: BTreeMap<String, Vec<&Spot>> = BTreeMap::new();
    let mut samples: BTreeMap<&str, BTreeMap<u32, Vec<&Spot>>> = BTreeMap::new();
    let mut numbers: BTreeMap<u32, Vec<&Spot>> = BTreeMap::new();
    for s in spots {
        exact.entry(&s.name).or_default().push(s);
        folded.entry(s.name.to_lowercase()).or_default().push(s);
        samples
            .entry(&s.sample)
            .or_default()
            .entry(s.n)
            .or_default()
            .push(s);
        numbers.entry(s.n).or_default().push(s);
    }
    for ss in exact.values().filter(|ss| ss.len() > 1) {
        out.push(finding(
            "duplicate_name",
            ss,
            format!("{}: duplicate name (case-sensitive)", names(ss)),
        ));
    }
    for ss in folded
        .values()
        .filter(|ss| ss.len() > 1 && ss.iter().any(|s| s.name != ss[0].name))
    {
        out.push(finding(
            "duplicate_name",
            ss,
            format!("{}: duplicate name ignoring case", names(ss)),
        ));
    }
    for (sample, ns) in samples {
        let mut ranges = Vec::new();
        let mut next = u64::from(*ns.keys().next().unwrap());
        for (&n, ss) in &ns {
            let n = u64::from(n);
            if n > next {
                ranges.push(if n == next + 1 {
                    next.to_string()
                } else {
                    format!("{next}–{}", n - 1)
                });
            }
            next = n + 1;
            if ss.len() > 1 {
                out.push(finding(
                    "duplicate_n",
                    ss,
                    format!("{}: sample {sample} repeats n={n}", names(ss)),
                ));
            }
        }
        if !ranges.is_empty() {
            let ss: Vec<_> = ns.values().flatten().copied().collect();
            out.push(finding(
                "n_gap",
                &ss,
                format!(
                    "Sample {sample} ({}): missing n {}",
                    names(&ss),
                    ranges.join(", ")
                ),
            ));
        }
    }
    for (n, ss) in numbers {
        let samples: BTreeSet<_> = ss.iter().map(|s| &s.sample).collect();
        if samples.len() > 1 {
            out.push(finding(
                "n_reused",
                &ss,
                format!(
                    "For information: {} use n={n} in several samples",
                    names(&ss)
                ),
            ));
        }
    }
    for (i, a) in spots.iter().enumerate() {
        for b in &spots[i + 1..] {
            if a.area_id == b.area_id
                && (a.x_um - b.x_um).hypot(a.y_um - b.y_um) < (a.diameter_um + b.diameter_um) / 2.0
            {
                out.push(finding(
                    "overlap",
                    &[a, b],
                    format!("{} and {}: rings overlap", a.name, b.name),
                ));
            }
        }
    }
    if let Some(labels) = labels {
        out.extend(match_labels(spots, labels));
    }
    group_findings(out, spots)
}

fn group_findings(findings: Vec<Finding>, spots: &[Spot]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut groups = BTreeMap::new();
    let mut examples: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for f in findings {
        if f.code == "near_miss" || f.code == "n_gap" || f.code == "no_spot" {
            out.push(f);
            continue;
        }
        let samples = if f.area_id.is_none() {
            spots
                .iter()
                .filter(|s| f.spot_ids.contains(&s.id))
                .map(|s| s.sample.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        };
        let key = (f.code.clone(), f.area_id.clone(), samples);
        if let Some(&i) = groups.get(&key) {
            let previous: &mut Finding = &mut out[i];
            previous.spot_ids.extend(f.spot_ids);
            previous.spot_ids.sort();
            previous.spot_ids.dedup();
            if previous.grain_id != f.grain_id {
                previous.grain_id = None;
            }
            examples.entry(i).or_default().push(f.message);
        } else {
            groups.insert(key, out.len());
            examples.insert(out.len(), vec![f.message.clone()]);
            out.push(f);
        }
    }
    for (i, messages) in examples {
        if messages.len() > 1 {
            out[i].message = format!(
                "{}: {} spots; {}{}",
                out[i].area_id.as_deref().unwrap_or("Samples"),
                out[i].spot_ids.len(),
                messages
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; "),
                if messages.len() > 6 {
                    format!("; … ({} more findings)", messages.len() - 6)
                } else {
                    String::new()
                }
            );
        }
    }
    out
}

/// Full saved project JSON, with strict parsing of UI-held grains and spots.
pub fn check_json(full_project_json: &str, labels_json: &str) -> crate::Result<String> {
    let invalid = |e: serde_json::Error| crate::Error::Invalid(e.to_string());
    let project: Project = serde_json::from_str(full_project_json).map_err(invalid)?;
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct Ui {
        grains: BTreeMap<String, Vec<Grain>>,
        spots: Vec<Spot>,
    }
    let ui: Ui = project
        .extra
        .get("grainatlas_ui")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(invalid)?
        .unwrap_or_default();
    let labels: Option<Vec<String>> = if labels_json.is_empty() {
        None
    } else {
        serde_json::from_str(labels_json).map_err(invalid)?
    };
    serde_json::to_string(&check_project(
        &project,
        &ui.grains,
        &ui.spots,
        labels.as_deref(),
    ))
    .map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn grain(id: u128, number: u32) -> Grain {
        serde_json::from_value(json!({"id": Uuid::from_u128(id), "number": number,
            "polygon_um": [[0.,0.],[100.,0.],[100.,100.],[0.,100.]], "bbox_um": [0.,0.,100.,100.],
            "centroid_um": [50.,50.], "area_um2": 10000., "eq_diameter_um": 100., "major_um": 100.,
            "minor_um": 100., "orientation_deg": 0., "mean_intensity": 0.5, "scale_bar_um": 20.}))
        .unwrap()
    }
    fn spot(id: u128, area: &str, sample: &str, n: u32, x: f64, y: f64) -> Spot {
        serde_json::from_value(json!({"id": Uuid::from_u128(id), "area_id": area, "sample": sample,
            "name": format!("{sample}_{n}"), "n": n, "grain_id": Uuid::from_u128(if area == "a" {1} else {2}),
            "grain_number": 1, "x_um": x, "y_um": y, "diameter_um": 10.})).unwrap()
    }
    fn project() -> Project {
        serde_json::from_value(json!({"schema_version":1,"uuid": Uuid::from_u128(99),"name":"test","created":"t",
            "package":{"path":"test","reader":"test","name":"test"},
            "areas": (["a","b"].map(|id| json!({"uuid": Uuid::from_u128(if id == "a" {10} else {11}),
                "id": id,"name": id,"sample": id.to_uppercase(), "pixel_size_nm":1000.,"stage_sign":[1.,1.],
                "tiles":[],"placement":null,"photometric":null,"mosaic":null})))})).unwrap()
    }
    fn gs() -> BTreeMap<String, Vec<Grain>> {
        BTreeMap::from([
            ("a".into(), vec![grain(1, 1)]),
            ("b".into(), vec![grain(2, 1)]),
        ])
    }
    fn codes(p: &Project, gs: &BTreeMap<String, Vec<Grain>>, ss: &[Spot]) -> Vec<String> {
        check_project(p, gs, ss, None)
            .into_iter()
            .map(|f| f.code)
            .collect()
    }

    #[test]
    fn geometry_and_clean_spot() {
        let p = project();
        let mut grains = gs();
        let clean = spot(100, "a", "A", 1, 50., 50.);
        assert!(
            check_project(
                &p,
                &grains,
                std::slice::from_ref(&clean),
                Some(&["A_1".into()])
            )
            .is_empty()
        );
        let mut s = clean.clone();
        s.x_um = 110.;
        assert_eq!(codes(&p, &grains, &[s.clone()]), ["off_grain"]);
        s.x_um = 50.;
        s.grain_id = Some(Uuid::from_u128(999));
        let f = check_project(&p, &grains, &[s.clone()], None);
        assert!(
            f.iter()
                .any(|f| f.code == "off_grain" && f.message.contains("no longer exists"))
        );
        assert!(f.iter().any(|f| f.code == "grain_mismatch"));
        s = clean.clone();
        s.grain_number = Some(7);
        assert_eq!(codes(&p, &grains, &[s.clone()]), ["grain_mismatch"]);
        s.x_um = 110.;
        assert_eq!(codes(&p, &grains, &[s]), ["off_grain", "grain_mismatch"]);
        s = clean.clone();
        s.x_um = 2.;
        let f = check_project(&p, &grains, &[s], None);
        assert_eq!(f[0].code, "ring_outside");
        assert!(f[0].message.contains("3.00 µm"));
        for kind in [SpotKind::Line, SpotKind::Raster] {
            s = clean.clone();
            s.kind = kind;
            s.x2_um = Some(110.);
            s.y2_um = Some(50.);
            assert!(codes(&p, &grains, &[s]).contains(&"off_grain".into()));
        }
        grains.get_mut("a").unwrap()[0].artifact = Some(crate::segment::Artifact::Manual);
        assert!(codes(&p, &grains, &[clean]).contains(&"off_grain".into()));
    }

    #[test]
    fn names_numbers_samples_overlap_and_patterns() {
        let mut p = project();
        let grains = gs();
        let a = spot(100, "a", "A", 1, 30., 30.);
        let mut b = spot(101, "a", "A", 1, 31., 30.);
        let mut c = spot(102, "b", "B", 3, 50., 50.);
        c.name = "a_1".into();
        let d = spot(103, "b", "B", 1, 20., 20.);
        let f = check_project(&p, &grains, &[a, b.clone(), c, d], None);
        for code in [
            "duplicate_name",
            "duplicate_n",
            "n_gap",
            "n_reused",
            "overlap",
            "pattern_mismatch",
        ] {
            assert!(f.iter().any(|f| f.code == code), "{code}");
        }
        assert!(
            f.iter()
                .any(|f| f.code == "duplicate_name" && f.message.contains("case-sensitive"))
        );
        assert!(
            f.iter()
                .any(|f| f.code == "duplicate_name" && f.message.contains("ignoring case"))
        );
        b.sample = "Old".into();
        b.name = "Old_1".into();
        assert_eq!(codes(&p, &grains, &[b.clone()]), ["sample_mismatch"]);
        b.sample = "A".into();
        b.name = "A_S_1".into();
        p.settings
            .area_patterns
            .insert("a".into(), "{sample}_S_{n}".into());
        assert!(codes(&p, &grains, &[b.clone()]).is_empty());
        let s =
            crate::spots::new_spot("a", "A", &grains["a"], &[], &p.settings, 50., 50., 10., "t");
        assert_eq!(s.name, "A_S_1");
        assert_eq!(p.settings.pattern_for("b"), "{sample}_{n}");
        p.settings.area_patterns.insert("a".into(), " ".into());
        assert_eq!(p.settings.pattern_for("a"), "{sample}_{n}");
        let f = check_project(&p, &grains, &[spot(100, "a", "A", 8, 50., 50.)], None);
        assert!(!f.iter().any(|f| f.code == "n_gap"));
    }

    #[test]
    fn gaps_are_strictly_inside_a_samples_observed_range() {
        let ss: Vec<_> = (488..=492)
            .map(|n| spot(n as u128, "a", "A", n, f64::from(n - 486) * 10., 50.))
            .collect();
        assert!(check_project(&project(), &gs(), &ss, None).is_empty());
        let ss: Vec<_> = [488, 489, 491]
            .into_iter()
            .map(|n| spot(n as u128, "a", "A", n, f64::from(n - 486) * 10., 50.))
            .collect();
        let findings = check_project(&project(), &gs(), &ss, None);
        let gaps: Vec<_> = findings.iter().filter(|f| f.code == "n_gap").collect();
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].message.ends_with("missing n 490"));
        assert_eq!(gaps[0].spot_ids.len(), 3);
    }

    #[test]
    fn bulk_naming_and_label_findings_are_grouped_by_area_or_sample() {
        let mut ss: Vec<_> = (1..=22)
            .map(|n| spot(n as u128 + 100, "a", "A", n, 50., 50.))
            .collect();
        for s in &mut ss {
            s.name = format!("A_S_{}", s.n);
        }
        let mut other = spot(200, "b", "B", 1, 50., 50.);
        other.name = "B_S_1".into();
        ss.push(other);
        let labels = (1..=22).map(|n| format!("A_S_xs_{n}")).collect::<Vec<_>>();
        let f = check_project(&project(), &gs(), &ss, Some(&labels));
        for code in ["pattern_mismatch", "no_label"] {
            let rows: Vec<_> = f.iter().filter(|f| f.code == code).collect();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].spot_ids.len(), 22);
        }
        let pattern = f.iter().find(|f| f.code == "pattern_mismatch").unwrap();
        assert!(
            pattern
                .message
                .contains("22 names differ from the pattern {sample}_{n}")
        );
        assert!(pattern.message.contains("e.g. A_S_1, pattern gives A_1"));
        let unused: Vec<_> = f.iter().filter(|f| f.code == "no_spot").collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("22 acquisition labels"));
        assert!(!unused[0].message.contains("A_S_xs_22"));
        assert_eq!(f.iter().filter(|f| f.code == "near_miss").count(), 22);
        assert_eq!(f.iter().filter(|f| f.code == "overlap").count(), 1);
    }

    #[test]
    fn repeated_numbers_across_samples_do_not_flood_the_report() {
        let ss: Vec<_> = (488..=492)
            .flat_map(|n| {
                [
                    spot(n as u128, "a", "A", n, 50., 50.),
                    spot(n as u128 + 1000, "b", "B", n, 50., 50.),
                ]
            })
            .collect();
        let findings = check_project(&project(), &gs(), &ss, None);
        let reused: Vec<_> = findings.iter().filter(|f| f.code == "n_reused").collect();
        assert_eq!(reused.len(), 1);
        assert_eq!(reused[0].spot_ids.len(), 10);
        assert!(reused[0].message.contains("n=488"));
        assert!(reused[0].message.contains("n=492"));
    }

    #[test]
    fn existing_other_grain_and_end_point_mismatch() {
        let p = project();
        let mut gs = gs();
        let mut other = grain(3, 2);
        for point in &mut other.polygon_um {
            point[0] += 120.;
        }
        other.bbox_um = [120., 0., 220., 100.];
        gs.get_mut("a").unwrap().push(other);
        let s = spot(100, "a", "A", 1, 150., 50.);
        assert_eq!(codes(&p, &gs, &[s]), ["grain_mismatch"]);
        let mut line = spot(101, "a", "A", 1, 50., 50.);
        line.kind = SpotKind::Line;
        line.x2_um = Some(150.);
        line.y2_um = Some(50.);
        assert_eq!(codes(&p, &gs, &[line]), ["grain_mismatch"]);
        let settings: crate::spots::Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.area_patterns.is_empty());
    }

    #[test]
    fn overlap_uses_sum_of_radii_and_strict_boundary() {
        let p = project();
        let gs = gs();
        let a = spot(100, "a", "A", 1, 30., 30.);
        let mut b = spot(101, "a", "A", 2, 40., 30.);
        assert!(!codes(&p, &gs, &[a.clone(), b.clone()]).contains(&"overlap".into()));
        b.diameter_um = 12.;
        assert!(codes(&p, &gs, &[a, b]).contains(&"overlap".into()));
    }

    #[test]
    fn acquisition_matching_examples_and_ambiguity() {
        assert!(label_matches("Skin_mafic-253", "Skin-mafic_xs_253"));
        assert!(label_matches("HH3-1", "HH3_S_1"));
        assert!(label_matches("HH3-1", "hh3_1"));
        assert!(!label_matches("HH3-1", "HH3_S_2"));
        assert!(!label_matches("HH3-1", "HH30_S_1"));
        let mut s = spot(100, "a", "A", 1, 50., 50.);
        s.name = "HH3-1".into();
        let labels = vec!["HH3_S_1".into()];
        let f = match_labels(&[s.clone()], &labels);
        assert_eq!(
            f.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(),
            ["no_label", "no_spot", "near_miss"]
        );
        assert_eq!(f[2].label.as_deref(), Some("HH3_S_1"));
        assert!(
            !match_labels(&[s.clone()], &["HH3_S_1".into(), "HH3_xs_1".into()])
                .iter()
                .any(|f| f.code == "near_miss")
        );
        let mut other = s.clone();
        other.id = Uuid::from_u128(101);
        assert!(
            !match_labels(&[s.clone(), other], &labels)
                .iter()
                .any(|f| f.code == "near_miss")
        );
        assert!(match_labels(&[s], &["HH3-1".into()]).is_empty());
    }

    #[test]
    fn full_json_and_session_are_unchanged() {
        let mut p = serde_json::to_value(project()).unwrap();
        p["grainatlas_ui"] = json!({"grains":gs(), "spots":[spot(100,"a","A",1,110.,50.)]});
        let input = serde_json::to_string_pretty(&p).unwrap();
        let before = input.as_bytes().to_vec();
        let mut session = crate::project::session::Session::new("test");
        session.load_project(&input).unwrap();
        let saved_before = session.project_json().unwrap();
        let project_before = session.project.clone();
        let findings = session.check_project(&input, "null").unwrap();
        assert!(findings.contains("off_grain"));
        assert_eq!(input.as_bytes(), before);
        assert_eq!(
            p,
            serde_json::from_str::<serde_json::Value>(&input).unwrap()
        );
        assert_eq!(session.project, project_before);
        assert_eq!(
            session.project_json().unwrap().as_bytes(),
            saved_before.as_bytes()
        );
        assert!(
            session
                .check_project(&input, "[]")
                .unwrap()
                .contains("no_label")
        );
        // Grain labels are area-qualified, even when local numbers repeat.
        let spots = [
            spot(100, "a", "A", 1, 50., 50.),
            spot(101, "b", "B", 1, 50., 50.),
        ];
        let doc = crate::atlas::spots_document(&project(), &spots, &["a", "b"]);
        assert_eq!(doc["documents"][0]["spots"][0]["grain"], "a/1");
        assert_eq!(doc["documents"][1]["spots"][0]["grain"], "b/1");
    }
}

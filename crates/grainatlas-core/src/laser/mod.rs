//! Laser registration and spot lists (plan sections 5.5 and 6.7, roadmap phase 11).
//!
//! The operator drives the laser stage to three or more fiducial spots and reads the stage
//! coordinates off the laser software; [`fit`] turns those pairs into an affine map from mosaic
//! micrometres to the laser stage (least squares, residuals in micrometres), and an adapter
//! writes the spot list in the laser's units. The registration is stored per area in
//! `project.json` under `laser`.
//!
//! # Adding the lab's laser software
//!
//! Implement [`LaserAdapter`] for one more unit struct in this module (or a submodule): an
//! `id` (stable, stored in the project), a `label` for the UI, the file `extension`, and
//! `write`, which receives the registered rows in the laser's units and returns the file
//! text in exactly the layout the software imports. Register it in [`adapters`]. Base the
//! layout on an example file exported from the software itself (column order, header lines,
//! decimal separator, line endings, units and axis directions), and add a test that writes
//! the fixture spots and compares with that example. `docs/laser-adapters.md` says the same
//! for people without the code open.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::Project;
use crate::spots::{Spot, SpotKind};
use crate::{Error, Result};

/// Residuals above this (micrometres) get a warning.
pub const WARN_UM: f64 = 5.0;

/// One fiducial: a spot (or a point) in mosaic micrometres and the laser stage reading there.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Fiducial {
    pub spot_id: Option<Uuid>,
    pub name: String,
    pub x_um: f64,
    pub y_um: f64,
    pub x_stage: Option<f64>,
    pub y_stage: Option<f64>,
}

/// The fitted map mosaic µm → stage: x' = a·x + c·y + e, y' = b·x + d·y + f (`[a, b, c, d, e, f]`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Fit {
    pub matrix: [f64; 6],
    /// Residual of each complete fiducial, in micrometres (same order as the fiducials used).
    pub residuals_um: Vec<f64>,
    pub rms_um: f64,
    pub max_um: f64,
    /// Fiducials used (with both stage values).
    pub n: usize,
    /// True with exactly three fiducials: an affine passes through them, residuals say nothing.
    pub exact: bool,
    /// Any residual above [`WARN_UM`].
    pub warn: bool,
    /// Scale (stage units per µm) along x and y, and the rotation in degrees, for a sanity check.
    pub scale: [f64; 2],
    pub rotation_deg: f64,
    pub mirrored: bool,
}

/// The registration of one area.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AreaRegistration {
    /// Stage units of the laser software: "mm" or "um".
    pub units: String,
    pub adapter: String,
    pub fiducials: Vec<Fiducial>,
    pub fit: Option<Fit>,
}

impl Default for AreaRegistration {
    fn default() -> Self {
        AreaRegistration { units: "mm".into(), adapter: "generic".into(), fiducials: Vec::new(), fit: None }
    }
}

/// `project.json` → `laser`: registrations per area id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LaserRegistration {
    pub areas: BTreeMap<String, AreaRegistration>,
}

impl LaserRegistration {
    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }
}

/// Micrometres per stage unit.
pub fn um_per_unit(units: &str) -> Result<f64> {
    match units {
        "mm" => Ok(1000.0),
        "um" | "µm" => Ok(1.0),
        u => Err(Error::Invalid(format!("laser stage units must be mm or um, not {u:?}"))),
    }
}

pub fn apply(m: &[f64; 6], x: f64, y: f64) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// Least-squares affine from the fiducials with both stage values (three or more, not on one
/// line). Points are centred first for a well-conditioned solve.
pub fn fit(fiducials: &[Fiducial], units: &str) -> Result<Fit> {
    let k = um_per_unit(units)?;
    let pts: Vec<([f64; 2], [f64; 2])> = fiducials.iter().filter_map(|f| Some(([f.x_um, f.y_um], [f.x_stage?, f.y_stage?]))).filter(|(a, b)| a.iter().chain(b).all(|v| v.is_finite())).collect();
    let n = pts.len();
    if n < 3 {
        return Err(Error::Invalid(format!("{n} fiducials with stage coordinates; at least 3 are needed")));
    }
    let (mx, my) = (pts.iter().map(|p| p.0[0]).sum::<f64>() / n as f64, pts.iter().map(|p| p.0[1]).sum::<f64>() / n as f64);
    let (sx, sy) = (pts.iter().map(|p| p.1[0]).sum::<f64>() / n as f64, pts.iter().map(|p| p.1[1]).sum::<f64>() / n as f64);
    // Normal equations of [u v] (centred µm) → centred stage, per axis: 2x2.
    let (mut suu, mut suv, mut svv) = (0.0, 0.0, 0.0);
    let (mut sux, mut svx, mut suy, mut svy) = (0.0, 0.0, 0.0, 0.0);
    for (a, b) in &pts {
        let (u, v) = (a[0] - mx, a[1] - my);
        let (x, y) = (b[0] - sx, b[1] - sy);
        suu += u * u;
        suv += u * v;
        svv += v * v;
        sux += u * x;
        svx += v * x;
        suy += u * y;
        svy += v * y;
    }
    let det = suu * svv - suv * suv;
    // Relative test: collinear or coincident points leave the system singular.
    if det.abs() <= 1e-9 * (suu * svv).max(1e-300) || suu == 0.0 || svv == 0.0 {
        return Err(Error::Invalid("the fiducials lie on one line: pick spots spread over the area".into()));
    }
    let a = (sux * svv - svx * suv) / det;
    let c = (svx * suu - sux * suv) / det;
    let b = (suy * svv - svy * suv) / det;
    let d = (svy * suu - suy * suv) / det;
    let e = sx - a * mx - c * my;
    let f = sy - b * mx - d * my;
    let m = [a, b, c, d, e, f];
    let residuals_um: Vec<f64> = pts
        .iter()
        .map(|(p, s)| {
            let q = apply(&m, p[0], p[1]);
            (q[0] - s[0]).hypot(q[1] - s[1]) * k
        })
        .collect();
    let rms_um = (residuals_um.iter().map(|r| r * r).sum::<f64>() / n as f64).sqrt();
    let max_um = residuals_um.iter().copied().fold(0.0, f64::max);
    let linear_det = a * d - b * c;
    Ok(Fit {
        matrix: m,
        rms_um,
        max_um,
        n,
        exact: n == 3,
        warn: max_um > WARN_UM,
        scale: [a.hypot(b), c.hypot(d)],
        rotation_deg: b.atan2(a).to_degrees(),
        mirrored: linear_det < 0.0,
        residuals_um,
    })
}

/// One spot in the laser's frame, as adapters receive it.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LaserRow {
    pub name: String,
    pub x_stage: f64,
    pub y_stage: f64,
    /// End of a line, opposite corner of a raster, in stage units.
    pub x2_stage: Option<f64>,
    pub y2_stage: Option<f64>,
    pub x_um: f64,
    pub y_um: f64,
    pub diameter_um: f64,
    pub kind: &'static str,
    pub grain: String,
    pub sample: String,
}

/// A laser control software's spot list format.
pub trait LaserAdapter: Sync {
    /// Stable id stored in `project.json` (`laser.areas.<id>.adapter`).
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn extension(&self) -> &'static str;
    /// The file text for `rows` (stage values in `units`, "mm" or "um").
    fn write(&self, rows: &[LaserRow], units: &str) -> String;
}

/// The generic CSV: `name, x_stage, y_stage, x_um, y_um, diameter_um, kind, grain, sample`,
/// then `x2_stage, y2_stage` (line ends and raster corners; empty for spots). Stage values in
/// the laser's units, 4 decimals in mm and 2 in µm; RFC 4180 quoting; LF line ends.
pub struct Generic;

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

impl LaserAdapter for Generic {
    fn id(&self) -> &'static str {
        "generic"
    }
    fn label(&self) -> &'static str {
        "Generic CSV"
    }
    fn extension(&self) -> &'static str {
        "csv"
    }
    fn write(&self, rows: &[LaserRow], units: &str) -> String {
        let d = if units == "mm" { 4 } else { 2 };
        let mut s = String::from("name,x_stage,y_stage,x_um,y_um,diameter_um,kind,grain,sample,x2_stage,y2_stage\n");
        let opt = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.d$}"));
        for r in rows {
            s.push_str(&format!(
                "{},{:.d$},{:.d$},{:.2},{:.2},{},{},{},{},{},{}\n",
                csv_field(&r.name),
                r.x_stage,
                r.y_stage,
                r.x_um,
                r.y_um,
                r.diameter_um,
                r.kind,
                csv_field(&r.grain),
                csv_field(&r.sample),
                opt(r.x2_stage),
                opt(r.y2_stage),
            ));
        }
        s
    }
}

static GENERIC: Generic = Generic;

/// Every adapter, the generic one first. Add the lab's laser software here.
pub fn adapters() -> Vec<&'static dyn LaserAdapter> {
    vec![&GENERIC]
}

pub fn adapter(id: &str) -> Result<&'static dyn LaserAdapter> {
    adapters().into_iter().find(|a| a.id() == id).ok_or_else(|| Error::NotFound(format!("laser adapter {id}")))
}

/// The registered rows of one area's spots.
pub fn rows(reg: &AreaRegistration, spots: &[Spot], area_id: &str) -> Result<Vec<LaserRow>> {
    let fit = reg.fit.as_ref().ok_or_else(|| Error::Invalid(format!("{area_id} has no laser registration yet")))?;
    let m = &fit.matrix;
    Ok(spots
        .iter()
        .filter(|s| s.area_id == area_id)
        .map(|s| {
            let p = apply(m, s.x_um, s.y_um);
            let q = match (s.x2_um, s.y2_um) {
                (Some(x2), Some(y2)) if s.kind != SpotKind::Spot => Some(apply(m, x2, y2)),
                _ => None,
            };
            LaserRow {
                name: s.name.clone(),
                x_stage: p[0],
                y_stage: p[1],
                x2_stage: q.map(|q| q[0]),
                y2_stage: q.map(|q| q[1]),
                x_um: s.x_um,
                y_um: s.y_um,
                diameter_um: s.diameter_um,
                kind: match s.kind {
                    SpotKind::Spot => "spot",
                    SpotKind::Line => "line",
                    SpotKind::Raster => "raster",
                },
                grain: s.grain_number.map_or(String::new(), |n| n.to_string()),
                sample: s.sample.clone(),
            }
        })
        .collect())
}

/// The spot list of one area in the registration's adapter (or `adapter_id` when given).
pub fn export(project: &Project, spots: &[Spot], area_id: &str, adapter_id: Option<&str>) -> Result<String> {
    let reg = project.laser.areas.get(area_id).ok_or_else(|| Error::Invalid(format!("{area_id} has no laser registration yet")))?;
    let a = adapter(adapter_id.unwrap_or(&reg.adapter))?;
    Ok(a.write(&rows(reg, spots, area_id)?, &reg.units))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fid(x: f64, y: f64, m: &[f64; 6]) -> Fiducial {
        let s = apply(m, x, y);
        Fiducial { spot_id: None, name: String::new(), x_um: x, y_um: y, x_stage: Some(s[0]), y_stage: Some(s[1]) }
    }

    /// 1° rotation, y flipped, µm → mm, offset.
    fn truth() -> [f64; 6] {
        let (c, s) = (1f64.to_radians().cos() * 1e-3, 1f64.to_radians().sin() * 1e-3);
        [c, s, s, -c, 12.5, 40.25]
    }

    #[test]
    fn three_points_recover_the_map_exactly() {
        let m = truth();
        let f = fit(&[fid(100.0, 80.0, &m), fid(2900.0, 150.0, &m), fid(1200.0, 1700.0, &m)], "mm").unwrap();
        for (a, b) in f.matrix.iter().zip(m) {
            assert!((a - b).abs() < 1e-12, "{a} vs {b}");
        }
        assert!(f.exact && !f.warn && f.mirrored);
        assert!(f.max_um < 1e-6);
        assert!((f.scale[0] - 1e-3).abs() < 1e-12);
        assert!((f.rotation_deg - 1.0).abs() < 1e-9);
    }

    #[test]
    fn least_squares_with_a_bad_reading_warns() {
        let m = truth();
        let mut fs = vec![fid(100.0, 80.0, &m), fid(2900.0, 150.0, &m), fid(1200.0, 1700.0, &m), fid(2500.0, 1600.0, &m)];
        fs[3].x_stage = fs[3].x_stage.map(|v| v + 0.03); // 30 µm off
        let f = fit(&fs, "mm").unwrap();
        assert_eq!(f.n, 4);
        assert!(!f.exact);
        assert!(f.warn);
        assert!(f.max_um > WARN_UM && f.max_um < 30.0);
    }

    #[test]
    fn too_few_or_collinear_fiducials_are_refused() {
        let m = truth();
        assert!(fit(&[fid(0.0, 0.0, &m), fid(1.0, 1.0, &m)], "mm").is_err());
        assert!(fit(&[fid(0.0, 0.0, &m), fid(100.0, 100.0, &m), fid(300.0, 300.0, &m)], "mm").is_err());
        let mut incomplete = vec![fid(100.0, 80.0, &m), fid(2900.0, 150.0, &m), fid(1200.0, 1700.0, &m)];
        incomplete[2].y_stage = None;
        assert!(fit(&incomplete, "mm").is_err());
        assert!(fit(&[fid(100.0, 80.0, &m), fid(2900.0, 150.0, &m), fid(1200.0, 1700.0, &m)], "inch").is_err());
    }

    #[test]
    fn generic_csv_in_the_lasers_units() {
        let rows = vec![LaserRow {
            name: "HJ-GC1-1-1".into(),
            x_stage: 12.6,
            y_stage: 40.1,
            x2_stage: None,
            y2_stage: None,
            x_um: 100.0,
            y_um: 80.0,
            diameter_um: 25.0,
            kind: "spot",
            grain: "1".into(),
            sample: "HJ, GC1".into(),
        }];
        let csv = Generic.write(&rows, "mm");
        assert_eq!(csv, "name,x_stage,y_stage,x_um,y_um,diameter_um,kind,grain,sample,x2_stage,y2_stage\nHJ-GC1-1-1,12.6000,40.1000,100.00,80.00,25,spot,1,\"HJ, GC1\",,\n");
        assert_eq!(adapter("generic").unwrap().extension(), "csv");
        assert!(adapter("qtegra-laser").is_err());
    }
}

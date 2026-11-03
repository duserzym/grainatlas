//! Spots (plan section 6.7): the spot record, the naming pattern with
//! continuation, and the CSV and GeoJSON exports. A spot carries its UUIDv7
//! from birth so it can become a laicpmsdb `physical_spot` without translation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::Project;
use crate::segment::Grain;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub naming_pattern: String,
    pub spot_presets_um: Vec<f64>,
    pub default_diameter_um: f64,
    pub user: String,
    pub theme: String,
    pub snap: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            naming_pattern: "{sample}-{grain}-{n}".into(),
            spot_presets_um: vec![15.0, 20.0, 25.0, 30.0, 35.0, 50.0],
            default_diameter_um: 25.0,
            user: String::new(),
            theme: "dark".into(),
            snap: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpotKind {
    #[default]
    Spot,
    Line,
    Raster,
}

/// A laser spot, line or raster in mosaic micrometres (y down).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Spot {
    pub id: Uuid,
    pub area_id: String,
    pub name: String,
    #[serde(default)]
    pub sample: String,
    pub grain_id: Option<Uuid>,
    pub grain_number: Option<u32>,
    /// The `{n}` this spot was named with (continuation counter).
    #[serde(default)]
    pub n: u32,
    pub x_um: f64,
    pub y_um: f64,
    /// End point of a line, opposite corner of a raster.
    #[serde(default)]
    pub x2_um: Option<f64>,
    #[serde(default)]
    pub y2_um: Option<f64>,
    pub diameter_um: f64,
    #[serde(default)]
    pub kind: SpotKind,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub created_by: String,
}

/// A free text label on the canvas.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Label {
    pub id: Uuid,
    pub area_id: String,
    pub x_um: f64,
    pub y_um: f64,
    pub text: String,
}

/// Expands a naming pattern: `{sample}`, `{grain}`, `{n}`, and zero-padded `{n:03}` /
/// `{grain:02}`. A spot outside any grain uses `0` for `{grain}`.
pub fn format_name(pattern: &str, sample: &str, grain: Option<u32>, n: u32) -> String {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let token = &rest[i + 1..i + j];
        let (key, width) = match token.split_once(':') {
            Some((k, w)) => (k, w.trim_start_matches('0').parse::<usize>().ok().or(Some(w.len())).filter(|_| w.starts_with('0'))),
            None => (token, None),
        };
        let val = match key {
            "sample" => Some(sample.to_string()),
            "grain" => Some(grain.unwrap_or(0).to_string()),
            "n" => Some(n.to_string()),
            _ => None,
        };
        match val {
            Some(v) => match width {
                Some(wd) if key != "sample" => out.push_str(&format!("{v:0>wd$}")),
                _ => out.push_str(&v),
            },
            None => out.push_str(&rest[i..i + j + 1]),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// The next `{n}`: one more than the highest in the project.
pub fn next_n(spots: &[Spot]) -> u32 {
    spots.iter().map(|s| s.n).max().unwrap_or(0) + 1
}

/// The grain whose polygon contains a point.
pub fn grain_at<'a>(grains: &'a [Grain], x: f64, y: f64) -> Option<&'a Grain> {
    grains.iter().find(|g| x >= g.bbox_um[0] && x <= g.bbox_um[2] && y >= g.bbox_um[1] && y <= g.bbox_um[3] && crate::segment::shape::inside(&g.polygon_um, x, y))
}

/// Creates a spot at a point, named from the pattern and linked to the grain under it.
#[allow(clippy::too_many_arguments)]
pub fn new_spot(area_id: &str, sample: &str, grains: &[Grain], existing: &[Spot], settings: &Settings, x_um: f64, y_um: f64, diameter_um: f64, created: &str) -> Spot {
    let g = grain_at(grains, x_um, y_um);
    let n = next_n(existing);
    Spot {
        id: crate::project::new_id(),
        area_id: area_id.to_string(),
        name: format_name(&settings.naming_pattern, sample, g.map(|g| g.number), n),
        sample: sample.to_string(),
        grain_id: g.map(|g| g.id),
        grain_number: g.map(|g| g.number),
        n,
        x_um,
        y_um,
        x2_um: None,
        y2_um: None,
        diameter_um,
        kind: SpotKind::Spot,
        comment: String::new(),
        created: created.to_string(),
        created_by: settings.user.clone(),
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Spot list CSV (plan requirement 10): ids, names, grain, position in um, px and stage,
/// diameter, kind, comment and provenance.
pub fn spots_csv(project: &Project, spots: &[Spot]) -> String {
    let mut s = String::from(
        "id,name,sample,area,grain,grain_id,x_um,y_um,x_px,y_px,diameter_um,kind,x2_um,y2_um,stage_x_nm,stage_y_nm,comment,created,created_by,area_uuid,project_uuid,laicpmsdb_mount_id\n",
    );
    for sp in spots {
        let area = project.areas.iter().find(|a| a.id == sp.area_id);
        let k = area.map_or(1.0, |a| a.um_per_px());
        let stage = area.and_then(|a| a.stage_of(sp.x_um, sp.y_um));
        let f = |v: Option<f64>, d: usize| v.map_or(String::new(), |v| format!("{v:.d$}"));
        s.push_str(&format!(
            "{},{},{},{},{},{},{:.2},{:.2},{:.1},{:.1},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            sp.id,
            csv_field(&sp.name),
            csv_field(&sp.sample),
            csv_field(&sp.area_id),
            sp.grain_number.map_or(String::new(), |n| n.to_string()),
            sp.grain_id.map_or(String::new(), |g| g.to_string()),
            sp.x_um,
            sp.y_um,
            sp.x_um / k,
            sp.y_um / k,
            sp.diameter_um,
            match sp.kind {
                SpotKind::Spot => "spot",
                SpotKind::Line => "line",
                SpotKind::Raster => "raster",
            },
            f(sp.x2_um, 2),
            f(sp.y2_um, 2),
            stage.map_or(String::new(), |s| format!("{:.0}", s[0])),
            stage.map_or(String::new(), |s| format!("{:.0}", s[1])),
            csv_field(&sp.comment),
            csv_field(&sp.created),
            csv_field(&sp.created_by),
            area.map_or(String::new(), |a| a.uuid.to_string()),
            project.uuid,
            project.laicpmsdb.mount_id.map_or(String::new(), |m| m.to_string()),
        ));
    }
    s
}

/// GeoJSON FeatureCollection of grain outlines and spots in mosaic micrometres (x right,
/// y down, origin at the mosaic's top-left; not geographic).
pub fn geojson(project: &Project, grains: &BTreeMap<String, Vec<Grain>>, spots: &[Spot]) -> serde_json::Value {
    use serde_json::json;
    let mut features = Vec::new();
    for (area_id, gs) in grains {
        let area_uuid = project.areas.iter().find(|a| &a.id == area_id).map(|a| a.uuid.to_string());
        for g in gs {
            let mut ring: Vec<[f64; 2]> = g.polygon_um.clone();
            if let Some(f) = ring.first().copied() {
                ring.push(f);
            }
            features.push(json!({
                "type": "Feature",
                "id": g.id,
                "geometry": { "type": "Polygon", "coordinates": [ring] },
                "properties": {
                    "kind": "grain", "number": g.number, "area": area_id, "area_uuid": area_uuid,
                    "area_um2": g.area_um2, "eq_diameter_um": g.eq_diameter_um, "major_um": g.major_um, "minor_um": g.minor_um,
                    "orientation_deg": g.orientation_deg, "scale_bar_um": g.scale_bar_um, "manual": g.manual,
                }
            }));
        }
    }
    for sp in spots {
        let geometry = match (sp.kind, sp.x2_um, sp.y2_um) {
            (SpotKind::Line, Some(x2), Some(y2)) => json!({ "type": "LineString", "coordinates": [[sp.x_um, sp.y_um], [x2, y2]] }),
            (SpotKind::Raster, Some(x2), Some(y2)) => json!({ "type": "Polygon", "coordinates": [[[sp.x_um, sp.y_um], [x2, sp.y_um], [x2, y2], [sp.x_um, y2], [sp.x_um, sp.y_um]]] }),
            _ => json!({ "type": "Point", "coordinates": [sp.x_um, sp.y_um] }),
        };
        features.push(json!({
            "type": "Feature",
            "id": sp.id,
            "geometry": geometry,
            "properties": {
                "kind": match sp.kind { SpotKind::Spot => "spot", SpotKind::Line => "line", SpotKind::Raster => "raster" },
                "name": sp.name, "sample": sp.sample, "area": sp.area_id, "grain": sp.grain_number, "grain_id": sp.grain_id,
                "diameter_um": sp.diameter_um, "comment": sp.comment, "created": sp.created, "created_by": sp.created_by,
            }
        }));
    }
    json!({
        "type": "FeatureCollection",
        "name": project.name,
        "grainmap": { "project_uuid": project.uuid, "units": "micrometre", "frame": "mosaic, origin top-left, y down", "laicpmsdb_mount_id": project.laicpmsdb.mount_id },
        "features": features,
    })
}

use super::*;
const BASE: &[(&str, &str)] = &[
    ("grain_id", "Grain UUID"),
    ("grain_label", "Stable grain label"),
    ("sample", "Per-grain sample, or source area sample"),
    ("source_area_id", "Area owning the polygon"),
    ("area_id", "Sampled layer area identifier"),
    ("area_name", "Sampled layer area name"),
    (
        "tile_index",
        "SEM tile index; overlap ties use nearest centre then lowest index; blank if unknown",
    ),
    ("layer", "Sampled area identifier, long format"),
    (
        "pixel_source",
        "original-f32, displayed-8bit or published-8bit",
    ),
    ("spot_names", "Linked spot names separated by semicolons"),
    ("session", "Known session metadata, blank otherwise"),
    (
        "acquisition_date",
        "Known date/time metadata, blank otherwise",
    ),
    ("signal", "Known metadata signal; blank if unknown"),
    (
        "mount_reference_group",
        "Mount reference identity: signal policy and pixel source",
    ),
    (
        "source_area_uuid",
        "Full UUID of the area owning the polygon",
    ),
    ("area_uuid", "Full UUID of the sampled layer area"),
    (
        "centroid_x_um",
        "Sampled mosaic centroid, x right, micrometres",
    ),
    (
        "centroid_y_um",
        "Sampled mosaic centroid, y down, micrometres",
    ),
    (
        "stage_x_um",
        "Instrument stage x, micrometres; blank without stage data",
    ),
    (
        "stage_y_um",
        "Instrument stage y, micrometres; blank without stage data",
    ),
    ("area_um2", "Existing segmented area, square micrometres"),
    ("perimeter_um", "Closed polygon perimeter, micrometres"),
    (
        "eq_diameter_um",
        "Existing equivalent diameter, micrometres",
    ),
    ("major_um", "Existing major axis, micrometres"),
    ("minor_um", "Existing minor axis, micrometres"),
    ("aspect_ratio", "Major / minor; blank for zero minor"),
    (
        "orientation_deg",
        "From +x towards +y (down) in sampled mosaic, -90..90 degrees",
    ),
    ("n_px", "Pixel centres inside the polygon"),
    ("mean", "Mean, 0..1"),
    ("median", "Median, 0..1"),
    ("sd", "Sample standard deviation, ddof=1"),
    ("min", "Minimum, 0..1"),
    ("max", "Maximum, 0..1"),
    ("p5", "5th percentile, linear interpolation"),
    ("p25", "25th percentile, linear interpolation"),
    ("p75", "75th percentile, linear interpolation"),
    ("p95", "95th percentile, linear interpolation"),
    ("iqr", "p75 - p25"),
    ("skewness", "Adjusted Fisher-Pearson G1"),
    (
        "glcm_contrast",
        "Mean of four directional symmetric GLCM contrasts, 32 levels",
    ),
    ("glcm_homogeneity", "Mean sum(Pij / (1 + (i-j)^2))"),
];
pub fn columns() -> Vec<Value> {
    let mut v: Vec<_> = BASE
        .iter()
        .map(|(n, d)| json!({"name":n,"description":d}))
        .collect();
    for scope in ["area", "tile", "mount"] {
        for measure in ["mean", "median"] {
            for (metric, description) in [
                (
                    "n",
                    "Number of non-artifact reference grains with this measure",
                ),
                ("z", "(x - reference mean) / sample sd"),
                ("robust_z", "(x - reference median) / (1.4826 * MAD)"),
                (
                    "percentile",
                    "100 * (count below x + 0.5 * count equal x) / n",
                ),
            ] {
                v.push(json!({"name":format!("{scope}_{measure}_{metric}"),"description":format!("{scope} reference of grain {measure}: {description}; tile measures blank for n < 3, area/mount for n < 2")}));
            }
        }
    }
    for (n, d) in [
        ("crop_file", "Relative crop file path"),
        ("mask_file", "Relative mask file path"),
        (
            "crop_x_px",
            "Left crop edge in sampled mosaic pixels; blank without images",
        ),
        (
            "crop_y_px",
            "Top crop edge in sampled mosaic pixels; blank without images",
        ),
    ] {
        v.push(json!({"name":n,"description":d}));
    }
    v
}
pub(super) fn field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.into()
    }
}
fn num(x: Option<f64>) -> String {
    x.filter(|v| v.is_finite())
        .map(|v| v.to_string())
        .unwrap_or_default()
}
impl Export {
    pub(super) fn csv(&self, ctl: &Ctl) -> Result<String> {
        let cols = columns();
        let mut text = cols
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join(",");
        text.push_str("\r\n");
        for r in self.rows.iter().filter(|r| r.selected) {
            ctl.check()?;
            let g = &r.grain;
            let a = self.area(&r.area);
            let own = self.area(&r.source_area);
            let stage = a.stage_of(g.centroid_um[0], g.centroid_um[1]);
            let names = self
                .spots
                .iter()
                .filter(|s| s.area_id == r.source_area && s.grain_id == Some(g.id))
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(";");
            let meta = acquisition(a, &self.project);
            let find = |needle: &str| {
                meta.iter()
                    .filter(|(k, _)| k.to_ascii_lowercase().contains(needle))
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(";")
            };
            let perimeter = g
                .polygon_um
                .iter()
                .zip(g.polygon_um.iter().cycle().skip(1))
                .take(g.polygon_um.len())
                .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]))
                .sum::<f64>();
            let mut row = vec![
                g.id.to_string(),
                grain_label(g),
                g.sample.clone().unwrap_or_else(|| own.sample.clone()),
                r.source_area.clone(),
                r.area.clone(),
                a.name.clone(),
                r.tile.map(|t| t.to_string()).unwrap_or_default(),
                r.area.clone(),
                r.pixel_source.clone(),
                names,
                find("session"),
                {
                    let date = find("date");
                    if date.is_empty() { find("time") } else { date }
                },
                crate::segment::signal_from_meta(&a.area_meta)
                    .or_else(|| crate::segment::signal_from_meta(&self.project.package.meta))
                    .map(|s| s.label().to_string())
                    .unwrap_or_default(),
                self.group(r, "mount"),
                own.uuid.to_string(),
                a.uuid.to_string(),
            ];
            row.extend(
                [
                    Some(g.centroid_um[0]),
                    Some(g.centroid_um[1]),
                    stage.map(|p| p[0] / 1000.0),
                    stage.map(|p| p[1] / 1000.0),
                    Some(g.area_um2),
                    Some(perimeter),
                    Some(g.eq_diameter_um),
                    Some(g.major_um),
                    Some(g.minor_um),
                    (g.minor_um > 0.0).then(|| g.major_um / g.minor_um),
                    Some(g.orientation_deg),
                ]
                .map(num),
            );
            row.push(r.stats.n_px.to_string());
            let s = &r.stats;
            row.extend(
                [
                    s.mean,
                    s.median,
                    s.sd,
                    s.min,
                    s.max,
                    s.p5,
                    s.p25,
                    s.p75,
                    s.p95,
                    s.iqr,
                    s.skewness,
                    s.glcm_contrast,
                    s.glcm_homogeneity,
                ]
                .map(num),
            );
            for scope in ["area", "tile", "mount"] {
                for (j, value) in [s.mean, s.median].into_iter().enumerate() {
                    let rel = self
                        .refs
                        .get(&self.group(r, scope))
                        .map(|v| v[j].of(value, if scope == "tile" { 3 } else { 2 }));
                    if scope == "tile" && r.tile.is_none() {
                        row.extend([String::new(), String::new(), String::new(), String::new()]);
                    } else if let Some(rel) = rel {
                        row.extend([
                            rel.n.to_string(),
                            num(rel.z),
                            num(rel.robust_z),
                            num(rel.percentile),
                        ]);
                    } else {
                        row.extend(["0".into(), String::new(), String::new(), String::new()]);
                    }
                }
            }
            let has_image = !r.crop.is_empty() || !r.mask.is_empty();
            row.extend([
                r.crop.clone(),
                r.mask.clone(),
                if has_image {
                    r.origin[0].to_string()
                } else {
                    String::new()
                },
                if has_image {
                    r.origin[1].to_string()
                } else {
                    String::new()
                },
            ]);
            debug_assert_eq!(row.len(), cols.len());
            text.push_str(&row.iter().map(|s| field(s)).collect::<Vec<_>>().join(","));
            text.push_str("\r\n");
        }
        Ok(text)
    }
    pub(super) fn outlines(&self, ctl: &Ctl) -> Result<Value> {
        let mut features = Vec::new();
        for r in self.rows.iter().filter(|r| r.selected && r.reference) {
            ctl.check()?;
            let g = &r.grain;
            let a = self.area(&r.area);
            let points: Option<Vec<_>> = g
                .polygon_um
                .iter()
                .map(|p| {
                    a.stage_of(p[0], p[1])
                        .map(|p| [p[0] / 1000.0, p[1] / 1000.0])
                })
                .collect();
            let geometry = points.map(|mut p| {
                if p.first() != p.last() {
                    p.push(p[0]);
                }
                json!({"type":"Polygon","coordinates":[p]})
            });
            features.push(json!({"type":"Feature","geometry":geometry,"properties":{"grain_id":g.id,"label":grain_label(g),"sample":g.sample.as_ref().unwrap_or(&a.sample),"area_id":a.id,"spot_names":self.spots.iter().filter(|s|s.grain_id==Some(g.id)&&s.area_id==a.id).map(|s|s.name.clone()).collect::<Vec<_>>().join(";"),"coordinate_status":if geometry.is_some(){"stage"}else{"unavailable"}}}));
        }
        Ok(
            json!({"type":"FeatureCollection","coordinate_frame":{"name":"instrument stage / mount","units":"micrometres","axes":"Instrument stage +x and +y; not geographic longitude/latitude. Per-area stage_sign in manifest relates mosaic right/down to stage. Geometry is null without stage metadata."},"features":features}),
        )
    }
}
pub fn readme() -> String {
    let mut s = String::from(
        r#"# Segmented grain export

`grains.csv` has one row per non-artifact grain per sampled layer (long format).
`crops/` holds bounding boxes plus the requested margin, clipped to the mosaic.
`masks/` holds separate 8-bit PNGs (255 inside, 0 outside), at the same origin
and size as the crop. Either image directory may be omitted. Pixels outside the
mask remain in crops as context; exclude them when analysing a grain.
Image names use the sanitised source area name, grain label and last eight
hex digits of the grain UUID. Additional layers append the sanitised name of
the sampled area; collisions append a counter (starting at 2).
TIFF crops use .tiff; masks always use .png. Full grain and area UUIDs remain
in grains.csv, so the short names are not identifiers.
`outlines.geojson` contains original grain polygons in instrument stage/mount
micrometres (not geographic coordinates); unavailable stage geometry is null.
`manifest.json` records options, provenance, corrections, hashes and columns.
CSV loads directly in pandas (`read_csv`), polars (`read_csv`) and R (`read.csv`).
Parquet is omitted to keep native and wasm exports dependency-light.

Original pixels are the f32 stitched mosaic before tone, not detector counts.
It already carries configured gain/offset and background leveling. The joint
background solve can include a smooth charging surface and additive beam-overlap
steps, with a common epoxy level; the recorded fallback may use gain/offset only.
It also carries manual tile overrides, optional HDR merge, solved placement, feather/multiband
blending and clamping to 0..1. There is no additional flat-field calibration.
The manifest stores parameters and applied photometric corrections per area.
Original images are grayscale 16-bit PNG or TIFF: round(clamp(v,0,1)*65535).
Divide by 65535 to recover the 0..1 scale, with quantisation error.
The image crate does not encode grayscale float TIFF, so float TIFF is omitted.
Displayed pixels include percentile stretch, optional CLAHE and per-grain
normalisation in the base, then black/white/gamma from the current display LUT,
in 8-bit.
Bundle-backed areas have only published 8-bit pixels; original precision cannot
be recovered. Their rows and areas say `published-8bit`, even in original mode.
Statistics use the full f32 values or exact u8/255, before 16-bit crop rounding.
SHA-256 covers the exact full mosaic sampled: row-major f32 little-endian bytes
or final u8 bytes, without a size/header. Hashes include pixels outside grains.

Pixels enter a mask if their centres ((x+0.5)*pixel_size, (y+0.5)*pixel_size)
pass an even-odd polygon test: a horizontal ray crosses edges whose y endpoints
straddle the centre (lower inclusive, upper exclusive), strictly to its right.
At polygon boundaries this half-open rule is deterministic. All such pixels,
including any uncovered mosaic fill inside own-area polygons, enter statistics.
Cross-area layers require full coverage and stage transforms agreeing across
real tiles within 0.1 pixel. A piecewise stage map is not assumed registered.
Source UUID/label/sample remain unchanged; area/layer/tile and centroid describe
the sampled target mosaic. Mapped copies do not enter reference populations.
SEM tile is the containing tile nearest the centroid (ties: lowest tile index).
Published mosaics lack trustworthy SEM tile placement: tile fields stay blank.
Mosaic axes are x right, y down. Stage axes follow the instrument; stage_sign
records their relation to mosaic axes. stage_of returns nm, converted to um.
Orientation is the existing major axis angle from +x towards +y, -90..90 degrees.

Statistics: mean and median; sample sd = sqrt(sum((x-mean)^2)/(n-1)); min/max;
p5/p25/p75/p95 using h=(n-1)*p, linear interpolation of adjacent sorted values;
IQR=p75-p25. Adjusted Fisher-Pearson skewness G1 =
n/((n-1)*(n-2)) * sum(((x-mean)/sample_sd)^3), n>=3, positive sd.
Undefined measures are empty cells, never NaN. Single pixels retain mean and
quantiles but have no sd or skewness. Empty masks have n_px=0.
Texture uses 32 levels between the grain's own p1 and p99 (the same linear
percentile method): floor(32*(x-p1)/(p99-p1)), clipped to 0..31. A constant grain
uses level 0. GLCMs use distance 1 pixel, directions (1,0),(0,1),(1,1),(-1,1),
symmetric counts and only pairs with both pixel centres inside the mask.
Each directional matrix is separately normalised. Contrast=sum(Pij*(i-j)^2),
homogeneity=sum(Pij/(1+(i-j)^2)); average directions that have at least one pair.
No pairs means both texture cells are empty.

Relative values are calculated separately for grain mean and grain median.
z=(x-reference_mean)/reference_sample_sd.
robust_z=(x-reference_median)/(1.4826*median(abs(reference-reference_median))).
Percentile rank=100*(count(reference<x)+0.5*count(reference=x))/n (mid-rank ties).
All reference sets include every native non-artifact grain with a defined
measure, regardless of export selection. Area references use the sampled area;
tile references use its centroid tile; mount references partition by known
metadata signal/detector if every area contributing native reference grains has a known signal, otherwise
pool all areas. Pixel-source kinds are always separated at mount level.
Area/mount values require n>=2; tile values require n>=3. Zero sd or MAD leaves
its z cell empty; percentile ranks remain defined when the reference is large
enough. The n columns count reference values, not exported rows.
Relative measures are comparable only within the stated reference set.
Use area_id and tile_index as grouping factors (for example a random intercept
for image and a tile intercept nested within image), with signal/session as
covariates when known. This does not make absolute intensity calibrated across
sessions. Grain UUID identifies repeated layers of the same physical detection.

Column order and dictionary (also machine-readable in manifest.json):

"#,
    );
    for v in columns() {
        s.push_str(&format!(
            "- `{}`: {}\n",
            v["name"].as_str().unwrap(),
            v["description"].as_str().unwrap()
        ));
    }
    s
}

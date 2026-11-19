//! Procedural input for the frozen canvas regression and the baseline comparison probe.
//! Only synthetic pixels; no package files or user metadata.
use crate::project::AreaParams;
use crate::source::MemSource;

pub(super) fn package() -> MemSource {
    let mut src = MemSource::new("Synthetic canvas regression");
    // Odd dimensions/origins, horizontal and vertical overlaps, a contained tile,
    // a disconnected island and holes exercise seam direction, ties and band margins.
    let rects = [
        (0, 0, 193, 161),
        (129, 7, 193, 161),
        (9, 105, 193, 161),
        (130, 110, 193, 161),
        (55, 47, 95, 87),
        (350, 289, 53, 47),
    ];
    let mut csv = String::from("file,x_um,y_um,pixel_size_nm\n");
    for (i, (x0, y0, w, h)) in rects.into_iter().enumerate() {
        let mut bytes = Vec::with_capacity(w * h * 2);
        for y in 0..h {
            for x in 0..w {
                let (gx, gy) = (x + x0, y + y0);
                let noise = ((gx as u32).wrapping_mul(73_856_093)
                    ^ (gy as u32).wrapping_mul(19_349_663))
                    % 1800;
                let (dx, dy) = ((gx % 67) as i64 - 31, (gy % 59) as i64 - 27);
                let grain = if dx * dx + dy * dy < 13 * 13 {
                    26_000
                } else {
                    0
                };
                let background = 11_000 + gx * 9 + gy * 5 + noise as usize;
                let step = if x > w * 2 / 3 { 1100 } else { 0 };
                let value =
                    ((background + grain) * (92 + i * 3) / 100 + i * 390 + step).min(65535) as u16;
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, w as u32, h as u32);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Sixteen);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&bytes)
                .unwrap();
        }
        src.insert(&format!("area/tile{i}.png"), png);
        csv.push_str(&format!("tile{i}.png,{x0},{y0},1000\n"));
    }
    src.insert("area/positions.csv", csv.into_bytes());
    src
}

pub(super) fn cases() -> Vec<(&'static str, AreaParams)> {
    use crate::blend::BlendMode;
    use crate::photometric::{GainModel, background::LevelMethod};
    let default = AreaParams::default();
    let mut joint = default.clone();
    joint.photometric.background_params.cell = 32;
    joint.blend.seam_factor = 3;
    joint.blend.levels = 4;
    let mut tile = joint.clone();
    tile.photometric.background_params.method = LevelMethod::Tile;
    let mut feather = joint.clone();
    feather.blend.mode = BlendMode::Feather;
    let mut gain = joint.clone();
    gain.photometric.model = GainModel::Gain;
    let mut none = joint.clone();
    none.photometric.model = GainModel::None;
    let mut clahe = joint.clone();
    clahe.tone.clahe = true;
    clahe.tone.clahe_tile = 32;
    vec![
        ("default", default),
        ("joint_multiband", joint),
        ("tile_multiband", tile),
        ("joint_feather", feather),
        ("gain_multiband", gain),
        ("none_multiband", none),
        ("clahe_multiband", clahe),
    ]
}

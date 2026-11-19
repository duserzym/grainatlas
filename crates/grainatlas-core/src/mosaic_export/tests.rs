use super::*;
use ::tiff::decoder::{Decoder, DecodingResult};
use std::{
    io::Cursor,
    sync::atomic::{AtomicBool, Ordering},
};
#[test]
fn tiled_classic_and_big_round_trip_8_16_gray_rgb_and_edges() {
    for big in [false, true] {
        for bits in [8, 16] {
            for channels in [1, 3] {
                for (w, h) in [(17, 19), (513, 517)] {
                    let mut l = tiff::Layout::new(w, h, bits, channels).unwrap();
                    l.big = big;
                    let mut bytes = l.header([0.5, 0.75], "{\"synthetic\":true}").unwrap();
                    for tile in 0..l.tiles {
                        let mut b = vec![0; l.tile_bytes as usize];
                        for y in 0..512 {
                            for x in 0..512 {
                                for c in 0..channels as usize {
                                    let value = (((tile % l.cols() * 512 + x)
                                        + (tile / l.cols() * 512 + y) * 3
                                        + c as u64 * 7)
                                        % 251)
                                        as u16;
                                    let i = (y * 512 + x) as usize * channels as usize + c;
                                    if bits == 8 {
                                        b[i] = value as u8;
                                    } else {
                                        b[i * 2..i * 2 + 2]
                                            .copy_from_slice(&(value * 257).to_le_bytes());
                                    }
                                }
                            }
                        }
                        bytes.extend(b);
                    }
                    let mut decoder = Decoder::new(Cursor::new(bytes)).unwrap();
                    assert_eq!(decoder.dimensions().unwrap(), (w as u32, h as u32));
                    let samples: Vec<u16> = match decoder.read_image().unwrap() {
                        DecodingResult::U8(v) => v.into_iter().map(|v| v as u16).collect(),
                        DecodingResult::U16(v) => v,
                        _ => panic!("sample type"),
                    };
                    for y in 0..h {
                        for x in 0..w {
                            for c in 0..channels as u64 {
                                assert_eq!(
                                    samples[((y * w + x) * channels as u64 + c) as usize],
                                    ((x + y * 3 + c * 7) % 251) as u16
                                        * if bits == 16 { 257 } else { 1 }
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
fn options(values: Values, whole_mount: bool) -> String {
    serde_json::to_string(&Options {
        area_ids: vec!["a".into(), "b".into()],
        whole_mount,
        values,
        display_tone: Default::default(),
        format: Format::Tiff,
        png_bits: 16,
        quality: 90,
        size_choice: SizeChoice::Full,
        downsample: 1,
    })
    .unwrap()
}
fn collect(job: &mut Export, s: &mut Session) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    while let Some(c) = job.next(s, &Ctl::NONE).unwrap() {
        files.entry(c.name).or_default().extend(c.data);
    }
    files
}
#[test]
fn export_matches_stitched_pixels_and_centroid_stage_frame() {
    let (mut s, p) = crate::grain_export::synthetic::mount();
    let expected: Vec<_> = s.outputs["a"]
        .linear
        .px
        .iter()
        .map(|v| (*v as f64 * 65535.0).round() as u16)
        .collect();
    let mut job = Export::begin(&s, &p.to_json(), &options(Values::Original, false)).unwrap();
    let files = collect(&mut job, &mut s);
    let mut d = Decoder::new(Cursor::new(
        &files["Synthetic mount_Synthetic a_mosaic.tiff"],
    ))
    .unwrap();
    let DecodingResult::U16(actual) = d.read_image().unwrap() else {
        panic!()
    };
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(a, b, "pixel {i}");
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&files["Synthetic mount_Synthetic a_mosaic.json"]).unwrap();
    let a = &p.areas[0];
    let t = affine(a).unwrap();
    let grains: Vec<crate::segment::Grain> =
        serde_json::from_value(p.extra["grainatlas_ui"]["grains"]["a"].clone()).unwrap();
    let g = &grains[0];
    // Read the centroid's stage coordinates from the real grain export table.
    let mut grains_job = crate::grain_export::Export::begin(
        &s,
        &p.to_json(),
        &json!({"include_crops":false,"include_masks":false,"selected":[g.id],"area_id":"a"})
            .to_string(),
    )
    .unwrap();
    loop {
        if grains_job.batch(&mut s, &Ctl::NONE).unwrap().done {
            break;
        }
    }
    let grain_files = grains_job.finish(&Ctl::NONE).unwrap();
    let csv = std::str::from_utf8(
        &grain_files
            .iter()
            .find(|(name, _)| name == "grains.csv")
            .unwrap()
            .1,
    )
    .unwrap();
    let mut lines = csv.lines();
    let header: Vec<_> = lines.next().unwrap().split(',').collect();
    let id_col = header.iter().position(|h| *h == "grain_id").unwrap();
    let x_col = header.iter().position(|h| *h == "stage_x_um").unwrap();
    let y_col = header.iter().position(|h| *h == "stage_y_um").unwrap();
    let id = g.id.to_string();
    let row: Vec<_> = lines
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|row| row[id_col] == id)
        .unwrap();
    let stage = [
        row[x_col].parse::<f64>().unwrap() * 1000.0,
        row[y_col].parse::<f64>().unwrap() * 1000.0,
    ];
    let col = (stage[0] / 1000.0 - t[2]) / t[0];
    let row = (stage[1] / 1000.0 - t[5]) / t[4];
    assert!((col - g.centroid_um[0] / a.um_per_px()).abs() < 1e-9);
    assert!((row - g.centroid_um[1] / a.um_per_px()).abs() < 1e-9);
    assert_eq!(
        actual[row.floor() as usize * 120 + col.floor() as usize],
        expected[row.floor() as usize * 120 + col.floor() as usize]
    );
    assert_eq!(meta["pixel_to_stage_um"], json!(t));
    let world: Vec<f64> = std::str::from_utf8(&files["Synthetic mount_Synthetic a_mosaic.tfw"])
        .unwrap()
        .lines()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(
        world,
        vec![t[0], t[3], t[1], t[4], t[2] + t[0] / 2.0, t[5] + t[4] / 2.0]
    );
    assert!(job.peak_output_bytes <= 512 * 512 * 3); // one 16-bit tile plus coverage bitmap
    assert_eq!(s.project.as_ref().unwrap().to_json(), p.to_json());
}
#[test]
fn displayed_matches_canvas_lut_and_whole_mount_flips_stage_axes() {
    let (mut s, p) = crate::grain_export::synthetic::mount();
    let mut job = Export::begin(&s, &p.to_json(), &options(Values::Displayed, true)).unwrap();
    assert_eq!(job.plan.images.len(), 1);
    let t = job.plan.images[0].pixel_to_stage_um.unwrap();
    assert!(t[0] > 0.0 && t[4] > 0.0);
    let lut = crate::photometric::tone::lut(&p.areas[0].params.tone);
    let expected: Vec<_> = s.outputs["a"]
        .base
        .iter()
        .map(|v| lut[*v as usize])
        .collect();
    let files = collect(&mut job, &mut s);
    let mut d = Decoder::new(Cursor::new(&files["Synthetic mount_mount_mosaic.tiff"])).unwrap();
    let DecodingResult::U8(actual) = d.read_image().unwrap() else {
        panic!()
    };
    assert_eq!(actual, expected.into_iter().rev().collect::<Vec<_>>());
}
#[test]
fn invalid_mount_falls_back_to_per_area_without_inventing_stage_coordinates() {
    let (mut s, mut p) = crate::grain_export::synthetic::mount();
    p.areas[1].tiles[0].stage_nm = None;
    s.project = Some(p.clone());
    let job = Export::begin(&s, &p.to_json(), &options(Values::Original, true)).unwrap();
    assert_eq!(job.plan.images.len(), 2);
    assert!(job.plan.whole_mount_reason.is_some());
    assert!(job.plan.images[1].pixel_to_stage_um.is_none());
}
#[test]
fn big_threshold_estimate_names_and_validation() {
    assert!(!tiff::Layout::new(100, 100, 16, 1).unwrap().big);
    assert!(tiff::Layout::new(100_000, 100_000, 16, 1).unwrap().big);
    for (w, h, b, c) in [(0, 1, 8, 1), (1, 0, 8, 1), (1, 1, 32, 1), (1, 1, 8, 4)] {
        assert!(tiff::Layout::new(w, h, b, c).is_err());
    }
    assert_eq!(safe_name("../CON:"), "_CON_");
    assert_eq!(safe_name("NUL"), "_NUL");
    assert_eq!(safe_name(" . "), "mosaic");
}
#[test]
fn cancellation_and_write_failure_remove_partial_files() {
    let (mut s, p) = crate::grain_export::synthetic::mount();
    let root = std::env::temp_dir().join(format!("mosaic-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&root).unwrap();
    let cancel = AtomicBool::new(false);
    let progress = |_: &str, _: f32| cancel.store(true, Ordering::SeqCst);
    let ctl = Ctl {
        cancel: &|| cancel.load(Ordering::SeqCst),
        progress: &progress,
    };
    for format in [Format::Tiff, Format::Png, Format::Jpg] {
        cancel.store(false, Ordering::SeqCst);
        let mode = if format == Format::Jpg {
            Values::Displayed
        } else {
            Values::Original
        };
        let mut opts: Options = serde_json::from_str(&options(mode, false)).unwrap();
        opts.format = format;
        let mut job =
            Export::begin(&s, &p.to_json(), &serde_json::to_string(&opts).unwrap()).unwrap();
        assert!(matches!(
            write_directory(&mut job, &mut s, &root, &ctl),
            Err(Error::Cancelled)
        ));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
    // Existing outputs must never be overwritten or removed.
    let existing = root.join("Synthetic mount_Synthetic a_mosaic.tiff");
    std::fs::write(&existing, b"keep").unwrap();
    let mut job = Export::begin(&s, &p.to_json(), &options(Values::Original, false)).unwrap();
    assert!(write_directory(&mut job, &mut s, &root, &Ctl::NONE).is_err());
    assert_eq!(std::fs::read(existing).unwrap(), b"keep");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_stitch_export_matches_blend_for_source_8_and_16_bits() {
    for bits in [8, 16] {
        let mut session = Session::new("Invented tiles");
        for tile in 0..2 {
            let samples: Vec<u16> = (0..64 * 48)
                .map(|i| ((i * 37 + (i / 64) * 11 + tile * 13) % 256) as u16 * 257)
                .collect();
            let bytes = if bits == 16 {
                crate::image::encode_png_gray16(64, 48, &samples)
            } else {
                crate::image::encode_png_gray(
                    64,
                    48,
                    &samples.iter().map(|v| (v / 257) as u8).collect::<Vec<_>>(),
                )
            };
            session.add_file(&format!("tile-{tile}.png"), bytes);
        }
        session.open_mem("2026-01-01T00:00:00Z").unwrap();
        session.open_loose(&json!({"um_per_px":0.5,"grid":{"cols":2,"rows":1,"overlap_pct":25.0,"order":"rows","name":"Invented area"},"files":["tile-0.png","tile-1.png"]}).to_string(),"2026-01-01T00:00:00Z").unwrap();
        let id = session.project.as_ref().unwrap().areas[0].id.clone();
        let mut params = crate::project::AreaParams::default();
        params.photometric.background = false;
        params.blend.levels = 3;
        session
            .stitch(
                &id,
                &serde_json::to_string(&params).unwrap(),
                &Ctl::NONE,
                &|| 0.0,
            )
            .unwrap();
        let project = session.project.as_ref().unwrap().to_json();
        let expected: Vec<u16> = session.outputs[&id]
            .linear
            .px
            .iter()
            .map(|v| {
                (*v as f64 * if bits == 16 { 65535.0 } else { 255.0 })
                    .round()
                    .clamp(0.0, 65535.0) as u16
            })
            .collect();
        let options = json!({"area_ids":[id],"whole_mount":false,"values":"original"}).to_string();
        let mut job = Export::begin(&session, &project, &options).unwrap();
        assert_eq!(job.plan.images[0].bit_depth, bits);
        let files = collect(&mut job, &mut session);
        let (_, bytes) = files
            .iter()
            .find(|(name, _)| name.ends_with(".tiff"))
            .unwrap();
        let mut d = Decoder::new(Cursor::new(bytes)).unwrap();
        let actual: Vec<u16> = match d.read_image().unwrap() {
            DecodingResult::U8(v) => v.into_iter().map(|v| v as u16).collect(),
            DecodingResult::U16(v) => v,
            _ => panic!(),
        };
        assert_eq!(actual.len(), expected.len());
        for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(*a, b, "bits {bits}, pixel {i}");
        }
    }
}

#[test]
#[ignore = "synthetic scale timing; writes about 151 MB into a temporary directory"]
fn large_synthetic_mosaic_timing() {
    let start = std::time::Instant::now();
    let (mut s, p) = crate::grain_export::synthetic::large_mount(42, 100);
    let generated = start.elapsed();
    let resident = s.memory_report().total;
    let mut job = Export::begin(&s, &p.to_json(), &options(Values::Original, false)).unwrap();
    let root = std::env::temp_dir().join(format!("mosaic-timing-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&root).unwrap();
    let start = std::time::Instant::now();
    let result = write_directory(&mut job, &mut s, &root, &Ctl::NONE);
    let elapsed = start.elapsed();
    let bytes = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum::<u64>();
    std::fs::remove_dir_all(&root).unwrap();
    result.unwrap();
    assert!(job.peak_output_bytes <= 512 * 512 * 3);
    assert_eq!(s.memory_report().total, resident);
    eprintln!(
        "synthetic mosaic: generation={:.3}s export={:.3}s bytes={bytes} tiles={} peak_output_bytes={} resident_source_bytes={resident}",
        generated.as_secs_f64(),
        elapsed.as_secs_f64(),
        job.progress().0,
        job.peak_output_bytes
    );
}

#[test]
fn names_collide_portably_and_single_area_mount_uses_stage_orientation() {
    let (mut s, mut p) = crate::grain_export::synthetic::mount();
    p.areas[0].name = "CON".into();
    p.areas[1].name = "con".into();
    let job = Export::begin(&s, &p.to_json(), &options(Values::Original, false)).unwrap();
    assert_eq!(job.plan.images[0].name, "Synthetic mount__CON_mosaic");
    assert_eq!(job.plan.images[1].name, "Synthetic mount__con_mosaic-2");
    assert_eq!(job.metadata(0)["area_name"], "CON");
    p.areas.truncate(1);
    s.project = Some(p.clone());
    let expected: Vec<_> = s.outputs["a"].base.iter().rev().copied().collect();
    let opts=json!({"area_ids":["a"],"whole_mount":true,"values":"displayed","display_tone":{"a":crate::photometric::ToneParams::neutral()}}).to_string();
    let mut job = Export::begin(&s, &p.to_json(), &opts).unwrap();
    let files = collect(&mut job, &mut s);
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| name.ends_with(".tiff"))
        .unwrap();
    let mut d = Decoder::new(Cursor::new(bytes)).unwrap();
    let DecodingResult::U8(actual) = d.read_image().unwrap() else {
        panic!()
    };
    assert_eq!(actual, expected);
}

#[test]
fn automatic_bigtiff_reads_a_tile_beyond_4_gb_without_a_large_allocation() {
    use std::io::{Read, Seek, SeekFrom};
    struct Virtual {
        header: Vec<u8>,
        pos: u64,
        len: u64,
    }
    impl Read for Virtual {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = (self.len - self.pos).min(b.len() as u64) as usize;
            b[..n].fill(0xab);
            if self.pos < self.header.len() as u64 {
                let k = n.min(self.header.len() - self.pos as usize);
                b[..k].copy_from_slice(&self.header[self.pos as usize..self.pos as usize + k]);
            }
            self.pos += n as u64;
            Ok(n)
        }
    }
    impl Seek for Virtual {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            let p = match from {
                SeekFrom::Start(p) => p as i128,
                SeekFrom::Current(d) => self.pos as i128 + d as i128,
                SeekFrom::End(d) => self.len as i128 + d as i128,
            };
            if p < 0 || p > self.len as i128 {
                return Err(std::io::Error::other("outside virtual TIFF"));
            }
            self.pos = p as u64;
            Ok(self.pos)
        }
    }
    let l = tiff::Layout::new(100_000, 100_000, 16, 1).unwrap();
    assert!(l.big);
    let header = l.header([0.5; 2], "{}").unwrap();
    let len = header.len() as u64 + l.pixel_bytes;
    assert!(len > u32::MAX as u64);
    assert!(header.len() < 1_048_576);
    let mut d = Decoder::new(Virtual {
        header,
        pos: 0,
        len,
    })
    .unwrap();
    let DecodingResult::U16(px) = d.read_chunk((l.tiles - 1) as u32).unwrap() else {
        panic!()
    };
    assert!(!px.is_empty());
    assert!(px.iter().all(|v| *v == 0xabab));
}
#[test]
fn png_8_16_and_jpeg_restart_strips_round_trip() {
    for format in [Format::Png, Format::Jpg] {
        for bits in [8, 16] {
            if format == Format::Jpg && bits == 16 {
                continue;
            }
            let (mut s, p) = crate::grain_export::synthetic::mount();
            let mode = if format == Format::Jpg {
                Values::Displayed
            } else {
                Values::Original
            };
            let mut opts: Options = serde_json::from_str(&options(mode, false)).unwrap();
            opts.format = format;
            opts.png_bits = bits;
            let mut job =
                Export::begin(&s, &p.to_json(), &serde_json::to_string(&opts).unwrap()).unwrap();
            let name = format!("{}.{}", job.plan.images[0].name, format.extension());
            let expected: Vec<f64> = if mode == Values::Original {
                s.outputs["a"].linear.px.iter().map(|&v| v as f64).collect()
            } else {
                let lut = crate::photometric::tone::lut(&p.areas[0].params.tone);
                s.outputs["a"]
                    .base
                    .iter()
                    .map(|&v| lut[v as usize] as f64 / 255.)
                    .collect()
            };
            let files = collect(&mut job, &mut s);
            let image = ::image::load_from_memory(&files[&name]).unwrap();
            let max = if bits == 16 { 65535. } else { 255. };
            let actual: Vec<u16> = if bits == 16 {
                image.to_luma16().into_raw()
            } else {
                image
                    .to_luma8()
                    .into_raw()
                    .into_iter()
                    .map(|v| v as u16)
                    .collect()
            };
            assert_eq!(actual.len(), expected.len());
            let mut squared = 0.;
            let mut worst = 0f64;
            for (&v, w) in actual.iter().zip(expected) {
                let delta = (v as f64 - (w * max).round()).abs();
                squared += delta * delta;
                worst = worst.max(delta);
                if format == Format::Png {
                    assert_eq!(delta, 0.);
                }
            }
            if format == Format::Jpg {
                eprintln!(
                    "JPEG quality 90: RMS {:.3}, max {:.0} grey levels",
                    (squared / actual.len() as f64).sqrt(),
                    worst
                );
                assert!((squared / actual.len() as f64).sqrt() < 5., "JPEG RMS");
                assert!(worst < 35., "JPEG max {worst}");
            }
            assert!(files.contains_key(&format!("{}.{}", job.plan.images[0].name, format.world())));
        }
    }
}
#[test]
fn format_preflight_downsampling_and_split_affines() {
    assert_eq!(fit_factor(65535, 65535), 1);
    assert_eq!(fit_factor(65536, 10), 2);
    assert_eq!(fit_factor(262141, 5), 8);
    let (mut s, mut p) = crate::grain_export::synthetic::mount();
    s.outputs.get_mut("a").unwrap().linear.w = 131071;
    p.areas[0].placement.as_mut().unwrap().width = 131071;
    s.project = Some(p.clone());
    let mut opts: Options = serde_json::from_str(&options(Values::Original, false)).unwrap();
    opts.area_ids = vec!["a".into()];
    opts.format = Format::Png;
    let plan = Export::begin(&s, &p.to_json(), &serde_json::to_string(&opts).unwrap()).unwrap();
    assert!(plan.plan.images[0].limit_reason.is_some());
    opts.size_choice = SizeChoice::Downsample;
    let plan = Export::begin(&s, &p.to_json(), &serde_json::to_string(&opts).unwrap()).unwrap();
    let a = &plan.plan.images[0];
    assert_eq!(a.downsample, 4);
    assert_eq!(a.width, 32768);
    assert_eq!(a.pixel_size_um, [p.areas[0].um_per_px() * 4.; 2]);
    assert_eq!(plan.metadata(0)["pixel_size_um"][0], a.pixel_size_um[0]);
    let original = affine(&p.areas[0]).unwrap();
    let scaled = a.pixel_to_stage_um.unwrap();
    assert_eq!(scaled[0], original[0] * 4.);
    assert_eq!(scaled[2], original[2]);
    opts.size_choice = SizeChoice::Split;
    let plan = Export::begin(&s, &p.to_json(), &serde_json::to_string(&opts).unwrap()).unwrap();
    assert_eq!(plan.plan.images.len(), 3);
    assert!(
        plan.plan
            .images
            .iter()
            .all(|a| a.width <= 65535 && a.height <= 65535 && a.limit_reason.is_none())
    );
    let second = &plan.plan.images[1];
    assert!(second.name.ends_with("_r1_c2"));
    assert_eq!(
        second.pixel_to_stage_um.unwrap()[2],
        original[2] + original[0] * 65535.
    );
}

#[test]
fn reduced_png_samples_and_world_file_share_the_same_grid() {
    let (mut s, p) = crate::grain_export::synthetic::mount();
    let opts=json!({"area_ids":["a"],"whole_mount":false,"values":"original","format":"png","png_bits":16,"size_choice":"downsample","downsample":16}).to_string();
    let mut job = Export::begin(&s, &p.to_json(), &opts).unwrap();
    let a = job.plan.images[0].clone();
    let (w, h) = (s.outputs["a"].linear.w, s.outputs["a"].linear.h);
    let expected: Vec<_> = (0..a.height as usize)
        .flat_map(|y| {
            (0..a.width as usize)
                .map(move |x| (y * 16 + 8).min(h - 1) * w + (x * 16 + 8).min(w - 1))
        })
        .map(|i| (s.outputs["a"].linear.px[i] as f64 * 65535.).round() as u16)
        .collect();
    let files = collect(&mut job, &mut s);
    let actual = ::image::load_from_memory(&files[&format!("{}.png", a.name)])
        .unwrap()
        .to_luma16();
    assert_eq!(actual.into_raw(), expected);
    let t = a.pixel_to_stage_um.unwrap();
    let world: Vec<f64> = std::str::from_utf8(&files[&format!("{}.pgw", a.name)])
        .unwrap()
        .lines()
        .map(|x| x.parse().unwrap())
        .collect();
    assert_eq!(world[0], t[0]);
    assert_eq!(world[4], t[2] + t[0] / 2.);
    assert_eq!(world[5], t[5] + t[4] / 2.);
}

#[test]
fn image_recipe_changes_only_display_levels_in_mosaic_exports() {
    let (mut s,mut p)=crate::grain_export::synthetic::mount();
    let mut baseline=Export::begin(&s,&p.to_json(),&options(Values::Original,false)).unwrap();
    let original=collect(&mut baseline,&mut s);
    let mut baseline=Export::begin(&s,&p.to_json(),&options(Values::Displayed,false)).unwrap();
    let display=collect(&mut baseline,&mut s);
    p.image_edits=Some(serde_json::from_value(serde_json::json!({"version":1,"areas":{"a":{"rotate_deg":90,"flip_h":true,"flip_v":true,"crop":{"kind":"rect","points_um":[[0,0],[1,1]]},"levels":{"black":30,"white":210},"masks":[{"points_um":[[0,0],[500,500]],"radius_um":500}]}}})).unwrap());
    let mut job=Export::begin(&s,&p.to_json(),&options(Values::Original,false)).unwrap();let after=collect(&mut job,&mut s);
    let name="Synthetic mount_Synthetic a_mosaic.tiff";
    assert_eq!(after[name],original[name],"original pixels, dimensions and stage alignment stay identical");
    let mut job=Export::begin(&s,&p.to_json(),&options(Values::Displayed,false)).unwrap();let after=collect(&mut job,&mut s);
    let mut d=Decoder::new(Cursor::new(&display[name])).unwrap();let dimensions=d.dimensions().unwrap();
    let DecodingResult::U8(before)=d.read_image().unwrap() else {panic!("8-bit displayed")};
    let mut d=Decoder::new(Cursor::new(&after[name])).unwrap();assert_eq!(d.dimensions().unwrap(),dimensions);
    let DecodingResult::U8(after)=d.read_image().unwrap() else {panic!("8-bit displayed")};
    for (a,b) in before.into_iter().zip(after) { let expected=((a as f64-30.0)*255.0/180.0).round().clamp(0.0,255.0) as u8; assert!(b.abs_diff(expected)<=1,"levels follow the display LUT without spatial edits"); }
}

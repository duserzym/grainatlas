use super::*;
use crate::mosaic_export::Values;
use crate::{image::Gray, project::AreaRecord};
impl Session {
    pub(crate) fn mosaic_export_provenance(&self, id: &str) -> serde_json::Value {
        serde_json::json!({"kind":if self.bundle_sources.contains_key(id) {"published-8bit"} else {"raw-stitched"},"published_package":self.bundle_digest,"display_mosaic_sha256":self.outputs.get(id).map(|o| &o.mosaic_json.sha256)})
    }
    /// Release export-only heavy buffers according to the session's own budget.
    pub fn finish_mosaic_export(&mut self) {
        self.trim(None);
    }

    pub(crate) fn mosaic_source_bits(&self, id: &str) -> Result<u16> {
        if self.bundle_sources.contains_key(id) {
            return Ok(8);
        }
        if let Some(bits) = self.mosaic_depths.get(id) {
            return Ok(*bits);
        }
        let Some(pkg) = self.pkg.as_ref() else {
            return Ok(16);
        };
        let area = pkg
            .info()
            .areas
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| invalid("Unknown source area"))?;
        // Decode at most one source field at a time. Mixed-depth areas retain 16 bits.
        for i in 0..pkg.info().areas[area].tiles.len() {
            let file = &pkg.info().areas[area].tiles[i].file;
            let bytes = pkg.source().read(file)?;
            if bytes.starts_with(&[0xff, 0xd8]) {
                continue;
            } // JPEG inputs are 8-bit.
            use ::image::ImageDecoder;
            let reader = ::image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(invalid)?;
            let decoder = reader.into_decoder().map_err(invalid)?;
            if matches!(
                decoder.color_type(),
                ::image::ColorType::L16
                    | ::image::ColorType::La16
                    | ::image::ColorType::Rgb16
                    | ::image::ColorType::Rgba16
            ) {
                return Ok(16);
            }
        }
        Ok(8)
    }

    pub(crate) fn mosaic_export_needs_source(&self, id: &str, values: Values) -> bool {
        values == Values::Original
            && !self.bundle_sources.contains_key(id)
            && self.outputs.get(id).is_none_or(|o| o.linear.px.is_empty())
    }
    pub(crate) fn mosaic_export_placement(&self, rec: &mut AreaRecord, ctl: &Ctl) -> Result<()> {
        let pkg = self
            .pkg
            .as_ref()
            .ok_or_else(|| invalid("Source tiles are unavailable"))?;
        let ai = pkg
            .info()
            .areas
            .iter()
            .position(|a| a.id == rec.id)
            .ok_or_else(|| invalid("Unknown source area"))?;
        let mut meta = pkg.info().areas[ai].clone();
        // Headers can lack dimensions. Decode one capture at a time, never the area.
        let mut failed = Vec::new();
        for (i, t) in meta.tiles.iter_mut().enumerate() {
            ctl.check()?;
            let decoded = pkg.tile(ai, i)?;
            t.w = decoded.image.w;
            t.h = decoded.image.h;
            if let Some(r) = rec.tiles.get_mut(i) {
                r.w = t.w;
                r.h = t.h;
                r.sha256 = decoded.sha256;
            }
            failed.push(decoded.failed);
        }
        let groups = pipeline::hdr_groups(&meta.tiles);
        meta.tiles = groups
            .iter()
            .enumerate()
            .map(|(i, g)| crate::readers::TileMeta {
                index: i,
                ..meta.tiles[g[0]].clone()
            })
            .collect();
        let failed: Vec<_> = groups
            .iter()
            .map(|g| g.iter().all(|&i| failed[i]))
            .collect();
        let sizes: Vec<_> = meta.tiles.iter().map(|t| (t.w, t.h)).collect();
        rec.placement = Some(crate::mosaic::stitch_loaded(
            &meta,
            &sizes,
            &failed,
            &rec.params.stitch,
            &|i| self.mosaic_export_tile(ai, &groups[i], ctl),
            ctl,
        )?);
        Ok(())
    }
    fn mosaic_export_tile(&self, area: usize, group: &[usize], ctl: &Ctl) -> Result<Gray> {
        let pkg = self
            .pkg
            .as_ref()
            .ok_or_else(|| invalid("Source tiles are unavailable"))?;
        let mut tiles = Vec::new();
        let mut fallback = None;
        for &i in group {
            ctl.check()?;
            let t = pkg.tile(area, i)?;
            if !t.failed {
                tiles.push(t.image);
            } else if fallback.is_none() {
                fallback = Some(t.image);
            }
        }
        if tiles.is_empty() {
            return fallback.ok_or_else(|| invalid("Empty source field"));
        }
        if tiles.len() == 1 {
            return Ok(tiles.pop().unwrap());
        }
        Ok(crate::photometric::hdr::merge_exposures(&tiles)
            .map(|m| m.image)
            .unwrap_or_else(|| tiles.remove(0)))
    }
    pub(crate) fn mosaic_export_source(
        &self,
        rec: &AreaRecord,
        ctl: &Ctl,
    ) -> Result<crate::mosaic_export::Source> {
        let pkg = self
            .pkg
            .as_ref()
            .ok_or_else(|| invalid("Source tiles are unavailable"))?;
        let area = pkg
            .info()
            .areas
            .iter()
            .position(|a| a.id == rec.id)
            .ok_or_else(|| invalid("Unknown source area"))?;
        let mut tiles = pkg.info().areas[area].tiles.clone();
        for (t, r) in tiles.iter_mut().zip(&rec.tiles) {
            t.w = r.w;
            t.h = r.h;
        }
        let groups = pipeline::hdr_groups(&tiles);
        let full = rec
            .placement
            .as_ref()
            .ok_or_else(|| invalid("Missing source placement"))?;
        let (placement, ids) = full.active();
        let load = |i: usize| self.mosaic_export_tile(area, &groups[ids[i]], ctl);
        let mut params = rec.params.photometric.clone();
        params.overrides = params
            .overrides
            .into_iter()
            .filter_map(|mut ov| {
                ov.index = ids.iter().position(|&i| i == ov.index)?;
                Some(ov)
            })
            .collect();
        let mut saved = rec.photometric.clone();
        if let Some(p) = &mut saved
            && p.auto_gain.len() == full.tiles.len()
        {
            p.auto_gain = ids.iter().map(|&i| p.auto_gain[i]).collect();
        }
        let photo = crate::photometric::streaming::prepare(
            &placement,
            &load,
            &params,
            saved.as_ref(),
            ctl,
        )?;
        let corrected = |i| Ok(photo.apply(i, &load(i)?));
        let blend =
            crate::blend::streaming::Prepared::new(&placement, &corrected, &rec.params.blend, ctl)?;
        Ok(crate::mosaic_export::Source {
            placement,
            ids,
            groups,
            area,
            photo,
            blend,
            strip: None,
            peak: 0,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_mosaic_export_pixels<T>(
        &self,
        id: &str,
        values: Values,
        tone: &ToneParams,
        ctl: &Ctl,
        source: Option<&mut crate::mosaic_export::Source>,
        y: usize,
        end: usize,
        f: impl FnOnce(usize, usize, usize, crate::grain_export::pixels::Pixels<'_>) -> Result<T>,
    ) -> Result<T> {
        use crate::grain_export::pixels::Pixels;
        if let Some(s) = source {
            if s.strip
                .as_ref()
                .is_none_or(|(old, g)| y < *old || end > *old + g.h)
            {
                s.strip = None;
                let load = |i: usize| {
                    let raw = self.mosaic_export_tile(s.area, &s.groups[s.ids[i]], ctl)?;
                    Ok(s.photo.apply(i, &raw))
                };
                let (g, _, peak) = s
                    .blend
                    .strip(&s.placement, &load, y, 512, s.photo.fill, ctl)?;
                s.peak = s.peak.max(peak);
                s.strip = Some((y, g));
            }
            let (origin, g) = s.strip.as_ref().unwrap();
            return f(g.w, g.h, *origin, Pixels::Float(&g.px));
        }
        let o = self.out(id)?;
        let w = o.linear.w;
        let rows = 512.min(o.linear.h - y);
        let range = y * w..(y + rows) * w;
        let px = if values == Values::Displayed {
            Pixels::Byte(
                &o.base[range],
                Box::new(crate::photometric::tone::lut(tone)),
            )
        } else if let Some(px) = self.bundle_sources.get(id) {
            Pixels::Byte(&px[range], Box::new(std::array::from_fn(|i| i as u8)))
        } else {
            Pixels::Float(&o.linear.px[range])
        };
        f(w, rows, y, px)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        readers::{AreaMeta, Package, PackageInfo, Tile, TileMeta},
        source::{MemSource, Source},
    };
    struct Synthetic {
        info: PackageInfo,
        source: MemSource,
    }
    impl Package for Synthetic {
        fn info(&self) -> &PackageInfo {
            &self.info
        }
        fn source(&self) -> &dyn Source {
            &self.source
        }
        fn tile(&self, area: usize, index: usize) -> Result<Tile> {
            let meta = self.info.areas[area].tiles[index].clone();
            let (w, h) = (meta.w, meta.h);
            let image = Gray {
                w,
                h,
                px: (0..w * h)
                    .map(|j| {
                        let x = j % w + meta.pos_um[0] as usize;
                        let y = j / w + meta.pos_um[1] as usize;
                        0.2 + ((x * 13 + y * 31) % 71) as f32 / 200.
                    })
                    .collect(),
            };
            Ok(Tile {
                meta,
                image,
                depth: crate::image::Depth::U16,
                sha256: "synthetic".into(),
                failed: false,
            })
        }
    }
    fn fixture() -> Session {
        let tiles = (0..4)
            .map(|i| {
                let x = (i % 2) * 72;
                let y = (i / 2) * 72;
                TileMeta {
                    index: i,
                    file: format!("synthetic-{i}"),
                    w: 96,
                    h: 96,
                    pos_um: [x as f64, y as f64],
                    stage_nm: Some([(x + 48) as f64 * 1000., (y + 48) as f64 * 1000.]),
                    pixel_size_nm: 1000.,
                    grid: Some([i % 2, i / 2]),
                    meta: Default::default(),
                }
            })
            .collect();
        let area = AreaMeta {
            id: "a".into(),
            name: "Area".into(),
            grid: Some([2, 2]),
            pixel_size_nm: 1000.,
            field_um: None,
            pitch_um: None,
            stage_sign: [1.; 2],
            tiles,
            meta: Default::default(),
        };
        let info = PackageInfo {
            reader: "synthetic".into(),
            kind: "package".into(),
            name: "Mount".into(),
            areas: vec![area],
            meta: Default::default(),
            navigation: None,
        };
        let mut s = Session::new("Synthetic");
        s.project = Some(Project::new(&info, "", "2026-01-01T00:00:00Z"));
        s.pkg = Some(Box::new(Synthetic {
            info,
            source: MemSource::default(),
        }));
        s.mosaic_depths.insert("a".into(), 16);
        s
    }
    #[test]
    fn saved_placement_reopens_without_registration() {
        let mut original = fixture();
        original.stitch("a", "{}", &Ctl::NONE, &|| 0.).unwrap();
        let mut project = original.project.clone().unwrap();
        // A distinctive saved solution: registration would replace this with the stage prior.
        let area = &mut project.areas[0];
        let placement = area.placement.as_mut().unwrap();
        placement.tiles[1].x += 3;
        placement.width += 3;
        let placement = placement.clone();
        let params = serde_json::to_string(&area.params).unwrap();
        project.extra.insert("grainatlas_ui".into(), serde_json::json!({
            "spots": [{"id":"synthetic-registered", "x_um":12.125, "y_um":24.25, "acquisition_ids":["synthetic-link"]}]
        }));
        let mut reopened = fixture();
        reopened.load_project(&project.to_json()).unwrap();
        let result: serde_json::Value = serde_json::from_str(
            &reopened.recompute("a", &params, &Ctl::NONE, &|| 0.).unwrap(),
        ).unwrap();
        assert_eq!(reopened.outputs["a"].placement, placement);
        assert!(!result["timings"].as_array().unwrap().iter().any(|t| t[0] == "stitch"));
        assert_eq!(reopened.project.as_ref().unwrap().extra["grainatlas_ui"], project.extra["grainatlas_ui"]);
    }

    #[test]
    fn saved_placement_replaces_an_older_resident_output() {
        let mut session = fixture();
        session.stitch("a", "{}", &Ctl::NONE, &|| 0.).unwrap();
        let mut project = session.project.clone().unwrap();
        let resident = session.outputs["a"].base.clone();
        project.extra.insert("grainatlas_ui".into(), serde_json::json!({"labels":[]}));
        session.load_project(&project.to_json()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&resident, &session.outputs["a"].base));
        let area = &mut project.areas[0];
        area.placement.as_mut().unwrap().tiles[1].x += 3;
        area.placement.as_mut().unwrap().width += 3;
        let placement = area.placement.clone().unwrap();
        let params = serde_json::to_string(&area.params).unwrap();
        session.load_project(&project.to_json()).unwrap();
        session.recompute("a", &params, &Ctl::NONE, &|| 0.).unwrap();
        assert_eq!(session.outputs["a"].placement, placement);
    }

    #[test]
    fn saved_metadata_retains_published_pixel_source() {
        let mut session = fixture();
        session.load_bundle_area("a", 8, 8, (0..64).collect()).unwrap();
        let mut project = session.project.clone().unwrap();
        project.areas[0].params.tone.gamma = 1.5;
        session.load_project(&project.to_json()).unwrap();
        assert!(session.outputs.contains_key("a"));
        assert!(session.bundle_sources.contains_key("a"));
    }

    #[test]
    fn evicted_and_never_built_exports_do_not_rehydrate_mosaics() {
        let mut s = fixture();
        s.stitch("a", "{}", &Ctl::NONE, &|| 0.).unwrap();
        let expected: Vec<_> = s.outputs["a"]
            .linear
            .px
            .iter()
            .map(|v| (*v as f64 * 65535.).round() as u16)
            .collect();
        s.outputs.get_mut("a").unwrap().linear.px.clear();
        s.outputs.get_mut("a").unwrap().coverage.clear();
        for mut session in [s, fixture()] {
            let p = session.project.as_ref().unwrap().to_json();
            let mut job = crate::mosaic_export::Export::begin(
                &session,
                &p,
                r#"{"area_ids":["a"],"whole_mount":false,"values":"original"}"#,
            )
            .unwrap();
            let mut bytes = Vec::new();
            while let Some(chunk) = job.next(&mut session, &Ctl::NONE).unwrap() {
                if chunk.name.ends_with(".tiff") {
                    bytes.extend(chunk.data);
                }
            }
            let mut decoder = ::tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).unwrap();
            let ::tiff::decoder::DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
                panic!()
            };
            assert_eq!(pixels, expected);
            assert!(session.outputs.values().all(|o| o.linear.px.is_empty()));
            assert!(job.peak_source_working_bytes < 16_000_000);
        }
    }
    #[test]
    fn virtual_large_source_counts_only_strip_sized_pixel_buffers() {
        let mut s = fixture();
        let mut rec = s.project.as_ref().unwrap().areas[0].clone();
        s.mosaic_export_placement(&mut rec, &Ctl::NONE).unwrap();
        let mut peaks = Vec::new();
        for height in [10_000, 100_000] {
            rec.placement.as_mut().unwrap().height = height;
            s.project.as_mut().unwrap().areas[0] = rec.clone();
            let mut job = crate::mosaic_export::Export::begin(
                &s,
                &s.project.as_ref().unwrap().to_json(),
                r#"{"area_ids":["a"],"whole_mount":false,"values":"original"}"#,
            )
            .unwrap();
            while job.next(&mut s, &Ctl::NONE).unwrap().is_some() {}
            assert!(s.outputs.is_empty());
            assert!(job.peak_source_working_bytes < 16_000_000);
            peaks.push(job.peak_source_working_bytes);
        }
        assert_eq!(peaks[0], peaks[1]);
        eprintln!(
            "virtual source 168 x 100000: peak pixel working bound {} bytes",
            peaks[1]
        );
    }
    #[test]
    fn scanline_chunks_reuse_the_existing_source_row() {
        let s = fixture();
        let mut rec = s.project.as_ref().unwrap().areas[0].clone();
        s.mosaic_export_placement(&mut rec, &Ctl::NONE).unwrap();
        rec.placement.as_mut().unwrap().height = 1000;
        let mut source = s.mosaic_export_source(&rec, &Ctl::NONE).unwrap();
        let tone = ToneParams::default();
        s.with_mosaic_export_pixels(
            "a",
            Values::Original,
            &tone,
            &Ctl::NONE,
            Some(&mut source),
            0,
            64,
            |_, _, _, _| Ok(()),
        )
        .unwrap();
        let ptr = source.strip.as_ref().unwrap().1.px.as_ptr();
        s.with_mosaic_export_pixels(
            "a",
            Values::Original,
            &tone,
            &Ctl::NONE,
            Some(&mut source),
            64,
            128,
            |_, _, origin, _| {
                assert_eq!(origin, 0);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(source.strip.as_ref().unwrap().1.px.as_ptr(), ptr);
    }
}

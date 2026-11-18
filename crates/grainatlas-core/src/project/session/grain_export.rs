use super::*;
impl Session {
    pub(crate) fn with_grain_export_area<T>(
        &mut self,
        id: &str,
        original: bool,
        ctl: &Ctl,
        f: impl FnOnce(&AreaOutput, Option<&[u8]>) -> Result<T>,
    ) -> Result<T> {
        // Coverage is needed to validate cross-area layers, even with displayed pixels.
        if original || self.out(id)?.coverage.is_empty() {
            self.ensure_heavy(id, ctl)?;
        }
        f(
            self.out(id)?,
            self.bundle_sources.get(id).map(|v| v.as_slice()),
        )
    }
    pub(crate) fn trim_grain_export(&mut self) {
        self.trim(None);
    }
}

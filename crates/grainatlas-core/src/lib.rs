//! grainatlas-core: every pixel operation of grainatlas (ADR-0001).
//!
//! The crate has no file-system access of its own: packages are read through
//! [`source::Source`] (an in-memory map in the browser, a directory on native
//! targets) and results are returned as byte buffers.
//!
//! Module map follows `ARCHITECTURE_PLAN.md` section 5.2.

pub mod atlas;
pub mod blend;
pub mod error;
pub mod image;
pub mod laser;
pub mod layout;
pub mod mosaic;
pub mod photometric;
pub mod project;
pub mod readers;
pub mod scale;
pub mod segment;
pub mod source;
pub mod spots;
pub mod util;

pub use error::{Error, Result};

/// Progress and cancellation hooks shared by long-running steps.
pub struct Ctl<'a> {
    pub cancel: &'a (dyn Fn() -> bool + Sync),
    pub progress: &'a (dyn Fn(&str, f32) + Sync),
}

impl Ctl<'_> {
    pub const NONE: Ctl<'static> = Ctl { cancel: &|| false, progress: &|_, _| {} };
    pub fn check(&self) -> Result<()> {
        if (self.cancel)() { Err(Error::Cancelled) } else { Ok(()) }
    }
    pub fn report(&self, stage: &str, f: f32) {
        (self.progress)(stage, f.clamp(0.0, 1.0));
    }
}

//! Intern objects produced by pdf-writer/svg2pdf, including font dependency trees.
//! Use pdf-writer's own xref offsets and reference rewriter; binary streams and
//! literal strings must never be scanned as indirect references.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{Error, Result};
use pdf_writer::{Chunk, Pdf, Primitive, Ref};

struct Raw<'a>(&'a [u8]);
impl Primitive for Raw<'_> {
    fn write(self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.0);
    }
}
#[derive(Default)]
pub(super) struct Objects(HashMap<Vec<u8>, Ref>);
impl Objects {
    pub fn import(
        &mut self,
        pdf: &mut Pdf,
        alloc: &mut Ref,
        chunk: &Chunk,
        root: Ref,
    ) -> Result<Ref> {
        let source = split(chunk)?;
        self.intern(
            pdf,
            alloc,
            root,
            &source,
            &mut HashMap::new(),
            &mut BTreeSet::new(),
        )
    }
    fn intern(
        &mut self,
        pdf: &mut Pdf,
        alloc: &mut Ref,
        id: Ref,
        source: &BTreeMap<Ref, Chunk>,
        mapped: &mut HashMap<Ref, Ref>,
        visiting: &mut BTreeSet<Ref>,
    ) -> Result<Ref> {
        if let Some(&id) = mapped.get(&id) {
            return Ok(id);
        }
        if !visiting.insert(id) {
            return Err(invalid());
        }
        let object = source.get(&id).ok_or_else(invalid)?;
        let mut deps = BTreeSet::new();
        object.renumber(|r| {
            if r != id {
                deps.insert(r);
            }
            r
        });
        for dep in deps {
            self.intern(pdf, alloc, dep, source, mapped, visiting)?;
        }
        // The outer object's number is not part of its content identity.
        let normalized = object.renumber(|r| if r == id { Ref::new(1) } else { mapped[&r] });
        let body = body(normalized.as_bytes(), Ref::new(1))?;
        let shared = if let Some(&shared) = self.0.get(body) {
            shared
        } else {
            let shared = alloc.bump();
            pdf.indirect(shared).primitive(Raw(body));
            self.0.insert(body.to_vec(), shared);
            shared
        };
        visiting.remove(&id);
        mapped.insert(id, shared);
        Ok(shared)
    }
}
fn invalid() -> Error {
    Error::Invalid("invalid generated PDF resource graph".into())
}
fn body(bytes: &[u8], id: Ref) -> Result<&[u8]> {
    let opening = format!("{} 0 obj\n", id.get());
    bytes
        .strip_prefix(opening.as_bytes())
        .and_then(|b| b.strip_suffix(b"\nendobj\n\n"))
        .ok_or_else(invalid)
}
fn split(chunk: &Chunk) -> Result<BTreeMap<Ref, Chunk>> {
    let mut wrapper = Pdf::new();
    wrapper.extend(chunk);
    let bytes = wrapper.finish();
    let marker = b"\nstartxref\n";
    let at = bytes
        .windows(marker.len())
        .rposition(|s| s == marker)
        .ok_or_else(invalid)?
        + marker.len();
    let end = bytes[at..]
        .iter()
        .position(|&c| c == b'\n')
        .ok_or_else(invalid)?
        + at;
    let offset = std::str::from_utf8(&bytes[at..end])
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(invalid)?;
    let mut lines = bytes
        .get(offset..)
        .ok_or_else(invalid)?
        .split(|&c| c == b'\n');
    if lines.next() != Some(b"xref".as_slice()) {
        return Err(invalid());
    }
    let header = std::str::from_utf8(lines.next().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let count: usize = header
        .strip_prefix("0 ")
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)?;
    let mut offsets = Vec::new();
    for id in 0..count {
        let line = std::str::from_utf8(lines.next().ok_or_else(invalid)?).map_err(|_| invalid())?;
        if line.split_whitespace().nth(2) == Some("n") {
            let at: usize = line
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(invalid)?;
            offsets.push((at, Ref::new(id as i32)));
        }
    }
    offsets.sort_unstable();
    let mut out = BTreeMap::new();
    for (i, &(start, id)) in offsets.iter().enumerate() {
        let end = offsets.get(i + 1).map_or(offset, |v| v.0);
        let mut object = Chunk::new();
        object
            .indirect(id)
            .primitive(Raw(body(bytes.get(start..end).ok_or_else(invalid)?, id)?));
        out.insert(id, object);
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shares_dependencies_and_preserves_binary_streams_and_strings() {
        let mut objects = Objects::default();
        let mut pdf = Pdf::new();
        let mut alloc = Ref::new(1);
        let make = |root: i32, leaf: i32| {
            let mut c = Chunk::new();
            c.indirect(Ref::new(root))
                .dict()
                .pair(pdf_writer::Name(b"Child"), Ref::new(leaf));
            c.stream(Ref::new(leaf), b"\nendobj\n\n99 0 obj\n 1 0 R")
                .pair(pdf_writer::Name(b"Text"), pdf_writer::Str(b"12 0 R"));
            c
        };
        let a = objects
            .import(&mut pdf, &mut alloc, &make(1, 2), Ref::new(1))
            .unwrap();
        let b = objects
            .import(&mut pdf, &mut alloc, &make(99, 120), Ref::new(99))
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(pdf.refs().len(), 2);
    }
}

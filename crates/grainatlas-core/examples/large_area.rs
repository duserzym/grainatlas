//! Generate the full-size synthetic benchmark outside the repository.
#[allow(dead_code)]
#[path = "../tests/support/large_area.rs"]
mod support;
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: large_area /tmp/grainatlas-large");
    support::generate(std::path::Path::new(&path), 1280, 960);
}

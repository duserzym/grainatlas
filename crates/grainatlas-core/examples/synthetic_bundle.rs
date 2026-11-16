//! Generate invented e2e inputs only under the ignored repository target directory.
mod support;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (session, project, _) = support::synthetic();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/remote-edit-bundle");
    for (path, bytes) in session.atlas_files(&project.to_json(), "")? {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    println!("{}", root.display());
    Ok(())
}

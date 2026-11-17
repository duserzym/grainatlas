//! Generate the invented print editor project under the ignored target directory.
#[allow(dead_code)]
mod support;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (session, project) = support::print_edit();
    let ui = &project.extra["grainatlas_ui"];
    for packing in ["lines", "packed"] {
        let params = serde_json::json!({ "paper": "tabloid", "landscape": true, "um_per_mm": 4.0, "packing": packing,
            "enhance": { "denoise": false, "sharpen": false } });
        let result: serde_json::Value = serde_json::from_str(&session.print_sheets(&params.to_string(), &ui["grains"].to_string(), &ui["tone"].to_string())?)?;
        let strips = result["plan"]["strips"].as_array().unwrap().len();
        let sheets = result["pages"].as_array().unwrap().len();
        assert!(strips >= 3, "editor project needs at least three strips");
        if packing == "lines" { assert!(sheets >= 2, "editor project needs at least two sheets in lines"); }
        assert!(result["problems"].as_array().is_none_or(Vec::is_empty), "automatic editor project must be valid");
        println!("{packing}: {strips} strips, {sheets} sheets");
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/print-edit-bundle");
    for (path, bytes) in session.atlas_files(&project.to_json(), "")? {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    println!("{}", root.display());
    Ok(())
}

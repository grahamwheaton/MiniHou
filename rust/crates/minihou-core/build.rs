use std::{collections::BTreeSet, env, fs, path::Path};

#[derive(Debug, PartialEq, Eq)]
struct Package { folder: String, id: String, name: String, category: String, description: String, keywords: String }

fn field<'a>(text: &'a str, key: &str) -> Result<&'a str, String> {
    let mut found = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (k, v) = line.split_once('=').ok_or_else(|| format!("invalid metadata line: {line}"))?;
        let k = k.trim();
        if !["id", "name", "category", "description", "keywords"].contains(&k) { return Err(format!("unknown metadata field {k}")); }
        if k == key {
            if found.is_some() { return Err(format!("duplicate metadata field {key}")); }
            found = Some(v.trim());
        }
    }
    let value = found.ok_or_else(|| format!("missing metadata field {key}"))?;
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("invalid metadata field {key}: expected nonempty quoted string"))
}

fn discover(root: &Path) -> Result<Vec<Package>, String> {
    let mut packages = Vec::new();
    if !root.exists() { return Ok(packages); }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() { continue; }
        let path = entry.path();
        let folder = entry.file_name().to_string_lossy().into_owned();
        if !folder.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || folder.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(format!("{folder}: folder must be a Rust snake_case identifier"));
        }
        if !path.join("node.rs").exists() { return Err(format!("{folder}: missing node.rs")); }
        let metadata = fs::read_to_string(path.join("node.toml")).map_err(|e| format!("{folder}: missing or unreadable node.toml: {e}"))?;
        let read = |key| field(&metadata, key).map(str::to_owned).map_err(|e| format!("{folder}: {e}"));
        let package = Package { folder: folder.clone(), id: read("id")?, name: read("name")?, category: read("category")?, description: read("description")?, keywords: read("keywords")? };
        const CATEGORIES: &[&str] = &["Create", "Modify", "Model", "Polygon", "Edges", "Deform", "Texture", "Groups", "Attributes", "Curves", "Points", "Rigid Bodies", "Constraints", "Terrain FX", "Volume", "PCG", "Procedural", "Flow", "Measure", "Library"];
        if !CATEGORIES.contains(&package.category.as_str()) { return Err(format!("{}: unknown category {:?}", package.folder, package.category)); }
        if !package.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || package.id.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(format!("{}: id must be snake_case", package.folder));
        }
        packages.push(package);
    }
    packages.sort_by(|a, b| a.id.cmp(&b.id).then(a.folder.cmp(&b.folder)));
    let mut ids = BTreeSet::new();
    for p in &packages { if !ids.insert(&p.id) { return Err(format!("duplicate node id: {}", p.id)); } }
    Ok(packages)
}

fn generate(packages: &[Package], root: &Path) -> String {
    let mut out = String::new();
    for p in packages {
        let path = root.join(&p.folder).join("node.rs").canonicalize().unwrap();
        out.push_str(&format!("#[path = {:?}] mod {};\n", path.to_string_lossy(), p.folder));
    }
    out.push_str("pub(super) const SPECS: &[super::NodeSpec] = &[\n");
    for p in packages { out.push_str(&format!("    {}::NODE,\n", p.folder)); }
    out.push_str("];\npub(super) const CATALOGUE: &[super::NodeCatalogueEntry] = &[\n");
    for p in packages {
        out.push_str(&format!("    super::NodeCatalogueEntry {{ id: {:?}, name: {:?}, category: {:?}, description: {:?}, keywords: {:?} }},\n", p.id, p.name, p.category, p.description, p.keywords));
    }
    out.push_str("];\n");
    out
}

fn main() {
    let root = Path::new("src/nodes/packages");
    println!("cargo:rerun-if-changed={}", root.display());
    let packages = discover(root).unwrap_or_else(|e| panic!("node package metadata: {}", e));
    for p in &packages { println!("cargo:rerun-if-changed={}/{}", root.display(), p.folder); }
    let output = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("node_packages.rs");
    fs::write(output, generate(&packages, root)).expect("write node package registry");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn metadata_errors_and_order() {
        let root = env::temp_dir().join(format!("minihou_node_build_{}", std::process::id()));
        fs::create_dir_all(root.join("zeta")).unwrap();
        fs::write(root.join("zeta/node.rs"), "").unwrap();
        assert!(discover(&root).unwrap_err().contains("node.toml"));
        let meta = |id: &str| format!("id = \"{id}\"\nname = \"Name\"\ncategory = \"Modify\"\ndescription = \"Description\"\nkeywords = \"offset\"\n");
        fs::write(root.join("zeta/node.toml"), "id = \"zeta\"\n").unwrap();
        assert!(discover(&root).unwrap_err().contains("missing metadata field"));
        fs::write(root.join("zeta/node.toml"), meta("zeta").replace("Modify", "Unknown")).unwrap();
        assert!(discover(&root).unwrap_err().contains("unknown category"));
        fs::write(root.join("zeta/node.toml"), meta("zeta")).unwrap();
        fs::create_dir_all(root.join("alpha")).unwrap();
        fs::write(root.join("alpha/node.rs"), "").unwrap();
        fs::write(root.join("alpha/node.toml"), meta("alpha")).unwrap();
        let list = discover(&root).unwrap();
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["alpha", "zeta"]);
        assert_eq!(generate(&list, &root), generate(&discover(&root).unwrap(), &root));
        fs::write(root.join("alpha/node.toml"), meta("zeta")).unwrap();
        assert!(discover(&root).unwrap_err().contains("duplicate node id"));
        fs::remove_dir_all(root).unwrap();
    }
}

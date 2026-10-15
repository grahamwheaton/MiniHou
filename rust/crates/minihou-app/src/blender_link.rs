//! Blender discovery and the process boundary for the native editor.
use std::{
    env, fs, io,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn version(path: &Path) -> Option<(u32, u32)> {
    let text = path.to_string_lossy();
    text.split(['/', '\\']).find_map(|part| {
        let suffix = part.strip_prefix("Blender ").or_else(|| part.strip_prefix("Blender-"))?;
        let (major, minor) = suffix.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    })
}

fn label(path: &Path) -> String {
    match version(path) {
        Some((major, minor)) => format!("Blender {major}.{minor}"),
        None => "Blender ?".to_owned(),
    }
}

fn add_exe(found: &mut Vec<(String, PathBuf)>, path: PathBuf) {
    if path.is_file() && !found.iter().any(|(_, p)| p == &path) {
        found.push((label(&path), path));
    }
}

/// Scan explicit executables and the installation roots used by the Python editor.
/// Keeping the roots injectable avoids depending on the machine's Blender installation in tests.
pub fn find_installs_in(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for root in roots {
        add_exe(&mut found, root.clone());
        add_exe(&mut found, root.join("blender.exe"));
        add_exe(&mut found, root.join("blender"));
        for parent in [root.join("Blender Foundation"), root.join("Programs/Blender Foundation")] {
            if let Ok(entries) = fs::read_dir(parent) {
                let mut paths: Vec<_> = entries.flatten().map(|e| e.path().join("blender.exe")).collect();
                paths.sort_by_key(|p| std::cmp::Reverse(version(p)));
                for path in paths { add_exe(&mut found, path); }
            }
        }
        add_exe(&mut found, root.join("Steam/steamapps/common/Blender/blender.exe"));
    }
    found
}

pub fn find_installs() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    for name in ["BLENDER", "BLENDER_EXE"] {
        if let Some(path) = env::var_os(name) { roots.push(PathBuf::from(path)); }
    }
    if let Some(path) = env::var_os("PATH") {
        roots.extend(env::split_paths(&path));
    }
    #[cfg(windows)]
    for name in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA", "ProgramFiles(x86)"] {
        if let Some(path) = env::var_os(name) { roots.push(PathBuf::from(path)); }
    }
    #[cfg(not(windows))]
    roots.extend([PathBuf::from("/usr/bin"), PathBuf::from("/snap/bin"), PathBuf::from("/opt")]);
    find_installs_in(&roots)
}

fn addon_zip_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut zips: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
            let name = name.to_ascii_lowercase();
            p.is_file() && name.ends_with(".zip") &&
                (name.starts_with("minihou_link") || name.starts_with("minihou-link"))
        }).collect();
        zips.sort();
        if let Some(zip) = zips.pop() { return Some(zip); }
    }
    None
}

pub fn addon_zip() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut dirs = vec![dir.to_path_buf(), dir.join("dist")];
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) {
        dirs.push(root.join("dist"));
    }
    addon_zip_in(&dirs)
}

/// Blender's startup script loads the packaged add-on, sets the graph and connects to the
/// same default endpoint as tools/launch_blender.py. The native host must provide that endpoint.
fn open_args(graph: &Path) -> Vec<OsString> {
    let graph = serde_json::to_string(graph.to_string_lossy().as_ref()).expect("path is serializable");
    let zip = addon_zip().map(|p| serde_json::to_string(p.to_string_lossy().as_ref()).expect("path is serializable"));
    let source = match zip {
        Some(zip) => format!(r#"import bpy, importlib, sys, tempfile, zipfile
from pathlib import Path
d = Path(tempfile.mkdtemp(prefix='minihou_link_'))
with zipfile.ZipFile({zip}) as z: z.extractall(d)
sys.path.insert(0, str(d))
import minihou_link
try: minihou_link.unregister()
except Exception: pass
importlib.reload(minihou_link)
minihou_link.register()
bpy.context.scene.minihou_graph_path = {graph}
bpy.context.window_manager.minihou_host = '127.0.0.1'
bpy.context.window_manager.minihou_port = 8765
def minihou_connect():
    from minihou_link import runtime
    runtime.get_runtime().connect('127.0.0.1', 8765)
    return None
bpy.app.timers.register(minihou_connect, first_interval=0.5)
"#),
        None => format!(r#"import bpy, addon_utils
addon_utils.enable('minihou_link', default_set=False)
bpy.context.scene.minihou_graph_path = {graph}
bpy.context.window_manager.minihou_host = '127.0.0.1'
bpy.context.window_manager.minihou_port = 8765
def minihou_connect():
    from minihou_link import runtime
    runtime.get_runtime().connect('127.0.0.1', 8765)
    return None
bpy.app.timers.register(minihou_connect, first_interval=0.5)
"#),
    };
    vec![OsString::from("--python-expr"), OsString::from(source)]
}

pub fn open_blender(exe: &Path, graph: &Path) -> io::Result<Child> {
    if !graph.is_file() { return Err(io::Error::new(io::ErrorKind::NotFound, "graph file does not exist")); }
    let mut command = Command::new(exe);
    command.args(open_args(graph)).stdin(Stdio::null());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x0000_0008 | 0x0000_0200); }
    command.spawn()
}

const INSTALL_SCRIPT: &str = r#"import json, os, shutil, sys, zipfile
import bpy
zip_path, out_path = sys.argv[sys.argv.index('--') + 1:][:2]
res = {'ok': False, 'blender': bpy.app.version_string}
try:
    base = bpy.utils.user_resource('SCRIPTS', path='addons', create=True)
    target = os.path.join(base, 'minihou_link')
    if os.path.isdir(target): shutil.rmtree(target)
    with zipfile.ZipFile(zip_path) as z: z.extractall(base)
    import addon_utils
    bpy.utils.refresh_script_paths()
    addon_utils.modules_refresh()
    mod = addon_utils.enable('minihou_link', default_set=True, persistent=True)
    bpy.ops.wm.save_userpref()
    res.update(ok=mod is not None, path=target)
    if mod is None: res['error'] = 'unpacked, but Blender did not enable the add-on (see its console)'
except Exception as e:
    res['error'] = repr(e)
with open(out_path, 'w', encoding='utf-8') as f: json.dump(res, f)
"#;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> io::Result<Self> {
        for _ in 0..10 {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = env::current_exe()?.parent().ok_or_else(|| io::Error::other("executable has no directory"))?
                .join(format!("minihou_install_{}_{}", std::process::id(), n));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not create temporary install directory"))
    }
}
impl Drop for TempDir { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

pub fn install_addon(exe: &Path, zip: &Path) -> io::Result<String> {
    let zip = zip.canonicalize()?;
    let temp = TempDir::new()?;
    let script = temp.0.join("install.py");
    let result = temp.0.join("result.json");
    fs::write(&script, INSTALL_SCRIPT)?;
    let mut child = Command::new(exe).args(["--background", "--python"])
        .arg(&script).arg("--").arg(&zip).arg(&result)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if child.try_wait()?.is_some() { break; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Blender did not finish within 180 s"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&result).map_err(|_| {
        io::Error::other("Blender ran but did not report a result")
    })?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if report["ok"] == true {
        Ok(format!("Installed and enabled in Blender {}. If Blender is already open, restart it (or enable MiniHou Link in Preferences > Add-ons).",
            report["blender"].as_str().unwrap_or("")))
    } else {
        Err(io::Error::other(report["error"].as_str().unwrap_or("installation failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_install_roots_newest_first_and_deduplicates() {
        let temp = TempDir::new().unwrap();
        for version in ["Blender 4.2", "Blender 5.1"] {
            let dir = temp.0.join("Blender Foundation").join(version);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("blender.exe"), "").unwrap();
        }
        let found = find_installs_in(&[temp.0.clone(), temp.0.clone()]);
        assert_eq!(found.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["Blender 5.1", "Blender 4.2"]);
    }

    #[test]
    fn launch_arguments_contain_graph_and_connection_endpoint() {
        let args = open_args(Path::new(r"C:\graphs\my file.json"));
        assert_eq!(args[0], "--python-expr");
        let source = args[1].to_string_lossy();
        assert!(source.contains("my file.json"));
        assert!(source.contains("127.0.0.1"));
        assert!(source.contains("8765"));
    }

    #[test]
    fn zip_lookup_skips_missing_directory_and_accepts_packaged_name() {
        let temp = TempDir::new().unwrap();
        let zip = temp.0.join("MiniHou_Link-0.1.zip");
        fs::write(&zip, b"zip placeholder").unwrap();
        assert_eq!(addon_zip_in(&[temp.0.join("absent"), temp.0.clone()]), Some(zip));
    }
}

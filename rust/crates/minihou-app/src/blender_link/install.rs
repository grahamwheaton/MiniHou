use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};
use minihou_app::Document;
use super::NEXT_TEMP;

#[path = "../addon_archive.rs"]
mod addon_archive;

fn version(path: &Path) -> Option<(u32, u32)> {
    let text = path.to_string_lossy();
    text.split(['/', '\\']).find_map(|part| {
        let suffix = part
            .strip_prefix("Blender ")
            .or_else(|| part.strip_prefix("Blender-"))?;
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
        for parent in [
            root.join("Blender Foundation"),
            root.join("Programs/Blender Foundation"),
        ] {
            if let Ok(entries) = fs::read_dir(parent) {
                let mut paths: Vec<_> = entries
                    .flatten()
                    .map(|e| e.path().join("blender.exe"))
                    .collect();
                paths.sort_by_key(|p| std::cmp::Reverse(version(p)));
                for path in paths {
                    add_exe(&mut found, path);
                }
            }
        }
        add_exe(
            &mut found,
            root.join("Steam/steamapps/common/Blender/blender.exe"),
        );
    }
    found
}

pub fn find_installs() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    for name in ["BLENDER", "BLENDER_EXE"] {
        if let Some(path) = env::var_os(name) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        roots.extend(env::split_paths(&path));
    }
    #[cfg(windows)]
    for name in [
        "ProgramFiles",
        "ProgramW6432",
        "LOCALAPPDATA",
        "ProgramFiles(x86)",
    ] {
        if let Some(path) = env::var_os(name) {
            roots.push(PathBuf::from(path));
        }
    }
    #[cfg(not(windows))]
    roots.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/snap/bin"),
        PathBuf::from("/opt"),
    ]);
    find_installs_in(&roots)
}

fn addon_zip_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut zips: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                    return false;
                };
                let name = name.to_ascii_lowercase();
                p.is_file()
                    && name.ends_with(".zip")
                    && (name.starts_with("minihou_link") || name.starts_with("minihou-link"))
            })
            .collect();
        zips.sort();
        if let Some(zip) = zips.pop() {
            return Some(zip);
        }
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

fn addon_source_in(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("blender_addon/minihou_link"))
        .find(|path| path.join("__init__.py").is_file())
}

/// `bl_info` version ("0.4.0") declared in an add-on `__init__.py`.
fn version_in_source(text: &str) -> Option<String> {
    text.lines().find(|l| l.trim_start().starts_with("\"version\"")).and_then(|l| {
        let inner = l.split('(').nth(1)?.split(')').next()?;
        Some(inner.split(',').map(|n| n.trim()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("."))
    })
}

/// The add-on source tree whose version equals `wanted`. Search roots can include an unrelated
/// older checkout (an exe copied into another repo folder), which must never be installed.
fn addon_source_matching(roots: &[PathBuf], wanted: &str) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("blender_addon/minihou_link"))
        .find(|path| {
            fs::read_to_string(path.join("__init__.py")).ok().and_then(|t| version_in_source(&t)).as_deref() == Some(wanted)
        })
}

/// Keep generated archives in the OS temp directory for Blender startup and manual installation.
pub fn prepare_addon_zip() -> io::Result<PathBuf> {
    let mut roots = Vec::new();
    for start in [
        env::current_dir().ok(),
        env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
    ]
    .into_iter()
    .flatten()
    {
        roots.extend(start.ancestors().map(Path::to_path_buf));
    }
    // A checkout must package its current source, even if an older release ZIP is nearby.
    let wanted = addon_version();
    let Some(source) = addon_source_matching(&roots, &wanted) else {
        let other = addon_source_in(&roots)
            .and_then(|p| fs::read_to_string(p.join("__init__.py")).ok())
            .and_then(|t| version_in_source(&t));
        return addon_zip().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, match other {
            Some(found) => format!("Add-on sources found beside the app are V{found}, but this app bundles V{wanted}. Not installing a mismatched add-on: build or run the app from the matching checkout, or place a MiniHou_Link*.zip of V{wanted} beside it."),
            None => format!("No MiniHou_Link*.zip or blender_addon/minihou_link/__init__.py (V{wanted}) found beside the app or in its source tree"),
        }));
    };
    let temp = TempDir::new()?;
    let zip = temp.0.join("MiniHou_Link.zip");
    addon_archive::build(&source, &zip)?;
    // These archives also back the Save ZIP action and must outlive the helper process.
    std::mem::forget(temp);
    Ok(zip)
}

/// Blender's startup script loads the packaged add-on, sets the graph and connects to the
/// same default endpoint as tools/launch_blender.py. The native host must provide that endpoint.
fn open_args(graph: &Path, zip: Option<&Path>, report: &Path) -> Vec<OsString> {
    let graph =
        serde_json::to_string(graph.to_string_lossy().as_ref()).expect("path is serializable");
    let zip = zip.map(|p| {
        serde_json::to_string(p.to_string_lossy().as_ref()).expect("path is serializable")
    });
    let source = match zip {
        Some(zip) => format!(
            r#"import bpy, importlib, sys, tempfile, zipfile
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
"#
        ),
        None => format!(
            r#"import bpy, addon_utils
addon_utils.enable('minihou_link', default_set=False)
bpy.context.scene.minihou_graph_path = {graph}
bpy.context.window_manager.minihou_host = '127.0.0.1'
bpy.context.window_manager.minihou_port = 8765
def minihou_connect():
    from minihou_link import runtime
    runtime.get_runtime().connect('127.0.0.1', 8765)
    return None
bpy.app.timers.register(minihou_connect, first_interval=0.5)
"#
        ),
    };
    let source = source.replace(
        "bpy.app.timers.register(minihou_connect,",
        "bpy.app.timers.register(lambda: minihou_guard(minihou_connect),",
    );
    let source = serde_json::to_string(&source).expect("script is serializable");
    let report =
        serde_json::to_string(report.to_string_lossy().as_ref()).expect("path is serializable");
    let guarded = format!(
        r#"import json, time, traceback
from pathlib import Path
def minihou_report(ok, error=''):
    path = Path({report})
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(dict(ok=ok, error=error)), encoding='utf-8')
    temporary.replace(path)
def minihou_guard(callback):
    try: return callback()
    except Exception:
        minihou_report(False, traceback.format_exc())
        return None
def minihou_bootstrap():
    exec({source}, globals())
    deadline = time.monotonic() + 30
    def check():
        if Path({report}).exists(): return None
        from minihou_link import runtime
        rt = runtime.get_runtime()
        if rt.state == 'connected':
            minihou_report(True)
            return None
        if time.monotonic() >= deadline:
            minihou_report(False, rt.detail or rt.last_error or 'Timed out connecting to 127.0.0.1:8765')
            return None
        return 0.25
    bpy.app.timers.register(lambda: minihou_guard(check), first_interval=1.0)
minihou_guard(minihou_bootstrap)
"#
    );
    vec![OsString::from("--python-expr"), OsString::from(guarded)]
}

/// Version of the bundled Blender add-on (bl_info in blender_addon/minihou_link/__init__.py), e.g. "0.1.5".
pub fn addon_version() -> String {
    const SOURCE: &str = include_str!("../../../../../blender_addon/minihou_link/__init__.py");
    version_in_source(SOURCE).unwrap_or_else(|| "unknown".into())
}

pub fn open_blender(exe: &Path, graph: &Path) -> io::Result<Child> {
    if !graph.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "graph file does not exist",
        ));
    }
    let zip = prepare_addon_zip()?;
    let temp = TempDir::new()?;
    let report = temp.0.join("launch.json");
    let log = temp.0.join("blender.log");
    let output = fs::File::create(&log)?;
    let mut command = Command::new(exe);
    command
        .args(open_args(graph, Some(&zip), &report))
        .stdin(Stdio::null())
        .stderr(output.try_clone()?)
        .stdout(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "Blender exited with {status}: {}",
                fs::read_to_string(&log).unwrap_or_default()
            )));
        }
        if report.is_file() {
            let result: serde_json::Value = serde_json::from_slice(&fs::read(&report)?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if result["ok"] == true {
                return Ok(child);
            }
            let error = result["error"]
                .as_str()
                .unwrap_or("Blender startup failed")
                .to_owned();
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(io::Error::other(error));
        }
        if Instant::now() >= deadline {
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Blender did not report a connection within 60 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// A launch snapshot lives until the launched Blender exits; it never changes the editor's save point.
pub struct LaunchGraph {
    pub path: PathBuf,
    _temporary: Option<TempDir>,
}
pub fn prepare_launch_graph(doc: &Document, path: &Path, dirty: bool) -> io::Result<LaunchGraph> {
    if path.is_file() && !dirty {
        return Ok(LaunchGraph {
            path: path.canonicalize()?,
            _temporary: None,
        });
    }
    let temporary = TempDir::new()?;
    let path = temporary.0.join("current-document.json");
    fs::write(&path, doc.text())?;
    Ok(LaunchGraph {
        path,
        _temporary: Some(temporary),
    })
}

const INSTALL_SCRIPT: &str = r#"import json, os, shutil, sys, zipfile, traceback
import bpy
zip_path, out_path = sys.argv[sys.argv.index('--') + 1:][:2]
res = {'ok': False, 'blender': bpy.app.version_string}
try:
    import addon_utils
    with zipfile.ZipFile(zip_path) as z:
        if 'minihou_link/__init__.py' not in z.namelist():
            raise RuntimeError('ZIP must contain minihou_link/__init__.py')
    base = bpy.utils.user_resource('SCRIPTS', path='addons', create=True)
    target = os.path.join(base, 'minihou_link')
    addon_utils.disable('minihou_link', default_set=True)
    if os.path.isdir(target): shutil.rmtree(target)
    with zipfile.ZipFile(zip_path) as z: z.extractall(base)
    bpy.utils.refresh_script_paths()
    addon_utils.modules(refresh=True)
    errors = []
    mod = addon_utils.enable('minihou_link', default_set=True, persistent=True,
        handle_error=lambda _ex: errors.append(traceback.format_exc()))
    bpy.ops.wm.save_userpref()
    res.update(ok=mod is not None, path=target)
    if mod is None: res['error'] = '\n'.join(errors) or 'Blender did not enable minihou_link'
except Exception as e:
    res['error'] = repr(e)
with open(out_path, 'w', encoding='utf-8') as f: json.dump(res, f)
"#;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> io::Result<Self> {
        for _ in 0..10 {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("minihou_install_{}_{}", std::process::id(), n));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create temporary install directory",
        ))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install_args(zip: &Path, result: &Path) -> Vec<OsString> {
    vec![
        "--background".into(),
        "--python-exit-code".into(),
        "1".into(),
        "--python-expr".into(),
        INSTALL_SCRIPT.into(),
        "--".into(),
        zip.into(),
        result.into(),
    ]
}

fn blender_running() -> bool {
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq blender.exe", "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
    };
    #[cfg(not(windows))]
    let output = Command::new("pgrep").args(["-x", "blender"]).output();
    output.is_ok_and(|out| {
        out.status.success() && {
            #[cfg(windows)]
            {
                String::from_utf8_lossy(&out.stdout)
                    .to_ascii_lowercase()
                    .contains("\"blender.exe\"")
            }
            #[cfg(not(windows))]
            {
                !out.stdout.is_empty()
            }
        }
    })
}

pub fn install_addon(exe: &Path, zip: &Path) -> io::Result<String> {
    let zip = zip.canonicalize()?;
    let temp = TempDir::new()?;
    let result = temp.0.join("result.json");
    let log = temp.0.join("blender.log");
    let output = fs::File::create(&log)?;
    let mut command = Command::new(exe);
    command
        .args(install_args(&zip, &result))
        .stdin(Stdio::null())
        .stderr(output.try_clone()?)
        .stdout(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Blender did not finish within 180 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let diagnostics = || fs::read_to_string(&log).unwrap_or_default();
    if !status.success() {
        return Err(io::Error::other(format!(
            "Blender exited with {status}: {}",
            diagnostics()
        )));
    }
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&result).map_err(|_| {
        io::Error::other(format!(
            "Blender ran but did not report a result: {}",
            diagnostics()
        ))
    })?)
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if report["ok"] == true {
        let running = blender_running();
        Ok(format!(
            "Installed to {} and enabled for Blender {}. {}",
            report["path"].as_str().unwrap_or("?"),
            report["blender"].as_str().unwrap_or(""),
            if running {
                "Blender is already running. Enable MiniHou Link in Preferences (reload/restart Blender for this update), then use Connect in the MiniHou N-panel or leave Auto-connect on."
            } else {
                "Open Blender to connect automatically, or press Connect in its MiniHou N-panel."
            }
        ))
    } else {
        Err(io::Error::other(
            report["error"].as_str().unwrap_or("installation failed"),
        ))
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
        assert_eq!(
            found.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            ["Blender 5.1", "Blender 4.2"]
        );
    }

    #[test]
    fn launch_arguments_contain_graph_and_connection_endpoint() {
        let args = open_args(
            Path::new(r"C:\graphs\my file.json"),
            Some(Path::new("addon's ZIP.zip")),
            Path::new("launch.json"),
        );
        assert_eq!(args[0], "--python-expr");
        let source = args[1].to_string_lossy();
        assert!(source.contains("my file.json"));
        assert!(source.contains("127.0.0.1"));
        assert!(source.contains("8765"));
        assert!(source.contains("addon's ZIP.zip"));
        assert!(source.contains("launch.json"));
        assert!(source.contains("traceback.format_exc()"));
    }

    #[test]
    fn launch_graph_snapshots_untitled_and_dirty_documents_without_saving_editor() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let temp = TempDir::new().unwrap();
        let original = temp.0.join("untitled.json");
        let snapshot = prepare_launch_graph(&doc, &original, false).unwrap();
        assert!(!original.exists());
        assert_eq!(fs::read_to_string(&snapshot.path).unwrap(), doc.text());
        let snapshot_path = snapshot.path.clone();
        drop(snapshot);
        assert!(!snapshot_path.exists());

        fs::write(&original, "previous saved document").unwrap();
        let snapshot = prepare_launch_graph(&doc, &original, true).unwrap();
        assert_ne!(snapshot.path, original);
        assert_eq!(fs::read_to_string(&snapshot.path).unwrap(), doc.text());
        assert_eq!(
            fs::read_to_string(&original).unwrap(),
            "previous saved document"
        );
        let saved = prepare_launch_graph(&doc, &original, false).unwrap();
        assert_eq!(saved.path, original.canonicalize().unwrap());
        drop(saved);
        assert!(original.exists());
    }

    #[test]
    fn install_arguments_keep_paths_separate_and_enable_addon() {
        let zip = Path::new(r"C:\a folder\Bob's addon.zip");
        let result = Path::new(r"C:\temp dir\result.json");
        let args = install_args(zip, result);
        assert_eq!(args[0], "--background");
        assert_eq!(args[3], "--python-expr");
        assert_eq!(args[5], "--");
        assert_eq!(args[6], zip.as_os_str());
        assert_eq!(args[7], result.as_os_str());
        let source = args[4].to_string_lossy();
        assert!(source.contains("addon_utils.modules(refresh=True)"));
        assert!(source.contains("default_set=True, persistent=True"));
        assert!(source.contains("save_userpref()"));
        assert!(!source.contains("graph"));
    }

    #[test]
    fn source_discovery_requires_addon_entrypoint() {
        let temp = TempDir::new().unwrap();
        let source = temp.0.join("blender_addon/minihou_link");
        fs::create_dir_all(&source).unwrap();
        assert!(addon_source_in(&[temp.0.clone()]).is_none());
        fs::write(source.join("__init__.py"), "").unwrap();
        assert_eq!(
            addon_source_in(&[temp.0.join("missing"), temp.0.clone()]),
            Some(source)
        );
    }

    #[test]
    fn addon_zip_has_top_level_folder_valid_headers_and_no_python_cache() {
        let temp = TempDir::new().unwrap();
        let source = temp.0.join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(source.join("__pycache__")).unwrap();
        fs::write(source.join("__init__.py"), b"123456789").unwrap();
        fs::write(source.join("nested/data.txt"), b"payload").unwrap();
        fs::write(source.join("__pycache__/junk.pyc"), b"ignore").unwrap();
        fs::write(source.join("stale.pyc"), b"ignore").unwrap();
        let zip = temp.0.join("addon.zip");
        addon_archive::build(&source, &zip).unwrap();
        let bytes = fs::read(&zip).unwrap();
        let u16_at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let end = bytes.len() - 22;
        assert_eq!(u32_at(end), 0x06054b50);
        assert_eq!(u16_at(end + 10), 2);
        let mut central = u32_at(end + 16) as usize;
        assert_eq!(central + u32_at(end + 12) as usize, end);
        for (name, payload, crc) in [
            (
                "minihou_link/__init__.py",
                b"123456789".as_slice(),
                0xcbf43926,
            ),
            (
                "minihou_link/nested/data.txt",
                b"payload".as_slice(),
                0x422c6a15,
            ),
        ] {
            assert_eq!(u32_at(central), 0x02014b50);
            let local = u32_at(central + 42) as usize;
            assert_eq!(u32_at(local), 0x04034b50);
            assert_eq!(u16_at(local + 6), 0x800); // UTF-8 names
            assert_eq!(u16_at(local + 8), 0); // stored
            assert_eq!(u32_at(local + 14), crc);
            assert_eq!(u32_at(central + 16), crc);
            assert_eq!(u16_at(local + 26) as usize, name.len());
            assert_eq!(&bytes[local + 30..local + 30 + name.len()], name.as_bytes());
            assert_eq!(
                &bytes[central + 46..central + 46 + name.len()],
                name.as_bytes()
            );
            let data = local + 30 + name.len();
            assert_eq!(u32_at(local + 18) as usize, payload.len());
            assert_eq!(&bytes[data..data + payload.len()], payload);
            central += 46 + name.len();
        }
        assert_eq!(central, end);
        addon_archive::build(&source, &zip).unwrap();
        assert_eq!(fs::read(zip).unwrap(), bytes); // deterministic output
        assert!(addon_archive::build(&temp.0.join("missing"), &temp.0.join("bad.zip")).is_err());
    }

    #[test]
    fn zip_lookup_skips_missing_directory_and_accepts_packaged_name() {
        let temp = TempDir::new().unwrap();
        let zip = temp.0.join("MiniHou_Link-0.1.zip");
        fs::write(&zip, b"zip placeholder").unwrap();
        assert_eq!(
            addon_zip_in(&[temp.0.join("absent"), temp.0.clone()]),
            Some(zip)
        );
    }
}

#[cfg(test)]
mod addon_version_tests {
    #[test] fn bundled_addon_version_is_parsed() {
        let v = super::addon_version();
        assert!(v.split('.').count() >= 2 && v.split('.').all(|n| n.parse::<u32>().is_ok()), "got {v}");
    }
}

#[cfg(test)]
mod addon_source_version_tests {
    use super::*;
    fn tree(root: &Path, version: &str) {
        let dir = root.join("blender_addon/minihou_link");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("__init__.py"), format!("bl_info = {{
    \"version\": ({version}),
}}
")).unwrap();
    }
    #[test]
    fn install_never_packages_a_mismatched_checkout() {
        let temp = TempDir::new().unwrap();
        let (old, new) = (temp.0.join("old"), temp.0.join("new"));
        tree(&old, "0, 1, 7");
        tree(&new, "0, 4, 0");
        // The older checkout comes first in the search order, as when the exe sits inside it.
        let roots = vec![old.clone(), new.clone()];
        assert_eq!(addon_source_matching(&roots, "0.4.0"), Some(new.join("blender_addon/minihou_link")));
        assert_eq!(addon_source_matching(&roots, "0.1.7"), Some(old.join("blender_addon/minihou_link")));
        assert_eq!(addon_source_matching(&roots, "9.9.9"), None);
        assert_eq!(version_in_source("    \"version\": (0, 4, 0),
").as_deref(), Some("0.4.0"));
    }
}

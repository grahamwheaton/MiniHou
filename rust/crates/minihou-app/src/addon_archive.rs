//! Small, dependency-free ZIP writer for the bundled Python add-on (stored entries).
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        if name == "__pycache__" || name.to_string_lossy().ends_with(".pyc") {
            continue;
        }
        // Do not follow symlinks out of the add-on source tree.
        if kind.is_dir() {
            collect(&entry.path(), files)?;
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

pub(super) fn build(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.join("__init__.py").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Missing add-on source: {}", source.display()),
        ));
    }
    let mut files = Vec::new();
    collect(source, &mut files)?;
    files.sort();
    let mut archive = Vec::new();
    let mut central = Vec::new();
    let too_large = || io::Error::other("add-on exceeds ZIP32 limits");
    let count = u16::try_from(files.len()).map_err(|_| too_large())?;
    for file in files {
        let name = format!(
            "minihou_link/{}",
            file.strip_prefix(source)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        );
        let bytes = fs::read(file)?;
        let size = u32::try_from(bytes.len()).map_err(|_| too_large())?;
        let offset = u32::try_from(archive.len()).map_err(|_| too_large())?;
        let name_len = u16::try_from(name.len()).map_err(|_| too_large())?;
        let crc = crc32(&bytes);
        archive.extend(0x04034b50u32.to_le_bytes());
        for v in [20u16, 0x800, 0, 0, 33] {
            archive.extend(v.to_le_bytes());
        }
        for v in [crc, size, size] {
            archive.extend(v.to_le_bytes());
        }
        for v in [name_len, 0] {
            archive.extend(v.to_le_bytes());
        }
        archive.extend(name.as_bytes());
        archive.extend(bytes);
        central.extend(0x02014b50u32.to_le_bytes());
        for v in [20u16, 20, 0x800, 0, 0, 33] {
            central.extend(v.to_le_bytes());
        }
        for v in [crc, size, size] {
            central.extend(v.to_le_bytes());
        }
        for v in [name_len, 0, 0, 0, 0] {
            central.extend(v.to_le_bytes());
        }
        for v in [0u32, offset] {
            central.extend(v.to_le_bytes());
        }
        central.extend(name.as_bytes());
    }
    let offset = u32::try_from(archive.len()).map_err(|_| too_large())?;
    let size = u32::try_from(central.len()).map_err(|_| too_large())?;
    archive.extend(central);
    archive.extend(0x06054b50u32.to_le_bytes());
    for v in [0u16, 0, count, count] {
        archive.extend(v.to_le_bytes());
    }
    for v in [size, offset] {
        archive.extend(v.to_le_bytes());
    }
    archive.extend(0u16.to_le_bytes());
    fs::File::create(destination)?.write_all(&archive)
}

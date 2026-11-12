mod config;
mod desktop;
mod integrate;
mod watch;

pub use config::Dirs;
pub use integrate::{Integrated, integrate, list, remove};
pub use watch::{Change, sync, watch};

use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use backhand::{BackhandError, FilesystemReader, InnerNode, SquashfsFileReader};

const MAX_DESKTOP_ENTRY_SIZE: u64 = 1 << 20;
const MAX_ICON_SIZE: u64 = 16 << 20;
const MAX_SYMLINK_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppImageType {
    Type1,
    Type2,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    NotAppImage,
    UnsupportedType1,
    UnsupportedPayload,
    Squashfs(BackhandError),
    MissingDesktopEntry,
    AlreadyIntegrated(String),
    DesktopEntryConflict(PathBuf),
    DestinationExists(PathBuf),
    NotIntegrated(String),
    WatchedDirGone(PathBuf),
    NoHomeDir,
    Config(PathBuf, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "{error}"),
            Error::NotAppImage => write!(f, "not an AppImage"),
            Error::UnsupportedType1 => write!(f, "type 1 AppImages are not supported"),
            Error::UnsupportedPayload => write!(f, "payload is not a squashfs image"),
            Error::Squashfs(error) => write!(f, "cannot read squashfs payload: {error}"),
            Error::MissingDesktopEntry => write!(f, "no desktop entry in the AppImage root"),
            Error::AlreadyIntegrated(id) => write!(f, "already integrated as {id}"),
            Error::DesktopEntryConflict(path) => {
                write!(f, "{} exists and was not created by appimage-integrator", path.display())
            }
            Error::DestinationExists(path) => write!(f, "{} already exists", path.display()),
            Error::NotIntegrated(id) => write!(f, "no integrated AppImage with id {id}"),
            Error::WatchedDirGone(path) => write!(f, "{} was moved or deleted", path.display()),
            Error::NoHomeDir => write!(f, "cannot determine the home directory"),
            Error::Config(path, message) => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<BackhandError> for Error {
    fn from(error: BackhandError) -> Self {
        Error::Squashfs(error)
    }
}

#[derive(Debug)]
pub struct Icon {
    pub path: PathBuf,
    pub extension: &'static str,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct AppImageInfo {
    pub desktop_file: String,
    pub desktop_entry: String,
    pub icon: Option<Icon>,
}

impl AppImageInfo {
    pub fn entry(&self, key: &str) -> Option<&str> {
        desktop::value(&self.desktop_entry, key)
    }

    pub fn name(&self) -> Option<&str> {
        self.entry("Name")
    }

    pub fn version(&self) -> Option<&str> {
        self.entry("X-AppImage-Version")
    }
}

pub fn appimage_type(path: &Path) -> io::Result<Option<AppImageType>> {
    let header = read_header(&mut File::open(path)?)?;
    Ok(header.as_ref().and_then(type_from_header))
}

pub fn inspect(path: &Path) -> Result<AppImageInfo, Error> {
    let mut file = File::open(path)?;
    let header = read_header(&mut file)?.ok_or(Error::NotAppImage)?;
    match type_from_header(&header).ok_or(Error::NotAppImage)? {
        AppImageType::Type1 => return Err(Error::UnsupportedType1),
        AppImageType::Type2 => {}
    }

    let offset = payload_offset(&header);
    let mut magic = [0u8; 4];
    file.seek(SeekFrom::Start(offset))?;
    if file.read_exact(&mut magic).is_err() || &magic != b"hsqs" {
        return Err(Error::UnsupportedPayload);
    }
    let fs = FilesystemReader::from_reader_with_offset(BufReader::new(file), offset)?;

    let desktop_path = fs
        .files()
        .map(|node| node.fullpath.as_path())
        .find(|path| {
            path.parent() == Some(Path::new("/"))
                && path.extension().is_some_and(|ext| ext == "desktop")
        })
        .ok_or(Error::MissingDesktopEntry)?;
    let (_, desktop_bytes) =
        read_file(&fs, desktop_path, MAX_DESKTOP_ENTRY_SIZE)?.ok_or(Error::MissingDesktopEntry)?;
    let desktop_entry = String::from_utf8_lossy(&desktop_bytes).into_owned();

    let icon_name = desktop::value(&desktop_entry, "Icon").filter(|name| !name.contains('/'));
    let icon = find_icon(&fs, icon_name)?;

    Ok(AppImageInfo {
        desktop_file: desktop_path.file_name().unwrap().to_string_lossy().into_owned(),
        desktop_entry,
        icon,
    })
}

fn read_header(file: &mut File) -> io::Result<Option<[u8; 64]>> {
    let mut header = [0u8; 64];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(Some(header)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn type_from_header(header: &[u8; 64]) -> Option<AppImageType> {
    if &header[..4] != b"\x7fELF" {
        return None;
    }
    match &header[8..11] {
        b"AI\x01" => Some(AppImageType::Type1),
        b"AI\x02" => Some(AppImageType::Type2),
        _ => None,
    }
}

// The payload starts where the ELF runtime ends: section header offset plus the size of the section header table.
fn payload_offset(header: &[u8; 64]) -> u64 {
    let u16_at = |i: usize| u64::from(u16::from_le_bytes([header[i], header[i + 1]]));
    if header[4] == 1 {
        let table = u64::from(u32::from_le_bytes(header[0x20..0x24].try_into().unwrap()));
        table + u16_at(0x2E) * u16_at(0x30)
    } else {
        let table = u64::from_le_bytes(header[0x28..0x30].try_into().unwrap());
        table.saturating_add(u16_at(0x3A) * u16_at(0x3C))
    }
}

fn find_icon(fs: &FilesystemReader, name: Option<&str>) -> Result<Option<Icon>, Error> {
    let mut candidates = Vec::new();
    if let Some(name) = name {
        let (scalable, largest_png) = hicolor_icons(fs, name);
        candidates.extend(scalable);
        candidates.push(PathBuf::from(format!("/{name}.svg")));
        candidates.extend(largest_png);
        candidates.push(PathBuf::from(format!("/{name}.png")));
    }
    candidates.push(PathBuf::from("/.DirIcon"));

    for candidate in candidates {
        if let Some((path, data)) = read_file(fs, &candidate, MAX_ICON_SIZE)? {
            return Ok(Some(Icon { path, extension: icon_extension(&data), data }));
        }
    }
    Ok(None)
}

fn hicolor_icons(fs: &FilesystemReader, name: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let svg_name = format!("{name}.svg");
    let png_name = format!("{name}.png");
    let mut scalable = None;
    let mut largest_png: Option<(u32, PathBuf)> = None;
    for node in fs.files() {
        let Ok(relative) = node.fullpath.strip_prefix("/usr/share/icons/hicolor") else {
            continue;
        };
        let mut parts = relative.iter().map(|part| part.to_string_lossy());
        let (Some(size), Some(context), Some(file), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if context != "apps" {
            continue;
        }
        if size == "scalable" && file == svg_name.as_str() {
            scalable = Some(node.fullpath.clone());
        } else if file == png_name.as_str()
            && let Some(width) = size.split('x').next().and_then(|width| width.parse::<u32>().ok())
            && largest_png.as_ref().is_none_or(|(best, _)| width > *best)
        {
            largest_png = Some((width, node.fullpath.clone()));
        }
    }
    (scalable, largest_png.map(|(_, path)| path))
}

fn read_file(
    fs: &FilesystemReader,
    path: &Path,
    limit: u64,
) -> Result<Option<(PathBuf, Vec<u8>)>, Error> {
    let Some((path, file)) = resolve(fs, path) else {
        return Ok(None);
    };
    let mut data = Vec::new();
    fs.file(file).reader().take(limit).read_to_end(&mut data)?;
    Ok(Some((path, data)))
}

fn resolve<'a>(fs: &'a FilesystemReader, path: &Path) -> Option<(PathBuf, &'a SquashfsFileReader)> {
    let mut path = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_DEPTH {
        let node = fs.files().find(|node| node.fullpath == path)?;
        match &node.inner {
            InnerNode::File(file) => return Some((path, file)),
            InnerNode::Symlink(symlink) => path = normalize(&path.parent()?.join(&symlink.link)),
            _ => return None,
        }
    }
    None
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                normalized.pop();
            }
            _ => {}
        }
    }
    normalized
}

fn icon_extension(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG") {
        "png"
    } else if data.starts_with(b"/* XPM") {
        "xpm"
    } else {
        "svg"
    }
}

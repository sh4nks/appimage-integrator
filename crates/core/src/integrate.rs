use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::{Dirs, Error, desktop, inspect};

#[derive(Debug, PartialEq)]
pub struct Integrated {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub appimage: PathBuf,
    pub desktop_file: PathBuf,
    pub icon: Option<PathBuf>,
}

enum Replace {
    Nothing,
    #[cfg(feature = "update")]
    Id(String),
    Existing,
}

pub fn integrate(file: &Path, dirs: &Dirs) -> Result<Integrated, Error> {
    Ok(integrate_into(file, Some(&dirs.appimages), dirs, Replace::Nothing)?.0)
}

pub fn replace(file: &Path, dirs: &Dirs) -> Result<Integrated, Error> {
    let (new, old) = integrate_into(file, Some(&dirs.appimages), dirs, Replace::Existing)?;
    if let Some(old) = old
        && old.appimage != new.appimage
    {
        remove_if_present(&old.appimage)?;
    }
    Ok(new)
}

pub(crate) fn integrate_in_place(file: &Path, dirs: &Dirs) -> Result<Integrated, Error> {
    Ok(integrate_into(file, None, dirs, Replace::Nothing)?.0)
}

#[cfg(feature = "update")]
pub(crate) fn replace_integration(old: &Integrated, file: &Path, dirs: &Dirs) -> Result<Integrated, Error> {
    let (new, _) = integrate_into(file, None, dirs, Replace::Id(old.id.clone()))?;
    if new.id != old.id {
        fs::remove_file(&old.desktop_file)?;
    }
    if old.icon != new.icon
        && let Some(icon) = &old.icon
    {
        remove_if_present(icon)?;
    }
    Ok(new)
}

fn integrate_into(
    file: &Path,
    destination: Option<&Path>,
    dirs: &Dirs,
    replace: Replace,
) -> Result<(Integrated, Option<Integrated>), Error> {
    let info = inspect(file)?;
    let source = fs::canonicalize(file)?;
    let desktop_file = dirs.desktop_entries.join(&info.desktop_file);
    let id = entry_id(&desktop_file);
    let previous = match fs::read_to_string(&desktop_file) {
        Ok(existing) => match parse(&desktop_file, &existing, dirs) {
            Some(previous) => Some(previous),
            None => return Err(Error::DesktopEntryConflict(desktop_file)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(previous) = &previous {
        let replaceable = match replace {
            Replace::Nothing => false,
            #[cfg(feature = "update")]
            Replace::Id(replaced) => replaced == id,
            Replace::Existing => true,
        };
        if !replaceable && previous.appimage != source {
            return Err(Error::AlreadyIntegrated(id));
        }
    }

    let appimage = match destination {
        Some(dir) => {
            fs::create_dir_all(dir)?;
            fs::canonicalize(dir)?.join(source.file_name().unwrap())
        }
        None => source.clone(),
    };
    if source != appimage {
        let takes_over_path = previous.as_ref().is_some_and(|previous| previous.appimage == appimage);
        if appimage.exists() && !takes_over_path {
            return Err(Error::DestinationExists(appimage));
        }
        move_file(&source, &appimage)?;
    }
    let mut permissions = fs::metadata(&appimage)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(&appimage, permissions)?;

    let icon = match &info.icon {
        Some(icon) => {
            fs::create_dir_all(&dirs.icons)?;
            let path = dirs.icons.join(format!("{id}.{}", icon.extension));
            fs::write(&path, &icon.data)?;
            Some(path)
        }
        None => None,
    };
    if let Some(previous) = &previous
        && previous.icon != icon
        && let Some(stale) = &previous.icon
    {
        remove_if_present(stale)?;
    }

    let entry = desktop::rewrite(
        &info.desktop_entry,
        &appimage.to_string_lossy(),
        icon.as_ref().map(|path| path.to_string_lossy()).as_deref(),
    );
    fs::create_dir_all(&dirs.desktop_entries)?;
    fs::write(&desktop_file, &entry)?;

    Ok((parse(&desktop_file, &entry, dirs).unwrap(), previous))
}

pub fn list(dirs: &Dirs) -> Result<Vec<Integrated>, Error> {
    let entries = match fs::read_dir(&dirs.desktop_entries) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut integrated = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "desktop")
            && let Ok(content) = fs::read_to_string(&path)
            && let Some(item) = parse(&path, &content, dirs)
        {
            integrated.push(item);
        }
    }
    integrated.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(integrated)
}

pub fn remove(id: &str, dirs: &Dirs, keep_appimage: bool) -> Result<Integrated, Error> {
    let desktop_file = dirs.desktop_entries.join(format!("{id}.desktop"));
    let integrated = fs::read_to_string(&desktop_file)
        .ok()
        .and_then(|content| parse(&desktop_file, &content, dirs))
        .ok_or_else(|| Error::NotIntegrated(id.to_string()))?;

    if !keep_appimage {
        remove_if_present(&integrated.appimage)?;
    }
    if let Some(icon) = &integrated.icon {
        remove_if_present(icon)?;
    }
    fs::remove_file(&desktop_file)?;
    Ok(integrated)
}

fn parse(desktop_file: &Path, content: &str, dirs: &Dirs) -> Option<Integrated> {
    let appimage = desktop::value(content, desktop::APPIMAGE_KEY)?;
    let icon = desktop::value(content, "Icon")
        .map(|icon| PathBuf::from(desktop::unescape(icon)))
        .filter(|icon| icon.starts_with(&dirs.icons));
    Some(Integrated {
        id: entry_id(desktop_file),
        name: desktop::value(content, "Name").map(str::to_string),
        version: desktop::value(content, "X-AppImage-Version").map(str::to_string),
        appimage: PathBuf::from(desktop::unescape(appimage)),
        desktop_file: desktop_file.to_path_buf(),
        icon,
    })
}

fn entry_id(desktop_file: &Path) -> String {
    desktop_file.file_stem().unwrap().to_string_lossy().into_owned()
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            // a running AppImage keeps reading its old inode, so never write into it
            remove_if_present(to)?;
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
        result => result,
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

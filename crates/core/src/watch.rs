use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use inotify::{EventMask, Inotify, WatchMask};

use crate::integrate::integrate_in_place;
use crate::{Dirs, Error, Integrated, appimage_type, list, remove};

#[derive(Debug)]
pub enum Change {
    Integrated(Integrated),
    Removed(Integrated),
    Failed(PathBuf, Error),
}

pub fn sync(dirs: &Dirs, report: &mut dyn FnMut(Change)) -> Result<(), Error> {
    let watched = watched_dirs(dirs)?;
    for item in list(dirs)? {
        let is_watched = item.appimage.parent().is_some_and(|parent| watched.iter().any(|dir| dir == parent));
        if is_watched && !item.appimage.exists() {
            report(Change::Removed(remove(&item.id, dirs, true)?));
        }
    }
    for dir in &watched {
        let mut files = Vec::new();
        for entry in fs::read_dir(dir)? {
            files.push(entry?.path());
        }
        files.sort();
        for file in files {
            integrate_new(&file, dirs, report)?;
        }
    }
    Ok(())
}

pub fn watch(dirs: &Dirs, report: &mut dyn FnMut(Change)) -> Result<(), Error> {
    let mut inotify = Inotify::init()?;
    let mut watches = Vec::new();
    for dir in watched_dirs(dirs)? {
        let descriptor = inotify.watches().add(
            &dir,
            WatchMask::CLOSE_WRITE
                | WatchMask::MOVED_TO
                | WatchMask::DELETE
                | WatchMask::MOVED_FROM
                | WatchMask::DELETE_SELF
                | WatchMask::MOVE_SELF,
        )?;
        watches.push((descriptor, dir));
    }
    sync(dirs, report)?;

    let mut buffer = [0u8; 4096];
    loop {
        for event in inotify.read_events_blocking(&mut buffer)? {
            let Some((_, dir)) = watches.iter().find(|(descriptor, _)| *descriptor == event.wd) else {
                continue;
            };
            let gone = EventMask::DELETE_SELF | EventMask::MOVE_SELF | EventMask::IGNORED;
            if event.mask.intersects(gone) {
                return Err(Error::WatchedDirGone(dir.clone()));
            }
            let Some(name) = event.name else { continue };
            if event.mask.contains(EventMask::ISDIR) {
                continue;
            }
            let file = dir.join(name);
            if event.mask.intersects(EventMask::CLOSE_WRITE | EventMask::MOVED_TO) {
                integrate_new(&file, dirs, report)?;
            } else if remove_integration(&file, dirs, report)? {
                sync(dirs, report)?;
            }
        }
    }
}

fn watched_dirs(dirs: &Dirs) -> io::Result<Vec<PathBuf>> {
    let mut watched = Vec::new();
    for dir in &dirs.watched {
        fs::create_dir_all(dir)?;
        watched.push(fs::canonicalize(dir)?);
    }
    watched.sort();
    watched.dedup();
    Ok(watched)
}

fn integrate_new(file: &Path, dirs: &Dirs, report: &mut dyn FnMut(Change)) -> Result<(), Error> {
    if !matches!(appimage_type(file), Ok(Some(_)))
        || list(dirs)?.iter().any(|item| item.appimage == file)
    {
        return Ok(());
    }
    report(match integrate_in_place(file, dirs) {
        Ok(integrated) => Change::Integrated(integrated),
        Err(error) => Change::Failed(file.to_path_buf(), error),
    });
    Ok(())
}

fn remove_integration(
    file: &Path,
    dirs: &Dirs,
    report: &mut dyn FnMut(Change),
) -> Result<bool, Error> {
    let Some(item) = list(dirs)?.into_iter().find(|item| item.appimage == file) else {
        return Ok(false);
    };
    report(Change::Removed(remove(&item.id, dirs, true)?));
    Ok(true)
}

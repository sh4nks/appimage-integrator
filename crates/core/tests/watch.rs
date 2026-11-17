mod common;

use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use appimage_integrator_core::{Change, Error, integrate, list, sync, watch};
use common::{dirs, example, example_version};

#[test]
fn sync_integrates_new_files_and_drops_entries_for_deleted_ones() {
    let dirs = dirs("sync-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    let appimage = dirs.appimages.join("Example.AppImage");
    fs::rename(example("sync.AppImage"), &appimage).unwrap();
    fs::write(dirs.appimages.join("notes.txt"), "not an AppImage").unwrap();
    fs::copy(&appimage, dirs.appimages.join("Other.AppImage.part")).unwrap();
    fs::copy(&appimage, dirs.appimages.join(".hidden.AppImage")).unwrap();
    fs::create_dir(dirs.appimages.join("folder")).unwrap();

    let mut changes = Vec::new();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(matches!(&changes[..], [Change::Integrated(item)] if item.id == "example"));

    changes.clear();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(changes.is_empty());

    fs::remove_file(&appimage).unwrap();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(matches!(&changes[..], [Change::Removed(item)] if item.id == "example"));
    assert!(list(&dirs).unwrap().is_empty());
}

#[test]
fn sync_reports_a_second_version_of_an_integrated_app() {
    let dirs = dirs("sync-duplicate-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    fs::rename(example("sync-a.AppImage"), dirs.appimages.join("A.AppImage")).unwrap();
    fs::rename(example("sync-b.AppImage"), dirs.appimages.join("B.AppImage")).unwrap();

    let mut changes = Vec::new();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(matches!(
        &changes[..],
        [Change::Integrated(_), Change::Failed(file, Error::AlreadyIntegrated(_))] if file.ends_with("B.AppImage")
    ));
}

#[test]
fn sync_covers_every_watched_dir_and_leaves_files_in_place() {
    let mut dirs = dirs("sync-multi-dirs");
    let extra = dirs.appimages.with_file_name("Games");
    dirs.watched.push(extra.clone());
    dirs.watched.push(extra.join("../Games"));
    fs::create_dir_all(&extra).unwrap();
    let game = extra.join("Game.AppImage");
    fs::rename(example("sync-multi.AppImage"), &game).unwrap();

    let mut changes = Vec::new();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(matches!(&changes[..], [Change::Integrated(item)] if item.appimage == fs::canonicalize(&game).unwrap()));
    assert!(game.exists());
    assert!(dirs.appimages.is_dir());

    changes.clear();
    fs::remove_file(&game).unwrap();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(matches!(&changes[..], [Change::Removed(_)]));
}

#[test]
fn sync_ignores_entries_outside_the_watched_dirs() {
    let dirs = dirs("sync-outside-dirs");
    let mut elsewhere = dirs.clone();
    elsewhere.appimages = dirs.appimages.with_file_name("Elsewhere");
    let integrated = integrate(&example("sync-outside.AppImage"), &elsewhere).unwrap();
    fs::remove_file(&integrated.appimage).unwrap();

    let mut changes = Vec::new();
    sync(&dirs, &mut |change| changes.push(change)).unwrap();
    assert!(changes.is_empty());
    assert_eq!(list(&dirs).unwrap().len(), 1);
}

#[test]
fn watch_covers_every_watched_dir() {
    let mut dirs = dirs("watch-multi-dirs");
    let extra = dirs.appimages.with_file_name("Games");
    dirs.watched.push(extra.clone());
    let (sender, receiver) = mpsc::channel();
    let watch_dirs = dirs.clone();
    thread::spawn(move || {
        watch(&watch_dirs, &mut |change| {
            let _ = sender.send(change);
        })
    });
    let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();

    let source = example("watch-multi.AppImage");
    while !extra.is_dir() {
        thread::sleep(Duration::from_millis(10));
    }
    let game = extra.join("Game.AppImage");
    fs::rename(source, &game).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("Games/Game.AppImage")));

    fs::rename(&game, dirs.appimages.join("Game.AppImage")).unwrap();
    assert!(matches!(next(), Change::Removed(_)));
    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("Applications/Game.AppImage")));
}

#[test]
fn watch_integrates_a_waiting_version_when_the_integrated_one_is_deleted() {
    let dirs = dirs("watch-takeover-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    let first = dirs.appimages.join("A.AppImage");
    fs::rename(example("takeover-a.AppImage"), &first).unwrap();
    fs::rename(example("takeover-b.AppImage"), dirs.appimages.join("B.AppImage")).unwrap();
    let (sender, receiver) = mpsc::channel();
    let watch_dirs = dirs.clone();
    thread::spawn(move || {
        watch(&watch_dirs, &mut |change| {
            let _ = sender.send(change);
        })
    });
    let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();

    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("A.AppImage")));
    assert!(matches!(next(), Change::Failed(_, Error::AlreadyIntegrated(_))));

    fs::remove_file(&first).unwrap();
    assert!(matches!(next(), Change::Removed(_)));
    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("B.AppImage")));
}

#[test]
fn watch_reacts_to_files_appearing_and_disappearing() {
    let dirs = dirs("watch-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    let (sender, receiver) = mpsc::channel();
    let watch_dirs = dirs.clone();
    thread::spawn(move || {
        watch(&watch_dirs, &mut |change| {
            let _ = sender.send(change);
        })
    });
    let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();

    let moved = dirs.appimages.join("Moved.AppImage");
    fs::rename(example("watch.AppImage"), &moved).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.id == "example"));

    fs::remove_file(&moved).unwrap();
    assert!(matches!(next(), Change::Removed(item) if item.id == "example"));
    assert!(list(&dirs).unwrap().is_empty());

    let copied = dirs.appimages.join("Copied.AppImage");
    fs::copy(example("watch-copy.AppImage"), &copied).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("Copied.AppImage")));

    fs::rename(&copied, dirs.appimages.join("Renamed.AppImage")).unwrap();
    assert!(matches!(next(), Change::Removed(_)));
    assert!(matches!(next(), Change::Integrated(item) if item.appimage.ends_with("Renamed.AppImage")));
}

#[test]
fn watch_refreshes_the_entry_when_the_integrated_file_is_rewritten() {
    let dirs = dirs("watch-rewrite-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    let appimage = dirs.appimages.join("Example.AppImage");
    fs::rename(example("watch-rewrite.AppImage"), &appimage).unwrap();
    let (sender, receiver) = mpsc::channel();
    let watch_dirs = dirs.clone();
    thread::spawn(move || {
        watch(&watch_dirs, &mut |change| {
            let _ = sender.send(change);
        })
    });
    let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.version.as_deref() == Some("1.2.3")));

    fs::rename(example_version("watch-rewrite-moved.AppImage", "2.0.0"), &appimage).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.version.as_deref() == Some("2.0.0")));

    fs::copy(example_version("watch-rewrite-copied.AppImage", "3.0.0"), &appimage).unwrap();
    assert!(matches!(next(), Change::Integrated(item) if item.version.as_deref() == Some("3.0.0")));
    assert_eq!(list(&dirs).unwrap().len(), 1);
}

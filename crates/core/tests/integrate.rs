mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use appimage_integrator_core::{Error, integrate, list, remove, replace};
use common::{PNG, dirs, example, example_version};

#[test]
fn integrates_lists_and_removes() {
    let dirs = dirs("lifecycle-dirs");
    let source = example("lifecycle.AppImage");

    let integrated = integrate(&source, &dirs).unwrap();
    assert_eq!(integrated.id, "example");
    assert_eq!(integrated.name.as_deref(), Some("Example App"));
    assert_eq!(integrated.version.as_deref(), Some("1.2.3"));
    assert!(!source.exists());
    assert_eq!(integrated.appimage.parent(), Some(fs::canonicalize(&dirs.appimages).unwrap().as_path()));
    assert_eq!(fs::metadata(&integrated.appimage).unwrap().permissions().mode() & 0o111, 0o111);

    let icon = integrated.icon.clone().unwrap();
    assert_eq!(icon, dirs.icons.join("example.png"));
    assert_eq!(fs::read(&icon).unwrap(), PNG);

    let entry = fs::read_to_string(&integrated.desktop_file).unwrap();
    let appimage = integrated.appimage.display();
    assert!(entry.contains(&format!("Exec=\"{appimage}\" %U\n")));
    assert!(entry.contains(&format!("TryExec={appimage}\n")));
    assert!(entry.contains(&format!("Icon={}\n", icon.display())));

    let listed = list(&dirs).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].appimage, integrated.appimage);

    remove("example", &dirs, false).unwrap();
    assert!(!integrated.appimage.exists());
    assert!(!integrated.desktop_file.exists());
    assert!(!icon.exists());
    assert!(list(&dirs).unwrap().is_empty());
}

#[test]
fn remove_can_keep_the_appimage() {
    let dirs = dirs("keep-dirs");
    let integrated = integrate(&example("keep.AppImage"), &dirs).unwrap();
    remove("example", &dirs, true).unwrap();
    assert!(integrated.appimage.exists());
    assert!(!integrated.desktop_file.exists());
}

#[test]
fn integrates_in_place_when_already_in_the_appimage_dir() {
    let dirs = dirs("in-place-dirs");
    fs::create_dir_all(&dirs.appimages).unwrap();
    let source = dirs.appimages.join("Example.AppImage");
    fs::rename(example("in-place.AppImage"), &source).unwrap();

    let integrated = integrate(&source, &dirs).unwrap();
    assert_eq!(integrated.appimage, fs::canonicalize(&source).unwrap());
}

#[test]
fn refuses_duplicates_and_conflicts() {
    let dirs = dirs("conflict-dirs");
    integrate(&example("first.AppImage"), &dirs).unwrap();

    let second = example("second.AppImage");
    assert!(matches!(integrate(&second, &dirs), Err(Error::AlreadyIntegrated(id)) if id == "example"));
    assert!(second.exists());

    remove("example", &dirs, false).unwrap();
    fs::write(dirs.desktop_entries.join("example.desktop"), "[Desktop Entry]\nName=Foreign\n").unwrap();
    assert!(matches!(integrate(&second, &dirs), Err(Error::DesktopEntryConflict(_))));
    assert!(matches!(remove("example", &dirs, false), Err(Error::NotIntegrated(_))));
    assert!(dirs.desktop_entries.join("example.desktop").exists());
}

#[test]
fn refuses_to_overwrite_an_existing_file_in_the_appimage_dir() {
    let dirs = dirs("destination-dirs");
    let source = example("destination.AppImage");
    fs::create_dir_all(&dirs.appimages).unwrap();
    fs::write(dirs.appimages.join(source.file_name().unwrap()), "other").unwrap();
    assert!(matches!(integrate(&source, &dirs), Err(Error::DestinationExists(_))));
    assert!(source.exists());
}

#[test]
fn replace_takes_the_place_of_an_integrated_version() {
    let dirs = dirs("replace-dirs");
    let old = integrate(&example("replace-old.AppImage"), &dirs).unwrap();

    let source = example_version("replace-new.AppImage", "2.0.0");
    let new = replace(&source, &dirs).unwrap();
    assert_eq!(new.id, old.id);
    assert_eq!(new.version.as_deref(), Some("2.0.0"));
    assert!(new.appimage.ends_with("Applications/replace-new.AppImage"));
    assert!(new.appimage.exists());
    assert!(!source.exists());
    assert!(!old.appimage.exists());
    assert!(new.icon.unwrap().exists());

    let listed = list(&dirs).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].appimage, new.appimage);
}

#[test]
fn replace_overwrites_an_integrated_file_of_the_same_name() {
    let dirs = dirs("replace-same-name-dirs");
    let old = integrate(&example("replace-same-name.AppImage"), &dirs).unwrap();

    let source = example_version("replace-same-name.AppImage", "2.0.0");
    let size = fs::metadata(&source).unwrap().len();
    let new = replace(&source, &dirs).unwrap();
    assert_eq!(new.appimage, old.appimage);
    assert_eq!(new.version.as_deref(), Some("2.0.0"));
    assert_eq!(fs::metadata(&new.appimage).unwrap().len(), size);
    assert!(!source.exists());
}

#[test]
fn replace_integrates_when_nothing_is_integrated_yet() {
    let dirs = dirs("replace-fresh-dirs");
    let integrated = replace(&example("replace-fresh.AppImage"), &dirs).unwrap();
    assert!(integrated.appimage.exists());
    assert_eq!(list(&dirs).unwrap().len(), 1);
}

#[test]
fn replace_leaves_unrelated_files_and_foreign_entries_alone() {
    let dirs = dirs("replace-conflict-dirs");
    let old = integrate(&example("replace-conflict-old.AppImage"), &dirs).unwrap();
    let source = example_version("replace-conflict-new.AppImage", "2.0.0");
    let occupied = dirs.appimages.join("replace-conflict-new.AppImage");
    fs::write(&occupied, "other").unwrap();
    assert!(matches!(replace(&source, &dirs), Err(Error::DestinationExists(_))));
    assert!(source.exists());
    assert!(old.appimage.exists());
    assert_eq!(fs::read_to_string(&occupied).unwrap(), "other");

    remove("example", &dirs, false).unwrap();
    fs::write(dirs.desktop_entries.join("example.desktop"), "[Desktop Entry]\nName=Foreign\n").unwrap();
    assert!(matches!(replace(&source, &dirs), Err(Error::DesktopEntryConflict(_))));
}

#[test]
fn integrating_the_integrated_path_again_refreshes_the_entry() {
    let dirs = dirs("refresh-dirs");
    let old = integrate(&example("refresh.AppImage"), &dirs).unwrap();
    fs::rename(example_version("refresh-rebuilt.AppImage", "2.0.0"), &old.appimage).unwrap();

    let new = integrate(&old.appimage, &dirs).unwrap();
    assert_eq!(new.appimage, old.appimage);
    assert_eq!(new.version.as_deref(), Some("2.0.0"));
    assert!(new.appimage.exists());
}

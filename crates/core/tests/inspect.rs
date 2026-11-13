mod common;

use std::path::Path;

use appimage_integrator_core::{AppImageType, Error, appimage_type, inspect};
use backhand::NodeHeader;
use common::{DESKTOP_ENTRY, PNG, appimage, appimage_bytes, push_file, runtime, write_fixture};

#[test]
fn detects_types() {
    let type1 = write_fixture("type1", &runtime(b"AI\x01", ""));
    let type2 = write_fixture("type2", &runtime(b"AI\x02", ""));
    let plain_elf = write_fixture("elf", &runtime(b"\0\0\0", ""));
    let short = write_fixture("short", b"\x7fELF");
    assert_eq!(appimage_type(&type1).unwrap(), Some(AppImageType::Type1));
    assert_eq!(appimage_type(&type2).unwrap(), Some(AppImageType::Type2));
    assert_eq!(appimage_type(&plain_elf).unwrap(), None);
    assert_eq!(appimage_type(&short).unwrap(), None);
}

#[test]
fn reads_embedded_update_information() {
    let with_info = appimage_bytes("gh-releases-zsync|owner|repo|latest|App-*.AppImage.zsync", |squashfs| {
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
    });
    let info = inspect(&write_fixture("update-info", &with_info)).unwrap();
    assert_eq!(info.update_info.as_deref(), Some("gh-releases-zsync|owner|repo|latest|App-*.AppImage.zsync"));

    let without_info = appimage("no-update-info", |squashfs| {
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
    });
    assert_eq!(inspect(&without_info).unwrap().update_info, None);
}

#[test]
fn reads_desktop_entry_and_named_icon() {
    let path = appimage("named-icon", |squashfs| {
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
        push_file(squashfs, "example.png", PNG);
    });
    let info = inspect(&path).unwrap();
    assert_eq!(info.desktop_file, "example.desktop");
    assert_eq!(info.name(), Some("Example App"));
    assert_eq!(info.version(), Some("1.2.3"));
    let icon = info.icon.unwrap();
    assert_eq!(icon.path, Path::new("/example.png"));
    assert_eq!(icon.extension, "png");
    assert_eq!(icon.data, PNG);
}

#[test]
fn follows_symlinks_for_desktop_entry_and_dir_icon() {
    let path = appimage("symlinks", |squashfs| {
        let header = NodeHeader::default();
        squashfs.push_dir_all("usr/share/icons", header).unwrap();
        push_file(squashfs, "usr/share/example.desktop", DESKTOP_ENTRY.as_bytes());
        push_file(squashfs, "usr/share/icons/app.svg", b"<svg/>");
        squashfs.push_symlink("usr/share/example.desktop", "example.desktop", header).unwrap();
        squashfs.push_symlink("usr/share/icons/../icons/app.svg", ".DirIcon", header).unwrap();
    });
    let info = inspect(&path).unwrap();
    assert_eq!(info.name(), Some("Example App"));
    let icon = info.icon.unwrap();
    assert_eq!(icon.path, Path::new("/usr/share/icons/app.svg"));
    assert_eq!(icon.extension, "svg");
}

#[test]
fn prefers_scalable_then_largest_hicolor_icon() {
    let header = NodeHeader::default();
    let with_png_sizes = appimage("hicolor-png", |squashfs| {
        squashfs.push_dir_all("usr/share/icons/hicolor/64x64/apps", header).unwrap();
        squashfs.push_dir_all("usr/share/icons/hicolor/256x256/apps", header).unwrap();
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
        push_file(squashfs, "example.png", PNG);
        push_file(squashfs, "usr/share/icons/hicolor/64x64/apps/example.png", PNG);
        push_file(squashfs, "usr/share/icons/hicolor/256x256/apps/example.png", PNG);
        push_file(squashfs, "usr/share/icons/hicolor/256x256/apps/other.png", PNG);
    });
    let icon = inspect(&with_png_sizes).unwrap().icon.unwrap();
    assert_eq!(icon.path, Path::new("/usr/share/icons/hicolor/256x256/apps/example.png"));

    let with_scalable = appimage("hicolor-svg", |squashfs| {
        squashfs.push_dir_all("usr/share/icons/hicolor/512x512/apps", header).unwrap();
        squashfs.push_dir_all("usr/share/icons/hicolor/scalable/apps", header).unwrap();
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
        push_file(squashfs, "usr/share/icons/hicolor/512x512/apps/example.png", PNG);
        push_file(squashfs, "usr/share/icons/hicolor/scalable/apps/example.svg", b"<svg/>");
    });
    let icon = inspect(&with_scalable).unwrap().icon.unwrap();
    assert_eq!(icon.path, Path::new("/usr/share/icons/hicolor/scalable/apps/example.svg"));
}

#[test]
fn reports_missing_icon_and_missing_desktop_entry() {
    let no_icon = appimage("no-icon", |squashfs| {
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
    });
    assert!(inspect(&no_icon).unwrap().icon.is_none());

    let no_desktop = appimage("no-desktop", |squashfs| {
        push_file(squashfs, "readme.txt", b"hello");
    });
    assert!(matches!(inspect(&no_desktop), Err(Error::MissingDesktopEntry)));
}

#[test]
fn rejects_symlink_loops() {
    let path = appimage("loop", |squashfs| {
        let header = NodeHeader::default();
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
        squashfs.push_symlink("b", ".DirIcon", header).unwrap();
        squashfs.push_symlink(".DirIcon", "b", header).unwrap();
    });
    assert!(inspect(&path).unwrap().icon.is_none());
}

#[test]
fn rejects_unsupported_inputs() {
    let type1 = write_fixture("inspect-type1", &runtime(b"AI\x01", ""));
    let no_payload = write_fixture("inspect-no-payload", &runtime(b"AI\x02", ""));
    let plain_elf = write_fixture("inspect-elf", &runtime(b"\0\0\0", ""));
    assert!(matches!(inspect(&type1), Err(Error::UnsupportedType1)));
    assert!(matches!(inspect(&no_payload), Err(Error::UnsupportedPayload)));
    assert!(matches!(inspect(&plain_elf), Err(Error::NotAppImage)));
}

#![allow(dead_code)]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use appimage_integrator_core::Dirs;
use backhand::{FilesystemWriter, NodeHeader};

pub const DESKTOP_ENTRY: &str = "\
[Desktop Entry]
Type=Application
Name=Example App
Icon=example
Exec=example %U
X-AppImage-Version=1.2.3
";
pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

pub fn runtime(magic: &[u8; 3], update_info: &str) -> Vec<u8> {
    let names = b"\0.shstrtab\0.upd_info\0";
    let mut info = [0u8; 1024];
    info[..update_info.len()].copy_from_slice(update_info.as_bytes());
    let info_offset = 64 + names.len();
    let table_offset = info_offset + info.len();

    let mut image = vec![0u8; 64];
    image[..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[8..11].copy_from_slice(magic);
    image[0x28..0x30].copy_from_slice(&(table_offset as u64).to_le_bytes());
    image[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes());
    image[0x3C..0x3E].copy_from_slice(&3u16.to_le_bytes());
    image[0x3E..0x40].copy_from_slice(&1u16.to_le_bytes());
    image.extend_from_slice(names);
    image.extend_from_slice(&info);

    let section = |name: u32, offset: usize, size: usize| {
        let mut entry = [0u8; 64];
        entry[..4].copy_from_slice(&name.to_le_bytes());
        entry[0x18..0x20].copy_from_slice(&(offset as u64).to_le_bytes());
        entry[0x20..0x28].copy_from_slice(&(size as u64).to_le_bytes());
        entry
    };
    image.extend_from_slice(&section(0, 0, 0));
    image.extend_from_slice(&section(1, 64, names.len()));
    image.extend_from_slice(&section(11, info_offset, info.len()));
    image
}

pub fn temp_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

pub fn write_fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let path = temp_path(name);
    fs::write(&path, bytes).unwrap();
    path
}

pub fn appimage(name: &str, build: impl FnOnce(&mut FilesystemWriter)) -> PathBuf {
    write_fixture(name, &appimage_bytes("", build))
}

pub fn appimage_bytes(update_info: &str, build: impl FnOnce(&mut FilesystemWriter)) -> Vec<u8> {
    let mut squashfs = FilesystemWriter::default();
    build(&mut squashfs);
    let runtime = runtime(b"AI\x02", update_info);
    let offset = runtime.len() as u64;
    let mut image = Cursor::new(runtime);
    squashfs.write_with_offset(&mut image, offset).unwrap();
    image.into_inner()
}

pub fn dirs(name: &str) -> Dirs {
    let root = temp_path(name);
    let _ = fs::remove_dir_all(&root);
    Dirs {
        appimages: root.join("Applications"),
        watched: vec![root.join("Applications")],
        desktop_entries: root.join("share/applications"),
        icons: root.join("share/appimage-integrator/icons"),
    }
}

pub fn example(name: &str) -> PathBuf {
    appimage(name, |squashfs| {
        push_file(squashfs, "example.desktop", DESKTOP_ENTRY.as_bytes());
        push_file(squashfs, "example.png", PNG);
    })
}

pub fn push_file(squashfs: &mut FilesystemWriter, path: &str, data: &[u8]) {
    squashfs.push_file(Cursor::new(data.to_vec()), path, NodeHeader::default()).unwrap();
}

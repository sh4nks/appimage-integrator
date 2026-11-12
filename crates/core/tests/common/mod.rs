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

pub fn elf_header(magic: &[u8; 3]) -> Vec<u8> {
    let mut header = vec![0u8; 64];
    header[..4].copy_from_slice(b"\x7fELF");
    header[4] = 2;
    header[5] = 1;
    header[8..11].copy_from_slice(magic);
    header[0x28..0x30].copy_from_slice(&64u64.to_le_bytes());
    header
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
    let mut squashfs = FilesystemWriter::default();
    build(&mut squashfs);
    let mut image = Cursor::new(elf_header(b"AI\x02"));
    squashfs.write_with_offset(&mut image, 64).unwrap();
    write_fixture(name, image.get_ref())
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

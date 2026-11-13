#![cfg(feature = "update")]

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::thread;

use appimage_integrator_core::update::{apply, check};
use appimage_integrator_core::{Error, Integrated, integrate, list};
use common::{PNG, appimage_bytes, dirs, push_file, write_fixture};
use zsync_rs::ControlFile;

fn version_bytes(version: &str, update_info: &str) -> Vec<u8> {
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Example App\nIcon=example\nExec=example\nX-AppImage-Version={version}\n"
    );
    let shared: Vec<u8> = (0..200_000u32).flat_map(|index| index.wrapping_mul(2_654_435_761).to_le_bytes()).collect();
    appimage_bytes(update_info, |squashfs| {
        push_file(squashfs, "example.desktop", entry.as_bytes());
        push_file(squashfs, "example.png", PNG);
        push_file(squashfs, "data.bin", &shared);
        push_file(squashfs, "version.txt", version.as_bytes());
    })
}

fn control_file(target: &[u8], filename: &str) -> Vec<u8> {
    let control = ControlFile::generate(&mut Cursor::new(target), filename, filename, None).unwrap();
    let mut bytes = Vec::new();
    control.write(&mut bytes).unwrap();
    bytes
}

fn serve(listener: TcpListener, files: HashMap<String, Vec<u8>>) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let path = request.split(' ').nth(1).unwrap_or("").to_string();
            let mut range = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(spec) = line.to_ascii_lowercase().trim().strip_prefix("range: bytes=") {
                    let (start, end) = spec.split_once('-').unwrap();
                    range = Some((start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap()));
                }
            }
            let response = match (files.get(&path), range) {
                (None, _) => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                (Some(body), None) => {
                    let mut response =
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                            .into_bytes();
                    response.extend_from_slice(body);
                    response
                }
                (Some(body), Some((start, end))) => {
                    let end = end.min(body.len() - 1);
                    let mut response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",
                        end - start + 1,
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&body[start..=end]);
                    response
                }
            };
            let _ = stream.write_all(&response);
        }
    });
}

struct Scenario {
    dirs: appimage_integrator_core::Dirs,
    integrated: Integrated,
    new_bytes: Vec<u8>,
}

fn scenario(name: &str, old_filename: &str, new_filename: &str) -> Scenario {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let update_info = format!("zsync|http://127.0.0.1:{port}/latest/App.AppImage.zsync");
    let new_bytes = version_bytes("2.0", &update_info);
    serve(
        listener,
        HashMap::from([
            ("/latest/App.AppImage.zsync".to_string(), control_file(&new_bytes, new_filename)),
            (format!("/latest/{new_filename}"), new_bytes.clone()),
        ]),
    );

    let dirs = dirs(name);
    fs::create_dir_all(&dirs.appimages).unwrap();
    let old = dirs.appimages.join(old_filename);
    fs::write(&old, version_bytes("1.0", &update_info)).unwrap();
    let integrated = integrate(&old, &dirs).unwrap();
    Scenario { dirs, integrated, new_bytes }
}

#[test]
fn updates_to_a_new_file_and_removes_the_old_one() {
    let Scenario { dirs, integrated, new_bytes } =
        scenario("update-new-file-dirs", "App-1.0.AppImage", "App-2.0.AppImage");
    assert_eq!(integrated.version.as_deref(), Some("1.0"));

    let update = check(&integrated).unwrap().unwrap();
    assert_eq!(update.filename, "App-2.0.AppImage");
    assert_eq!(update.size, new_bytes.len() as u64);

    let updated = apply(&integrated, update, &dirs, |_, _| {}).unwrap();
    assert_eq!(updated.id, integrated.id);
    assert_eq!(updated.version.as_deref(), Some("2.0"));
    assert!(updated.appimage.ends_with("App-2.0.AppImage"));
    assert_eq!(fs::read(&updated.appimage).unwrap(), new_bytes);
    assert_eq!(fs::metadata(&updated.appimage).unwrap().permissions().mode() & 0o111, 0o111);
    assert!(!integrated.appimage.exists());

    let listed = list(&dirs).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].appimage, updated.appimage);
    let entry = fs::read_to_string(&updated.desktop_file).unwrap();
    assert!(entry.contains(&format!("Exec=\"{}\"\n", updated.appimage.display())));

    assert!(check(&updated).unwrap().is_none());
    let leftovers: Vec<_> = fs::read_dir(&dirs.appimages).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(leftovers, ["App-2.0.AppImage"]);
}

#[test]
fn updates_in_place_when_the_filename_is_unchanged() {
    let Scenario { dirs, integrated, new_bytes } = scenario("update-in-place-dirs", "App.AppImage", "App.AppImage");
    let update = check(&integrated).unwrap().unwrap();
    let updated = apply(&integrated, update, &dirs, |_, _| {}).unwrap();
    assert_eq!(updated.appimage, integrated.appimage);
    assert_eq!(updated.version.as_deref(), Some("2.0"));
    assert_eq!(fs::read(&updated.appimage).unwrap(), new_bytes);
    assert_eq!(fs::metadata(&updated.appimage).unwrap().permissions().mode() & 0o111, 0o111);
}

#[test]
fn refuses_to_overwrite_an_unrelated_file() {
    let Scenario { dirs, integrated, .. } = scenario("update-collision-dirs", "App-1.0.AppImage", "App-2.0.AppImage");
    fs::write(dirs.appimages.join("App-2.0.AppImage"), "something else").unwrap();
    let update = check(&integrated).unwrap().unwrap();
    assert!(matches!(apply(&integrated, update, &dirs, |_, _| {}), Err(Error::DestinationExists(_))));
    assert!(integrated.appimage.exists());
}

#[test]
fn reports_missing_unsupported_and_insecure_update_information() {
    let dirs = dirs("update-info-dirs");
    let integrate_with = |filename: &str, id: &str, update_info: &str| {
        let entry = format!("[Desktop Entry]\nName={id}\nExec={id}\n");
        let bytes = appimage_bytes(update_info, |squashfs| {
            push_file(squashfs, &format!("{id}.desktop"), entry.as_bytes());
        });
        integrate(&write_fixture(filename, &bytes), &dirs).unwrap()
    };

    let none = integrate_with("update-none.AppImage", "none", "");
    assert!(matches!(check(&none), Err(Error::NoUpdateInfo)));

    let guess = integrate_with("update-guess.AppImage", "guess", "guess");
    assert!(matches!(check(&guess), Err(Error::UnsupportedUpdateInfo(info)) if info == "guess"));

    let insecure = integrate_with("update-http.AppImage", "insecure", "zsync|http://example.invalid/App.zsync");
    assert!(matches!(check(&insecure), Err(Error::Update(_))));
}

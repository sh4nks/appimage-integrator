#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, apps)]
        #[qproperty(QString, pending)]
        #[qproperty(QString, status)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, updatable)]
        type Backend = super::BackendRust;

        #[qinvokable]
        fn start(self: Pin<&mut Self>);

        #[qinvokable]
        fn refresh(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "removeApp"]
        fn remove_app(self: Pin<&mut Self>, id: &QString, keep_file: bool);

        #[qinvokable]
        #[cxx_name = "launchApp"]
        fn launch_app(self: Pin<&mut Self>, id: &QString);

        #[qinvokable]
        #[cxx_name = "integratePending"]
        fn integrate_pending(self: Pin<&mut Self>, run: bool) -> bool;

        #[qinvokable]
        #[cxx_name = "runPending"]
        fn run_pending(self: Pin<&mut Self>) -> bool;

        #[qinvokable]
        #[cxx_name = "checkUpdates"]
        fn check_updates(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "updateApp"]
        fn update_app(self: Pin<&mut Self>, id: &QString);
    }

    impl cxx_qt::Threading for Backend {}
}

use core::pin::Pin;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use appimage_integrator_core::{Dirs, Integrated, inspect, integrate, list, remove};
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use serde_json::json;

#[derive(Clone, Default)]
struct UpdateState {
    text: String,
    available: bool,
}

#[derive(Default)]
pub struct BackendRust {
    apps: QString,
    pending: QString,
    status: QString,
    busy: bool,
    updatable: bool,
    dirs: Option<Dirs>,
    pending_file: Option<PathBuf>,
    update_states: HashMap<String, UpdateState>,
}

impl qobject::Backend {
    pub fn start(mut self: Pin<&mut Self>) {
        self.as_mut().set_updatable(cfg!(feature = "update"));
        match Dirs::load() {
            Ok(dirs) => self.as_mut().rust_mut().dirs = Some(dirs),
            Err(error) => return self.show_status(error),
        }
        let file = std::env::args_os().skip(1).find(|argument| !argument.to_string_lossy().starts_with('-'));
        if let Some(file) = file {
            self.as_mut().open_file(PathBuf::from(file));
        }
        self.refresh();
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        let Some(items) = self.as_mut().items() else {
            return;
        };
        let states = &self.rust().update_states;
        let apps: Vec<serde_json::Value> = items
            .iter()
            .map(|item| {
                let state = states.get(&item.id).cloned().unwrap_or_default();
                json!({
                    "id": item.id,
                    "name": item.name.as_deref().unwrap_or(&item.id),
                    "version": item.version.as_deref().unwrap_or(""),
                    "path": item.appimage.to_string_lossy(),
                    "icon": item.icon.as_deref().map(file_url).unwrap_or_default(),
                    "missing": !item.appimage.exists(),
                    "updateText": state.text,
                    "updateAvailable": state.available,
                })
            })
            .collect();
        self.set_apps(QString::from(&serde_json::Value::from(apps).to_string()));
    }

    pub fn remove_app(mut self: Pin<&mut Self>, id: &QString, keep_file: bool) {
        let id = id.to_string();
        let Some(dirs) = self.rust().dirs.clone() else {
            return;
        };
        match remove(&id, &dirs, keep_file) {
            Ok(removed) => {
                self.as_mut().rust_mut().update_states.remove(&id);
                self.as_mut().show_status(format!("Removed {}", removed.name.as_deref().unwrap_or(&id)));
            }
            Err(error) => self.as_mut().show_status(error),
        }
        self.refresh();
    }

    pub fn launch_app(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        let Some(item) = self.as_mut().items().and_then(|items| items.into_iter().find(|item| item.id == id))
        else {
            return;
        };
        if let Err(error) = run_appimage(&item.appimage) {
            self.show_status(format!("Cannot start {}: {error}", item.appimage.display()));
        }
    }

    pub fn integrate_pending(mut self: Pin<&mut Self>, run: bool) -> bool {
        let (Some(file), Some(dirs)) = (self.rust().pending_file.clone(), self.rust().dirs.clone()) else {
            return false;
        };
        let integrated = match integrate(&file, &dirs) {
            Ok(integrated) => integrated,
            Err(error) => {
                self.show_status(error);
                return false;
            }
        };
        if run && let Err(error) = run_appimage(&integrated.appimage) {
            self.as_mut().show_status(format!("Integrated, but cannot start it: {error}"));
        } else {
            self.as_mut().show_status(format!("Integrated {}", integrated.name.as_deref().unwrap_or(&integrated.id)));
        }
        self.as_mut().rust_mut().pending_file = None;
        self.as_mut().set_pending(QString::default());
        self.refresh();
        true
    }

    pub fn run_pending(self: Pin<&mut Self>) -> bool {
        let Some(file) = self.rust().pending_file.clone() else {
            return false;
        };
        match run_appimage(&file) {
            Ok(()) => true,
            Err(error) => {
                self.show_status(format!("Cannot start {}: {error}", file.display()));
                false
            }
        }
    }

    #[cfg(not(feature = "update"))]
    pub fn check_updates(self: Pin<&mut Self>) {}

    #[cfg(not(feature = "update"))]
    pub fn update_app(self: Pin<&mut Self>, _id: &QString) {}

    fn open_file(mut self: Pin<&mut Self>, file: PathBuf) {
        let file = fs::canonicalize(&file).unwrap_or(file);
        let integrated = self.as_mut().items().unwrap_or_default().into_iter().any(|item| item.appimage == file);
        if integrated {
            match run_appimage(&file) {
                Ok(()) => std::process::exit(0),
                Err(error) => return self.show_status(format!("Cannot start {}: {error}", file.display())),
            }
        }
        let filename = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let pending = match inspect(&file) {
            Ok(info) => json!({
                "file": file.to_string_lossy(),
                "name": info.name().unwrap_or(&filename),
                "version": info.version().unwrap_or(""),
                "icon": info.icon.as_ref().map(|icon| data_url(icon.extension, &icon.data)).unwrap_or_default(),
                "error": "",
            }),
            Err(error) => json!({
                "file": file.to_string_lossy(),
                "name": filename,
                "version": "",
                "icon": "",
                "error": error.to_string(),
            }),
        };
        self.as_mut().rust_mut().pending_file = Some(file);
        self.set_pending(QString::from(&pending.to_string()));
    }

    fn items(self: Pin<&mut Self>) -> Option<Vec<Integrated>> {
        let dirs = self.rust().dirs.clone()?;
        match list(&dirs) {
            Ok(items) => Some(items),
            Err(error) => {
                self.show_status(error);
                None
            }
        }
    }

    fn show_status(self: Pin<&mut Self>, message: impl ToString) {
        self.set_status(QString::from(&message.to_string()));
    }
}

#[cfg(feature = "update")]
mod update {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use appimage_integrator_core::update::{apply, check};
    use appimage_integrator_core::{Dirs, Error, Integrated};
    use cxx_qt::{CxxQtType, Threading};
    use cxx_qt_lib::QString;

    use super::{UpdateState, qobject};

    impl qobject::Backend {
        pub fn check_updates(mut self: Pin<&mut Self>) {
            let Some(items) = self.as_mut().items() else {
                return;
            };
            for item in &items {
                self.as_mut().set_update_state(&item.id, "Checking for updates...", false);
            }
            self.as_mut().set_busy(true);
            self.as_mut().refresh();

            let qt_thread = self.qt_thread();
            thread::spawn(move || {
                for item in items {
                    let (text, available) = match check(&item) {
                        Ok(Some(update)) => (
                            format!("Update available: {} ({:.1} MB)", update.filename, update.size as f64 / 1e6),
                            true,
                        ),
                        Ok(None) => ("Up to date".to_string(), false),
                        Err(Error::NoUpdateInfo | Error::UnsupportedUpdateInfo(_)) => {
                            ("No update source".to_string(), false)
                        }
                        Err(error) => (format!("Update check failed: {error}"), false),
                    };
                    let _ = qt_thread.queue(move |mut backend| {
                        backend.as_mut().set_update_state(&item.id, &text, available);
                        backend.refresh();
                    });
                }
                let _ = qt_thread.queue(|backend| backend.set_busy(false));
            });
        }

        pub fn update_app(mut self: Pin<&mut Self>, id: &QString) {
            let id = id.to_string();
            let Some(dirs) = self.rust().dirs.clone() else {
                return;
            };
            let Some(item) = self.as_mut().items().and_then(|items| items.into_iter().find(|item| item.id == id))
            else {
                return;
            };
            self.as_mut().set_update_state(&id, "Updating...", false);
            self.as_mut().set_busy(true);
            self.as_mut().refresh();

            let qt_thread = self.qt_thread();
            thread::spawn(move || {
                let progress_thread = qt_thread.clone();
                let name = item.name.clone().unwrap_or_else(|| item.id.clone());
                let last_percent = AtomicU64::new(u64::MAX);
                let result = download(&item, &dirs, move |done, total| {
                    let percent = done * 100 / total.max(1);
                    if last_percent.swap(percent, Ordering::Relaxed) != percent {
                        let message = format!("Downloading {name}: {percent}%");
                        let _ = progress_thread.queue(move |backend| backend.show_status(message));
                    }
                });
                let _ = qt_thread.queue(move |mut backend| {
                    backend.as_mut().rust_mut().update_states.remove(&item.id);
                    match result {
                        Ok(Some(updated)) => {
                            let version = updated.version.as_deref().unwrap_or("a new version");
                            backend.as_mut().set_update_state(&updated.id, &format!("Updated to {version}"), false);
                            backend.as_mut().show_status("");
                        }
                        Ok(None) => backend.as_mut().set_update_state(&item.id, "Up to date", false),
                        Err(error) => backend.as_mut().show_status(format!("Update failed: {error}")),
                    }
                    backend.as_mut().set_busy(false);
                    backend.refresh();
                });
            });
        }

        fn set_update_state(self: Pin<&mut Self>, id: &str, text: &str, available: bool) {
            let state = UpdateState { text: text.to_string(), available };
            self.rust_mut().update_states.insert(id.to_string(), state);
        }
    }

    fn download(
        item: &Integrated,
        dirs: &Dirs,
        progress: impl Fn(u64, u64) + Send + Sync + 'static,
    ) -> Result<Option<Integrated>, Error> {
        match check(item)? {
            Some(update) => apply(item, update, dirs, progress).map(Some),
            None => Ok(None),
        }
    }
}

fn run_appimage(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    if permissions.mode() & 0o100 == 0 {
        permissions.set_mode(permissions.mode() | 0o100);
        fs::set_permissions(path, permissions)?;
    }
    let mut child = Command::new(path).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for character in path.to_string_lossy().chars() {
        match character {
            ' ' | '%' | '#' | '?' => url.push_str(&format!("%{:02X}", character as u32)),
            _ => url.push(character),
        }
    }
    url
}

fn data_url(extension: &str, data: &[u8]) -> String {
    let mime = match extension {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => "image/x-xpixmap",
    };
    format!("data:{mime};base64,{}", base64(data))
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let bytes = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let group = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                encoded.push(ALPHABET[(group >> (18 - 6 * index)) as usize & 63] as char);
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{base64, file_url};
    use std::path::Path;

    #[test]
    fn encodes_base64_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn escapes_file_urls() {
        assert_eq!(file_url(Path::new("/icons/GitHub Copilot.png")), "file:///icons/GitHub%20Copilot.png");
        assert_eq!(file_url(Path::new("/icons/50%#1?.png")), "file:///icons/50%25%231%3F.png");
    }
}

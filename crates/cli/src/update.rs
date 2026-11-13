use std::io::IsTerminal;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use appimage_integrator_core::update::{apply, check};
use appimage_integrator_core::{Dirs, Error, Integrated, list};

enum Outcome {
    UpToDate,
    Available(String),
    Updated(Integrated),
}

pub fn command(id: Option<&str>, check_only: bool, notify: bool, dirs: &Dirs) -> Result<(), Error> {
    let mut items = list(dirs)?;
    if let Some(id) = id {
        items.retain(|item| item.id == id);
        if items.is_empty() {
            return Err(Error::NotIntegrated(id.to_string()));
        }
    }

    let mut failures = 0;
    let mut available = Vec::new();
    for item in &items {
        match update_one(item, check_only, dirs) {
            Ok(Outcome::UpToDate) => println!("{}: up to date", item.id),
            Ok(Outcome::Available(description)) => {
                println!("{}: update available: {description}", item.id);
                available.push(item.name.clone().unwrap_or_else(|| item.id.clone()));
            }
            Ok(Outcome::Updated(updated)) => println!(
                "{}: updated to {} ({})",
                item.id,
                updated.version.as_deref().unwrap_or("a new version"),
                updated.appimage.display()
            ),
            Err(error @ (Error::NoUpdateInfo | Error::UnsupportedUpdateInfo(_))) => {
                println!("{}: {error}", item.id)
            }
            Err(error) => {
                eprintln!("{}: {error}", item.id);
                failures += 1;
            }
        }
    }
    if notify && !available.is_empty() {
        send_notification(&available)?;
    }
    if failures > 0 {
        return Err(Error::Update(format!("{failures} of {} failed", items.len())));
    }
    Ok(())
}

fn update_one(item: &Integrated, check_only: bool, dirs: &Dirs) -> Result<Outcome, Error> {
    let Some(update) = check(item)? else {
        return Ok(Outcome::UpToDate);
    };
    let megabytes = update.size as f64 / 1e6;
    if check_only {
        return Ok(Outcome::Available(format!("{} ({megabytes:.1} MB)", update.filename)));
    }

    let show_progress = std::io::stderr().is_terminal();
    let last_percent = AtomicU64::new(u64::MAX);
    let id = item.id.clone();
    let updated = apply(item, update, dirs, move |done, total| {
        let percent = done * 100 / total.max(1);
        if show_progress && last_percent.swap(percent, Ordering::Relaxed) != percent {
            eprint!("\r{id}: downloading {percent}% of {megabytes:.1} MB");
        }
    })?;
    if show_progress {
        eprintln!();
    }
    Ok(Outcome::Updated(updated))
}

fn notification_summary(count: usize) -> String {
    if count == 1 {
        "1 AppImage update available".to_string()
    } else {
        format!("{count} AppImage updates available")
    }
}

fn send_notification(names: &[String]) -> Result<(), Error> {
    let status = Command::new("notify-send")
        .arg("--app-name=AppImage Integrator")
        .arg("--icon=application-vnd.appimage")
        .arg("--hint=string:desktop-entry:appimage-integrator-gui")
        .arg(notification_summary(names.len()))
        .arg(names.join(", "))
        .status()
        .map_err(|error| Error::Update(format!("cannot run notify-send: {error}")))?;
    if !status.success() {
        return Err(Error::Update(format!("notify-send failed: {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::notification_summary;

    #[test]
    fn pluralises_the_notification_summary() {
        assert_eq!(notification_summary(1), "1 AppImage update available");
        assert_eq!(notification_summary(3), "3 AppImage updates available");
    }
}

use std::fs::{self, File};
use std::path::Path;

use serde::Deserialize;
use zsync_rs::checksum::calc_sha1_stream;
use zsync_rs::http::{CONNECT_TIMEOUT, RESOLVE_TIMEOUT};
use zsync_rs::{ControlFile, HttpClient, ZsyncAssembly};

use crate::integrate::replace_integration;
use crate::{Dirs, Error, Integrated, inspect};

const GITHUB_API: &str = "https://api.github.com";

pub struct Update {
    pub filename: String,
    pub size: u64,
    control: ControlFile,
    base_url: String,
    client: HttpClient,
}

#[derive(Debug, PartialEq)]
enum Source {
    Zsync(String),
    GithubReleases { owner: String, repo: String, tag: String, pattern: String },
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn check(item: &Integrated) -> Result<Option<Update>, Error> {
    let info = inspect(&item.appimage)?.update_info.ok_or(Error::NoUpdateInfo)?;
    let source = parse_source(&info)?;
    let plain_http_allowed = matches!(&source, Source::Zsync(url) if is_loopback(url));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(!plain_http_allowed)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_resolve(Some(RESOLVE_TIMEOUT))
        .build()
        .into();

    let control_url = control_url(&source, &agent)?;
    let client = HttpClient::with_agent(agent);
    let control = client.fetch_control_file(&control_url).map_err(failed)?;
    let remote_sha1 = control
        .sha1
        .clone()
        .ok_or_else(|| Error::Update("the zsync file has no SHA-1 checksum".to_string()))?;
    let local_sha1: String = calc_sha1_stream(&mut File::open(&item.appimage)?)?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if local_sha1.eq_ignore_ascii_case(&remote_sha1) {
        return Ok(None);
    }

    let current_name = item.appimage.file_name().unwrap();
    let filename = control
        .filename
        .as_deref()
        .and_then(|name| Path::new(name).file_name())
        .unwrap_or(current_name)
        .to_string_lossy()
        .into_owned();
    let base_url = control_url.split(['?', '#']).next().unwrap();
    let base_url = base_url[..=base_url.rfind('/').unwrap()].to_string();
    Ok(Some(Update { filename, size: control.length, control, base_url, client }))
}

pub fn apply(
    item: &Integrated,
    update: Update,
    dirs: &Dirs,
    progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> Result<Integrated, Error> {
    let target = item.appimage.with_file_name(&update.filename);
    let is_new_file = target != item.appimage;
    if is_new_file && target.exists() {
        return Err(Error::DestinationExists(target));
    }

    let mut assembly =
        ZsyncAssembly::with_client(update.control, &target, Some(&update.base_url), update.client)
            .map_err(failed)?;
    assembly.set_progress_callback(progress);
    assembly.submit_source_file(&item.appimage).map_err(failed)?;
    while assembly.download_missing_blocks().map_err(failed)? > 0 {}
    assembly.complete().map_err(failed)?;

    match replace_integration(item, &target, dirs) {
        Ok(integrated) => {
            if is_new_file {
                fs::remove_file(&item.appimage)?;
            }
            Ok(integrated)
        }
        Err(error) => {
            if is_new_file {
                let _ = fs::remove_file(&target);
            }
            Err(error)
        }
    }
}

fn parse_source(info: &str) -> Result<Source, Error> {
    let fields: Vec<&str> = info.split('|').collect();
    match fields[..] {
        ["zsync", url] => Ok(Source::Zsync(url.to_string())),
        ["gh-releases-zsync", owner, repo, tag, pattern]
            if [owner, repo, tag].into_iter().all(is_url_safe) =>
        {
            Ok(Source::GithubReleases {
                owner: owner.to_string(),
                repo: repo.to_string(),
                tag: tag.to_string(),
                pattern: pattern.to_string(),
            })
        }
        _ => Err(Error::UnsupportedUpdateInfo(info.to_string())),
    }
}

fn control_url(source: &Source, agent: &ureq::Agent) -> Result<String, Error> {
    let (owner, repo, tag, pattern) = match source {
        Source::Zsync(url) => return Ok(url.clone()),
        Source::GithubReleases { owner, repo, tag, pattern } => (owner, repo, tag, pattern),
    };
    let release = if tag == "latest" { "latest".to_string() } else { format!("tags/{tag}") };
    let release: Release = agent
        .get(format!("{GITHUB_API}/repos/{owner}/{repo}/releases/{release}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(failed)?
        .body_mut()
        .read_json()
        .map_err(failed)?;
    release
        .assets
        .into_iter()
        .find(|asset| glob_matches(pattern, &asset.name))
        .map(|asset| asset.browser_download_url)
        .ok_or_else(|| Error::Update(format!("no file in the release matches {pattern}")))
}

fn failed(error: impl std::fmt::Display) -> Error {
    Error::Update(error.to_string())
}

fn is_url_safe(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part.chars().all(|character| character.is_ascii_alphanumeric() || "._-+".contains(character))
}

fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap();
    let host = authority.rsplit_once(':').map_or(authority, |(host, _)| host);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn glob_matches(pattern: &str, name: &str) -> bool {
    let mut parts: Vec<&str> = pattern.split('*').collect();
    let Some(mut rest) = name.strip_prefix(parts.remove(0)) else {
        return false;
    };
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for part in parts {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::{Source, glob_matches, is_loopback, parse_source};

    #[test]
    fn parses_supported_update_information() {
        assert_eq!(
            parse_source("zsync|https://example.com/app.AppImage.zsync").unwrap(),
            Source::Zsync("https://example.com/app.AppImage.zsync".to_string())
        );
        assert_eq!(
            parse_source("gh-releases-zsync|owner|repo|latest|App-*x86_64.AppImage.zsync").unwrap(),
            Source::GithubReleases {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                tag: "latest".to_string(),
                pattern: "App-*x86_64.AppImage.zsync".to_string(),
            }
        );
    }

    #[test]
    fn rejects_unknown_or_unsafe_update_information() {
        assert!(parse_source("guess").is_err());
        assert!(parse_source("pling-v1-zsync|123|App-*.AppImage.zsync").is_err());
        assert!(parse_source("gh-releases-zsync|owner|repo|latest").is_err());
        assert!(parse_source("gh-releases-zsync|owner/../other|repo|latest|*.zsync").is_err());
        assert!(parse_source("gh-releases-zsync|owner|..|latest|*.zsync").is_err());
        assert!(parse_source("gh-releases-zsync|owner|repo|v1?x=1|*.zsync").is_err());
    }

    #[test]
    fn matches_release_file_patterns() {
        assert!(glob_matches("App-*x86_64.AppImage.zsync", "App-1.2.3-x86_64.AppImage.zsync"));
        assert!(glob_matches("App-*-*.zsync", "App-1.2-x86_64.zsync"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("exact.zsync", "exact.zsync"));
        assert!(!glob_matches("exact.zsync", "exact.zsync.sig"));
        assert!(!glob_matches("App-*x86_64.AppImage.zsync", "App-1.2.3-aarch64.AppImage.zsync"));
        assert!(!glob_matches("App-*x86_64.AppImage.zsync", "App-1.2.3-x86_64.AppImage"));
        assert!(!glob_matches("a*a", "a"));
    }

    #[test]
    fn allows_plain_http_for_loopback_only() {
        assert!(is_loopback("http://127.0.0.1:8080/app.zsync"));
        assert!(is_loopback("http://localhost/app.zsync"));
        assert!(is_loopback("http://[::1]:8080/app.zsync"));
        assert!(!is_loopback("http://example.com/app.zsync"));
        assert!(!is_loopback("http://127.0.0.1.example.com/app.zsync"));
        assert!(!is_loopback("https://127.0.0.1/app.zsync"));
    }
}

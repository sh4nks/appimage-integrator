use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone)]
pub struct Dirs {
    pub appimages: PathBuf,
    pub watched: Vec<PathBuf>,
    pub desktop_entries: PathBuf,
    pub icons: PathBuf,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    appimage_dir: Option<String>,
    watch_dirs: Option<Vec<String>>,
}

impl Dirs {
    pub fn load() -> Result<Dirs, Error> {
        let home = std::env::home_dir().ok_or(Error::NoHomeDir)?;
        let xdg_dir = |variable: &str, fallback: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        let data = xdg_dir("XDG_DATA_HOME", ".local/share");
        let config_file = xdg_dir("XDG_CONFIG_HOME", ".config").join("appimage-integrator/config.toml");
        let content = match fs::read_to_string(&config_file) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let (appimages, watched) =
            parse(&content, &home).map_err(|message| Error::Config(config_file, message))?;
        Ok(Dirs {
            appimages,
            watched,
            desktop_entries: data.join("applications"),
            icons: data.join("appimage-integrator/icons"),
        })
    }
}

fn parse(content: &str, home: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let config: ConfigFile = toml::from_str(content).map_err(|error| error.to_string())?;
    let appimages = match &config.appimage_dir {
        Some(dir) => expand(dir, home)?,
        None => home.join("Applications"),
    };
    let watched = match &config.watch_dirs {
        Some(dirs) => dirs.iter().map(|dir| expand(dir, home)).collect::<Result<_, _>>()?,
        None => vec![appimages.clone()],
    };
    Ok((appimages, watched))
}

fn expand(value: &str, home: &Path) -> Result<PathBuf, String> {
    if value == "~" {
        Ok(home.to_path_buf())
    } else if let Some(rest) = value.strip_prefix("~/") {
        Ok(home.join(rest))
    } else if Path::new(value).is_absolute() {
        Ok(PathBuf::from(value))
    } else {
        Err(format!("\"{value}\" must be an absolute path or start with ~/"))
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use std::path::{Path, PathBuf};

    const HOME: &str = "/home/user";

    fn parsed(content: &str) -> Result<(PathBuf, Vec<PathBuf>), String> {
        parse(content, Path::new(HOME))
    }

    #[test]
    fn defaults_to_applications_in_home() {
        let (appimages, watched) = parsed("").unwrap();
        assert_eq!(appimages, Path::new("/home/user/Applications"));
        assert_eq!(watched, [appimages]);
    }

    #[test]
    fn watches_the_appimage_dir_unless_watch_dirs_is_set() {
        let (appimages, watched) = parsed("appimage_dir = \"~/Apps\"").unwrap();
        assert_eq!(appimages, Path::new("/home/user/Apps"));
        assert_eq!(watched, [appimages]);

        let (appimages, watched) =
            parsed("appimage_dir = \"/opt/apps\"\nwatch_dirs = [\"~/Games/AppImages\", \"/srv/apps\", \"~\"]")
                .unwrap();
        assert_eq!(appimages, Path::new("/opt/apps"));
        assert_eq!(
            watched,
            [Path::new("/home/user/Games/AppImages"), Path::new("/srv/apps"), Path::new("/home/user")]
        );
    }

    #[test]
    fn rejects_relative_paths_unknown_keys_and_wrong_types() {
        assert!(parsed("appimage_dir = \"Apps\"").unwrap_err().contains("must be an absolute path"));
        assert!(parsed("watch_dirs = [\"~user/Apps\"]").is_err());
        assert!(parsed("watch_dir = \"~/Apps\"").unwrap_err().contains("unknown field"));
        assert!(parsed("watch_dirs = \"~/Apps\"").is_err());
    }
}

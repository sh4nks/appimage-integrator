pub fn value<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    let mut in_main_group = false;
    for line in entry.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
        } else if in_main_group
            && let Some((candidate, value)) = line.split_once('=')
            && candidate.trim_end() == key
        {
            return Some(value.trim_start());
        }
    }
    None
}

pub const APPIMAGE_KEY: &str = "X-AppImageIntegrator-Path";

pub fn rewrite(entry: &str, appimage: &str, icon: Option<&str>) -> String {
    let mut output = String::with_capacity(entry.len() + 3 * appimage.len());
    let mut in_main_group = false;
    for line in entry.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
            output.push_str(line);
            output.push('\n');
            if in_main_group {
                output.push_str(&format!("TryExec={}\n", escape(appimage)));
                output.push_str(&format!("{APPIMAGE_KEY}={}\n", escape(appimage)));
                if let Some(icon) = icon {
                    output.push_str(&format!("Icon={}\n", escape(icon)));
                }
            }
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((key, value)) if !line.starts_with('#') => (key.trim_end(), value),
            _ => ("", ""),
        };
        match key {
            "Exec" => {
                output.push_str(&format!("Exec={}{}\n", exec_quote(appimage), exec_arguments(value)))
            }
            "TryExec" | APPIMAGE_KEY if in_main_group => {}
            "Icon" if in_main_group && icon.is_some() => {}
            _ => {
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    output
}

pub fn unescape(value: &str) -> String {
    value.replace("\\\\", "\\")
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\")
}

// Exec arguments are quoted first and then escaped as a desktop entry string, hence the doubled backslashes.
fn exec_quote(path: &str) -> String {
    let mut quoted = String::from("\"");
    for character in path.chars() {
        match character {
            '"' | '`' | '$' => quoted.push_str("\\\\"),
            '\\' => quoted.push_str("\\\\\\"),
            '%' => quoted.push('%'),
            _ => {}
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

fn exec_arguments(value: &str) -> &str {
    let value = value.trim_start();
    let Some(quoted) = value.strip_prefix('"') else {
        return value.find(' ').map_or("", |index| &value[index..]);
    };
    let mut characters = quoted.char_indices().peekable();
    let mut escaped = false;
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => {
                characters.next_if(|&(_, next)| next == '\\');
                escaped = !escaped;
            }
            '"' if !escaped => return &quoted[index + 1..],
            _ => escaped = false,
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::{rewrite, value};

    #[test]
    fn rewrites_exec_tryexec_and_icon() {
        let entry = "\
[Desktop Entry]
Name=Example App
Exec=example --flag %U
TryExec=example
Icon=example
# Exec=commented

[Desktop Action New]
Exec=\"quoted \\\\\" app\" --new
Icon=action-icon
";
        let rewritten = rewrite(entry, "/apps/Example.AppImage", Some("/icons/example.png"));
        assert_eq!(
            rewritten,
            "\
[Desktop Entry]
TryExec=/apps/Example.AppImage
X-AppImageIntegrator-Path=/apps/Example.AppImage
Icon=/icons/example.png
Name=Example App
Exec=\"/apps/Example.AppImage\" --flag %U
# Exec=commented

[Desktop Action New]
Exec=\"/apps/Example.AppImage\" --new
Icon=action-icon
"
        );
    }

    #[test]
    fn keeps_icon_when_none_was_extracted_and_handles_bare_exec() {
        let rewritten = rewrite("[Desktop Entry]\nIcon=example\nExec=example\n", "/a.AppImage", None);
        assert_eq!(value(&rewritten, "Icon"), Some("example"));
        assert_eq!(value(&rewritten, "Exec"), Some("\"/a.AppImage\""));
    }

    #[test]
    fn quotes_special_characters_in_the_path() {
        let rewritten = rewrite("[Desktop Entry]\nExec=app %f\n", "/my apps/50% $off.AppImage", None);
        assert_eq!(value(&rewritten, "Exec"), Some("\"/my apps/50%% \\\\$off.AppImage\" %f"));
        assert_eq!(value(&rewritten, "TryExec"), Some("/my apps/50% $off.AppImage"));
    }

    const ENTRY: &str = "\
# comment
[Desktop Entry]
Name=Example App
Name[de]=Beispiel
Icon = example
Exec=example %U

[Desktop Action New]
Name=New Window
X-AppImage-Version=9.9
";

    #[test]
    fn reads_keys_from_the_main_group_only() {
        assert_eq!(value(ENTRY, "Name"), Some("Example App"));
        assert_eq!(value(ENTRY, "Icon"), Some("example"));
        assert_eq!(value(ENTRY, "Exec"), Some("example %U"));
        assert_eq!(value(ENTRY, "X-AppImage-Version"), None);
        assert_eq!(value(ENTRY, "Missing"), None);
    }
}

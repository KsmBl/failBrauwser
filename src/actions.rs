//! Custom actions in the right-click menu, in Thunar's format (`uca.xml`), so the actions
//! defined in Thunar work here too:
//!
//! ```xml
//! <action>
//!   <icon>utilities-terminal</icon>
//!   <name>Open Terminal Here</name>
//!   <command>exo-open --working-directory %f --launch TerminalEmulator</command>
//!   <patterns>*</patterns>
//!   <directories/>
//! </action>
//! ```
//!
//! An action shows when every selected item matches one of its `patterns` (`;`-separated
//! globs on the name) and one of its file kinds (`directories`, `audio-files`,
//! `image-files`, `text-files`, `video-files`, `other-files`). In the command `%f`/`%F` are
//! the first/all paths, `%d`/`%D` their folders, `%n`/`%N` their names, `%u`/`%U` URIs, `%%` a
//! percent sign — all shell-quoted.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomAction {
    pub name: String,
    pub icon: String,
    pub command: String,
    pub description: String,
    pub patterns: Vec<String>,
    pub unique_id: String,
    pub directories: bool,
    pub audio: bool,
    pub image: bool,
    pub text: bool,
    pub video: bool,
    pub other: bool,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Reads the actions of a `uca.xml`. Unknown elements are ignored.
pub fn parse(xml: &str) -> Vec<CustomAction> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<action>") {
        let after = &rest[start + 8..];
        let Some(end) = after.find("</action>") else { break };
        let body = &after[..end];
        rest = &after[end + 9..];
        let text = |tag: &str| -> String {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            body.find(&open)
                .and_then(|i| body[i + open.len()..].find(&close).map(|j| unescape(body[i + open.len()..i + open.len() + j].trim())))
                .unwrap_or_default()
        };
        let flag = |tag: &str| body.contains(&format!("<{tag}/>")) || body.contains(&format!("<{tag}>"));
        let patterns: Vec<String> = text("patterns").split(';').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
        let a = CustomAction {
            name: text("name"),
            icon: text("icon"),
            command: text("command"),
            description: text("description"),
            patterns: if patterns.is_empty() { vec!["*".into()] } else { patterns },
            unique_id: text("unique-id"),
            directories: flag("directories"),
            audio: flag("audio-files"),
            image: flag("image-files"),
            text: flag("text-files"),
            video: flag("video-files"),
            other: flag("other-files"),
        };
        if !a.name.is_empty() && !a.command.is_empty() {
            out.push(a);
        }
    }
    out
}

/// A selected item as the matching sees it.
#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub is_dir: bool,
    pub content_type: String,
}

/// Thunar patterns: wildcards match the whole name; a plain word must equal it.
fn glob(pattern: &str, name: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        crate::search::Query::new(pattern).matches(name)
    } else {
        pattern.eq_ignore_ascii_case(name)
    }
}

impl CustomAction {
    fn kind_ok(&self, t: &Target) -> bool {
        if t.is_dir {
            return self.directories;
        }
        let ct = t.content_type.as_str();
        if ct.starts_with("audio/") {
            self.audio
        } else if ct.starts_with("image/") {
            self.image
        } else if ct.starts_with("video/") {
            self.video
        } else if ct.starts_with("text/") {
            self.text
        } else {
            self.other
        }
    }

    /// Shown for this selection?
    pub fn applies_to(&self, targets: &[Target]) -> bool {
        !targets.is_empty()
            && targets.iter().all(|t| {
                let name = t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.kind_ok(t) && self.patterns.iter().any(|p| glob(p, &name))
            })
    }

    /// The command line for these targets, ready for `sh -c`.
    pub fn expand(&self, targets: &[Target]) -> String {
        let q = |s: &str| gtk::glib::shell_quote(s).to_string_lossy().into_owned();
        let paths: Vec<String> = targets.iter().map(|t| t.path.to_string_lossy().into_owned()).collect();
        let dirs: Vec<String> = targets.iter().map(|t| t.path.parent().unwrap_or(Path::new("/")).to_string_lossy().into_owned()).collect();
        let names: Vec<String> = targets.iter().map(|t| t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
        let uris: Vec<String> = targets.iter().filter_map(|t| gtk::glib::filename_to_uri(&t.path, None).ok().map(|u| u.to_string())).collect();
        let first = |v: &Vec<String>| v.first().map(|s| q(s)).unwrap_or_default();
        let all = |v: &Vec<String>| v.iter().map(|s| q(s)).collect::<Vec<_>>().join(" ");
        let mut out = String::new();
        let mut chars = self.command.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('f') => out.push_str(&first(&paths)),
                Some('F') => out.push_str(&all(&paths)),
                Some('d') => out.push_str(&first(&dirs)),
                Some('D') => out.push_str(&all(&dirs)),
                Some('n') => out.push_str(&first(&names)),
                Some('N') => out.push_str(&all(&names)),
                Some('u') => out.push_str(&first(&uris)),
                Some('U') => out.push_str(&all(&uris)),
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }
}

/// A terminal that exists on this system, for the built-in "Open Terminal Here".
pub fn terminal_command() -> Option<String> {
    let exists = |cmd: &str| gtk::glib::find_program_in_path(cmd).is_some();
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() && exists(t.split_whitespace().next().unwrap_or(&t)) {
            return Some(t);
        }
    }
    [
        ("xdg-terminal-exec", "xdg-terminal-exec"),
        ("exo-open", "exo-open --launch TerminalEmulator"),
        ("kgx", "kgx"),
        ("gnome-terminal", "gnome-terminal"),
        ("konsole", "konsole"),
        ("xfce4-terminal", "xfce4-terminal"),
        ("alacritty", "alacritty"),
        ("kitty", "kitty"),
        ("foot", "foot"),
        ("xterm", "xterm"),
    ]
    .iter()
    .find(|(bin, _)| exists(bin))
    .map(|(_, cmd)| cmd.to_string())
}

/// Thunar's actions, then ours (same name: ours wins), then the built-ins nobody replaced.
pub fn load_all(thunar: &Path, own: &Path) -> Vec<CustomAction> {
    let read = |p: &Path| std::fs::read_to_string(p).map(|t| parse(&t)).unwrap_or_default();
    let mut all: Vec<CustomAction> = Vec::new();
    for a in read(thunar).into_iter().chain(read(own)) {
        all.retain(|x| x.name != a.name);
        all.push(a);
    }
    if !all.iter().any(|a| a.name == "Open Terminal Here") {
        if let Some(term) = terminal_command() {
            all.insert(
                0,
                CustomAction {
                    name: "Open Terminal Here".into(),
                    icon: "utilities-terminal".into(),
                    // The terminal starts in the folder (the command runs there).
                    command: format!("cd %f && exec {term}"),
                    patterns: vec!["*".into()],
                    directories: true,
                    ..Default::default()
                },
            );
        }
    }
    all
}

pub fn thunar_file() -> PathBuf {
    gtk::glib::user_config_dir().join("Thunar/uca.xml")
}

pub fn own_file() -> PathBuf {
    gtk::glib::user_config_dir().join("failbrauwser/actions.xml")
}

/// A starting point for the user's own actions file.
pub const TEMPLATE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- failBrauwser custom actions, in Thunar's uca.xml format (Thunar's own
     ~/.config/Thunar/uca.xml is read too). Placeholders in <command>:
     %f/%F first/all paths, %d/%D their folders, %n/%N names, %u/%U URIs.
     File kinds: directories, audio-files, image-files, text-files, video-files, other-files. -->
<actions>
<action>
	<icon>edit-find</icon>
	<name>Count Lines</name>
	<command>wc -l %F | xmessage -file - || true</command>
	<description>Example: counts the lines of the selected text files</description>
	<patterns>*</patterns>
	<text-files/>
</action>
</actions>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const UCA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<actions>
<action>
	<icon>utilities-terminal</icon>
	<name>Open Terminal Here</name>
	<unique-id>1-1</unique-id>
	<command>exo-open --working-directory %f --launch TerminalEmulator</command>
	<patterns>*</patterns>
	<directories/>
</action>
<action>
	<name>Shrink &amp; Share</name>
	<command>convert %F -resize 50% out-%n; echo 100%%</command>
	<patterns>*.jpg;*.png</patterns>
	<image-files/>
</action>
</actions>"#;

    fn t(p: &str, dir: bool, ct: &str) -> Target {
        Target { path: PathBuf::from(p), is_dir: dir, content_type: ct.into() }
    }

    #[test]
    fn parses_thunar_actions() {
        let a = parse(UCA);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].name, "Open Terminal Here");
        assert!(a[0].directories && !a[0].image);
        assert_eq!(a[1].name, "Shrink & Share");
        assert_eq!(a[1].patterns, vec!["*.jpg", "*.png"]);
    }

    #[test]
    fn matching_by_kind_and_pattern() {
        let a = parse(UCA);
        assert!(a[0].applies_to(&[t("/x/dir", true, "inode/directory")]));
        assert!(!a[0].applies_to(&[t("/x/a.jpg", false, "image/jpeg")]));
        assert!(a[1].applies_to(&[t("/x/a.jpg", false, "image/jpeg"), t("/x/b.PNG", false, "image/png")]));
        assert!(!a[1].applies_to(&[t("/x/a.jpg", false, "image/jpeg"), t("/x/c.gif", false, "image/gif")]));
        assert!(!a[1].applies_to(&[]));
    }

    #[test]
    fn placeholders_are_quoted() {
        let a = parse(UCA);
        let cmd = a[1].expand(&[t("/p/my photo.jpg", false, "image/jpeg"), t("/p/it's.png", false, "image/png")]);
        assert_eq!(cmd, "convert '/p/my photo.jpg' '/p/it'\\''s.png' -resize 50% out-'my photo.jpg'; echo 100%");
        assert_eq!(a[0].expand(&[t("/home/me", true, "inode/directory")]), "exo-open --working-directory '/home/me' --launch TerminalEmulator");
    }

    #[test]
    fn own_actions_override_thunar_and_builtin_steps_aside() {
        let d = tempfile::tempdir().unwrap();
        let thunar = d.path().join("uca.xml");
        let own = d.path().join("actions.xml");
        std::fs::write(&thunar, UCA).unwrap();
        std::fs::write(&own, "<actions><action><name>Shrink &amp; Share</name><command>mine %f</command><other-files/></action></actions>").unwrap();
        let all = load_all(&thunar, &own);
        assert_eq!(all.iter().filter(|a| a.name == "Open Terminal Here").count(), 1);
        assert_eq!(all.iter().find(|a| a.name == "Shrink & Share").unwrap().command, "mine %f");
        assert!(parse(TEMPLATE).len() == 1);
    }
}

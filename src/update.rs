//! Updating failBrauwser from Help > Check for Updates: the newest release on GitHub (a
//! ready-built package) or the current source, built here. The work is done by a shell
//! script started on its own, so it outlives the program it replaces; the source it builds
//! from is deleted when it is done.

use std::path::{Path, PathBuf};

pub const REPO: &str = "KsmBl/failBrauwser";

/// Where the newest release is asked for (`FB_UPDATE_API` replaces it in tests).
pub fn release_api() -> String {
    std::env::var("FB_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

/// Where the current source is fetched from (`FB_UPDATE_SOURCE` replaces it in tests).
pub fn source_url() -> String {
    std::env::var("FB_UPDATE_SOURCE").unwrap_or_else(|_| format!("https://codeload.github.com/{REPO}/tar.gz/refs/heads/main"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub name: String,
    /// When it was published (`2026-10-30T12:00:00Z`).
    pub published: String,
    /// The ready-built package for this machine, if the release has one.
    pub package: Option<String>,
    pub page: String,
}

/// The name of the ready-built package for a version and this machine.
pub fn package_name(version: &str) -> String {
    format!("failbrauwser-{}-{}-linux.tar.gz", version.trim_start_matches('v'), std::env::consts::ARCH)
}

/// Reads GitHub's answer for the latest release.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v["tag_name"].as_str()?.to_string();
    let suffix = format!("-{}-linux.tar.gz", std::env::consts::ARCH);
    let package = v["assets"].as_array().and_then(|a| {
        a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.ends_with(&suffix))).and_then(|x| x["browser_download_url"].as_str()).map(str::to_string)
    });
    Some(Release {
        name: v["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&tag).to_string(),
        published: v["published_at"].as_str().unwrap_or_default().to_string(),
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        tag,
        package,
    })
}

fn numbers(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split(['.', '-']).map_while(|p| p.parse().ok()).collect()
}

/// Whether version `tag` (`v0.6`) is newer than `current` (`0.5.0`).
pub fn is_newer(tag: &str, current: &str) -> bool {
    let (mut a, mut b) = (numbers(tag), numbers(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// The installation this program runs from: `~/.local` for `~/.local/bin/failbrauwser`.
pub fn prefix_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(Path::to_path_buf)).flatten()
}

/// The options the last install was made with (kept by install.sh), without the ones that
/// only make sense once.
pub fn install_options(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|s| s.split_whitespace().filter(|o| !matches!(*o, "--uninstall" | "--prebuilt" | "--reopen")).map(str::to_string).collect())
        .unwrap_or_default()
}

/// `'text'` for the shell.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell script that installs a downloaded release package, or builds the source
/// (`package: None`), into `prefix` with `options`; the download goes into a folder of its own
/// below `cache` that is removed at the end, whatever happened.
pub fn script(package: Option<&str>, prefix: &Path, options: &[String], cache: &Path) -> String {
    let opts: String = options.iter().map(|o| format!(" {}", quote(o))).collect();
    let fetch = |url: &str, into: &str| format!("curl -fL --retry 2 -o {into} {} || wget -O {into} {}", quote(url), quote(url));
    let (get, run) = match package {
        Some(url) => (
            format!("echo \"Downloading {url}\"\n{}\ntar xzf package.tar.gz\ncd failbrauwser-*/", fetch(url, "package.tar.gz")),
            format!("./install.sh --prebuilt --prefix {}{opts} --reopen", quote(&prefix.display().to_string())),
        ),
        None => (
            format!("echo \"Downloading the source\"\n{}\nmkdir source\ntar xzf source.tar.gz -C source --strip-components=1\ncd source", fetch(&source_url(), "source.tar.gz")),
            format!("./install.sh --prefix {}{opts} --reopen", quote(&prefix.display().to_string())),
        ),
    };
    format!(
        "#!/bin/sh\n# failBrauwser update (Help > Check for Updates)\nset -e\nmkdir -p {cache}\nwork=$(mktemp -d {cache}/failbrauwser-update.XXXXXX)\ntrap 'cd /; rm -rf \"$work\"; echo \"Removed the downloaded files.\"' EXIT\ncd \"$work\"\n{get}\necho \"Installing\"\n{run}\necho \"UPDATE FINISHED\"\n",
        cache = quote(&cache.display().to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_and_versions() {
        let pkg = package_name("v0.6");
        assert_eq!(pkg, format!("failbrauwser-0.6-{}-linux.tar.gz", std::env::consts::ARCH));
        let json = format!(
            r#"{{"tag_name": "v0.6", "name": "failBrauwser 0.6", "published_at": "2026-10-30T19:00:00Z", "html_url": "https://github.com/{REPO}/releases/tag/v0.6",
               "assets": [{{"name": "notes.txt", "browser_download_url": "https://x/notes.txt"}}, {{"name": "{pkg}", "browser_download_url": "https://x/{pkg}"}}]}}"#
        );
        let r = parse_release(&json).unwrap();
        assert_eq!((r.tag.as_str(), r.name.as_str()), ("v0.6", "failBrauwser 0.6"));
        assert_eq!(r.package, Some(format!("https://x/{pkg}")));
        assert!(parse_release(r#"{"tag_name": "v0.2", "assets": []}"#).unwrap().package.is_none());
        assert!(parse_release("not json").is_none());
        assert!(is_newer("v0.6", "0.5.0") && is_newer("v0.10", "0.9.3") && is_newer("v1.0", "0.99"));
        assert!(!is_newer("v0.5", "0.5.0") && !is_newer("v0.4", "0.5.0"));
    }

    #[test]
    fn installs_and_scripts() {
        assert_eq!(prefix_of(Path::new("/home/me/.local/bin/failbrauwser")), Some(PathBuf::from("/home/me/.local")));
        assert_eq!(prefix_of(Path::new("/home/me/src/target/release/failbrauwser")), None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("install-options");
        std::fs::write(&f, "--default --no-network --reopen\n").unwrap();
        assert_eq!(install_options(&f), ["--default", "--no-network"]);
        assert!(install_options(&dir.path().join("none")).is_empty());
        assert_eq!(quote("it's"), r"'it'\''s'");
        let s = script(Some("https://x/p.tar.gz"), Path::new("/home/me/.local"), &["--default".into()], Path::new("/home/me/.cache"));
        assert!(s.contains("./install.sh --prebuilt --prefix '/home/me/.local' '--default' --reopen"), "{s}");
        assert!(s.contains("trap 'cd /; rm -rf \"$work\"") && s.contains("mktemp -d '/home/me/.cache'/failbrauwser-update.XXXXXX"));
        let s = script(None, Path::new("/opt/fb"), &[], Path::new("/c"));
        assert!(s.contains("tar xzf source.tar.gz -C source --strip-components=1") && s.contains("./install.sh --prefix '/opt/fb' --reopen"));
    }
}

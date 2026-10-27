//! The colors of the window manager's theme where it has them: tileWin's `theme.conf`
//! (the theme named in `~/.config/tileWin/current-theme`, with the keys of the theme it
//! inherits from and its dark variant). The progress window draws its speed chart in them,
//! as tileWin draws the charts of its taskbar flyouts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A color as red, green, blue and alpha, each 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    pub fn from_u32(c: u32) -> Rgba {
        let b = |s: u32| ((c >> s) & 0xff) as f64 / 255.0;
        Rgba(b(24), b(16), b(8), b(0))
    }

    /// `#rrggbb` or `#rrggbbaa`.
    pub fn parse(text: &str) -> Option<Rgba> {
        let hex = text.trim().strip_prefix('#')?;
        let v = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            6 => Some(Rgba::from_u32(v << 8 | 0xff)),
            8 => Some(Rgba::from_u32(v)),
            _ => None,
        }
    }

    pub fn with_alpha(self, a: f64) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }

    /// `self` moved toward `other` by `t`.
    pub fn mix(self, other: Rgba, t: f64) -> Rgba {
        let m = |a: f64, b: f64| a + (b - a) * t;
        Rgba(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2), self.3)
    }

    /// Perceived brightness, 0..=1.
    pub fn luma(self) -> f64 {
        0.299 * self.0 + 0.587 * self.1 + 0.114 * self.2
    }
}

/// A theme's keys flattened: `flyout.chart_bg` → `#000000`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Theme {
    pub keys: HashMap<String, String>,
}

impl Theme {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    /// A color key: `#rrggbb[aa]` or `$accent`, `$accent_light`, `$accent_dark` with an
    /// optional `/aa` alpha, as tileWin reads them.
    pub fn color(&self, key: &str) -> Option<Rgba> {
        let value = self.get(key)?;
        let Some(rest) = value.strip_prefix("$accent") else { return Rgba::parse(value) };
        let mut c = self.get("accent.color").filter(|v| !v.starts_with('$')).and_then(Rgba::parse).unwrap_or(Rgba::from_u32(0x0078d7ff));
        let rest = if let Some(r) = rest.strip_prefix("_light") {
            c = c.mix(Rgba(1.0, 1.0, 1.0, 1.0), 0.45);
            r
        } else if let Some(r) = rest.strip_prefix("_dark") {
            c = c.mix(Rgba(0.0, 0.0, 0.0, 1.0), 0.35);
            r
        } else {
            rest
        };
        if let Some(a) = rest.strip_prefix('/').and_then(|a| u8::from_str_radix(a.get(..2)?, 16).ok()) {
            c.3 = a as f64 / 255.0;
        }
        Some(c)
    }

    /// The classic look (Windows 95 and older): sunken edges, square corners.
    pub fn classic(&self) -> bool {
        let style = self.get("panel.style").or_else(|| self.get("style")).unwrap_or("win10");
        matches!(style, "classic" | "win95" | "win3" | "win1")
    }
}

/// Splits `theme.conf` text into words and the symbols `{`, `}` and `;` (a line end is a
/// `;` too). `#` starts a comment at the start of a statement or before a space, else it
/// is part of a word (`#000080`).
fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut line_has_token = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' || c == ';' {
            out.push(";".into());
            line_has_token = false;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '#' && (!line_has_token || matches!(chars.get(i + 1), None | Some(' ' | '\t' | '#' | '\n'))) {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '{' || c == '}' {
            out.push(c.to_string());
            line_has_token = false;
            i += 1;
        } else if c == '"' || c == '\'' {
            let mut word = String::new();
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && c == '"' && i + 1 < chars.len() {
                    i += 1;
                }
                word.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(format!("\"{word}"));
            line_has_token = true;
        } else {
            let mut word = String::new();
            while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], ';' | '{' | '}') {
                word.push(chars[i]);
                i += 1;
            }
            out.push(word);
            line_has_token = true;
        }
    }
    out
}

/// The keys of one `theme.conf` file, with block names joined by dots.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let word = |w: &String| w.strip_prefix('"').unwrap_or(w).to_string();
    let key = |path: &[String], name: &str| if path.is_empty() { name.to_string() } else { format!("{}.{name}", path.join(".")) };
    for t in tokens(text) {
        match t.as_str() {
            ";" | "}" => {
                if let Some((name, args)) = words.split_first() {
                    if !args.is_empty() {
                        out.push((key(&path, &word(name)), args.iter().map(word).collect::<Vec<_>>().join(" ")));
                    }
                }
                words.clear();
                if t == "}" {
                    path.pop();
                }
            }
            "{" => {
                if let Some((name, args)) = words.split_first() {
                    if !args.is_empty() {
                        out.push((key(&path, &word(name)), args.iter().map(word).collect::<Vec<_>>().join(" ")));
                    }
                    path.push(word(name));
                } else {
                    path.push(String::new());
                }
                words.clear();
            }
            _ => words.push(t),
        }
    }
    out
}

/// Where tileWin keeps its settings.
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("tileWin")
}

/// The folder of the theme called `name`: the user's own first, then the installed ones.
fn find_theme(config: &Path, data_dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.join("theme.conf").is_file().then_some(p);
    }
    std::iter::once(config.join("themes")).chain(data_dirs.iter().map(|d| d.join("themes"))).map(|d| d.join(name)).find(|d| d.join("theme.conf").is_file())
}

fn first_line(p: &Path) -> Option<String> {
    let text = std::fs::read_to_string(p).ok()?;
    Some(text.lines().next()?.trim().to_string()).filter(|l| !l.is_empty())
}

/// The active tileWin theme as tileWin reads it, from `config` (its settings folder) and
/// the installed themes in `data_dirs`; None when tileWin is not set up.
pub fn load_from(config: &Path, data_dirs: &[PathBuf]) -> Option<Theme> {
    let name = first_line(&config.join("current-theme"))?;
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut load = |name: &str| -> Option<()> {
        // The theme inherited from comes first, so the theme's own keys win.
        let mut chain = Vec::new();
        let mut next = Some(name.to_string());
        while let Some(n) = next.take() {
            if chain.len() > 5 {
                break;
            }
            let dir = find_theme(config, data_dirs, &n)?;
            let parsed = parse(&std::fs::read_to_string(dir.join("theme.conf")).ok()?);
            next = parsed.iter().find(|(k, _)| k == "inherit").map(|(_, v)| v.clone());
            chain.push(parsed);
        }
        for parsed in chain.into_iter().rev() {
            keys.extend(parsed.into_iter().filter(|(k, _)| k != "inherit"));
        }
        Some(())
    };
    load(&name)?;
    let scheme = keys.iter().rev().find(|(k, _)| k == "scheme").map(|(_, v)| v.to_lowercase());
    let dark = match scheme.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => first_line(&config.join("color-scheme")).is_some_and(|s| s.eq_ignore_ascii_case("dark")),
    };
    if dark {
        let overrides: Vec<(String, String)> = keys.iter().filter_map(|(k, v)| Some((k.strip_prefix("dark.")?.to_string(), v.clone()))).collect();
        keys.extend(overrides);
    }
    Some(Theme { keys: keys.into_iter().collect() })
}

/// The active tileWin theme of this user, if tileWin is set up.
pub fn load() -> Option<Theme> {
    let data = ["/usr/local/share/tileWin", "/usr/share/tileWin"].map(PathBuf::from);
    load_from(&config_dir(), &data)
}

/// How a chart looks: the colors tileWin draws its flyout charts in.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartStyle {
    pub bg: Rgba,
    pub grid: Rgba,
    pub line: Rgba,
    /// Text on the chart.
    pub text: Rgba,
    pub radius: f64,
    /// Sunken edges (highlight, light, shadow, dark), for the classic look.
    pub bevel: Option<[Rgba; 4]>,
}

impl ChartStyle {
    /// The chart of `theme`, as tileWin's flyouts derive it; `under` is the color the chart
    /// is drawn on (the window background).
    pub fn from_theme(theme: &Theme, under: Rgba) -> ChartStyle {
        let classic = theme.classic();
        let fly = |name: &str| theme.color(&format!("flyout.{name}")).or_else(|| theme.color(&format!("menu.{name}")));
        let flyout_bg = fly("bg").unwrap_or(Rgba::from_u32(0xf2f2f2ff));
        let dark = flyout_bg.luma() < 0.5;
        let overlay = if dark { Rgba(1.0, 1.0, 1.0, 0.0) } else { Rgba(0.0, 0.0, 0.0, 0.0) };
        let accent = theme
            .color("flyout.accent")
            .unwrap_or_else(|| if classic { Rgba::from_u32(0x000080ff) } else { theme.color("taskbar.indicator").unwrap_or(Rgba::from_u32(0x0078d4ff)) });
        let bg = theme.color("flyout.chart_bg").unwrap_or(overlay.with_alpha(if dark { 0x18 } else { 0x10 } as f64 / 255.0));
        let grid = theme
            .color("flyout.chart_grid")
            .unwrap_or(if classic { Rgba::from_u32(0x808080ff) } else { overlay.with_alpha(if dark { 0x2a } else { 0x1f } as f64 / 255.0) });
        let line = theme.color("flyout.chart_line").unwrap_or(accent);
        let radius = theme.get("flyout.chart_radius").and_then(|r| r.parse().ok()).unwrap_or(if classic { 0.0 } else { 6.0 });
        let bevel = classic.then(|| {
            let c = |k: &str, d: u32| theme.color(&format!("decoration.{k}")).unwrap_or(Rgba::from_u32(d));
            [c("highlight", 0xffffffff), c("light", 0xdfdfdfff), c("shadow", 0x808080ff), c("dark", 0x000000ff)]
        });
        ChartStyle { text: text_on(bg, under), bg, grid, line, radius, bevel }
    }

    /// A chart in the GTK theme's colors, where the window manager has none.
    pub fn plain(base: Rgba, accent: Rgba, border: Rgba, under: Rgba) -> ChartStyle {
        ChartStyle { bg: base, grid: border.with_alpha(0.6), line: accent, text: text_on(base, under), radius: 4.0, bevel: None }
    }
}

/// Text that reads on `bg` drawn over `under`.
fn text_on(bg: Rgba, under: Rgba) -> Rgba {
    let seen = under.mix(bg, bg.3);
    if seen.luma() < 0.5 { Rgba(1.0, 1.0, 1.0, 0.9) } else { Rgba(0.0, 0.0, 0.0, 0.85) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"# a theme
name "Windows 95"
style win95
decoration {
	face #c0c0c0
	shadow #808080
	active {
		title_gradient "0:#000080 1:#1084d0"
	}
}
panel { bg #c0c0c0; style classic }
flyout {
	# charts as the Task Manager drew them
	chart_bg #000000
	chart_grid #008040
	chart_line #00ff00 # bright
}
dark {
	decoration {
		shadow #202020
	}
	flyout { link #8fa8ff; bar #3a5fd0 }
}
"#;

    #[test]
    fn parses_blocks_one_liners_comments_and_quotes() {
        let keys: HashMap<String, String> = parse(BASE).into_iter().collect();
        assert_eq!(keys["name"], "Windows 95");
        assert_eq!(keys["decoration.active.title_gradient"], "0:#000080 1:#1084d0");
        assert_eq!(keys["panel.style"], "classic");
        assert_eq!(keys["panel.bg"], "#c0c0c0");
        assert_eq!(keys["flyout.chart_line"], "#00ff00");
        assert_eq!(keys["dark.flyout.bar"], "#3a5fd0");
        assert_eq!(keys["dark.decoration.shadow"], "#202020");
        assert!(!keys.keys().any(|k| k.contains("charts") || k.contains("bright")));
    }

    #[test]
    fn colors_and_accents() {
        assert_eq!(Rgba::parse("#ff000080"), Some(Rgba(1.0, 0.0, 0.0, 128.0 / 255.0)));
        assert_eq!(Rgba::parse("#00ff00"), Some(Rgba(0.0, 1.0, 0.0, 1.0)));
        assert_eq!(Rgba::parse("red"), None);
        let t = Theme { keys: [("accent.color", "#0078d7"), ("a", "$accent"), ("b", "$accent/d0"), ("c", "$accent_light")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
        assert_eq!(t.color("a"), Rgba::parse("#0078d7"));
        assert_eq!(t.color("b").unwrap().3, 0xd0 as f64 / 255.0);
        assert!(t.color("c").unwrap().luma() > t.color("a").unwrap().luma());
        assert_eq!(t.color("missing"), None);
    }

    fn setup(dir: &Path, scheme: &str) -> (PathBuf, Vec<PathBuf>) {
        let config = dir.join("config");
        let data = dir.join("data");
        std::fs::create_dir_all(data.join("themes/win95")).unwrap();
        std::fs::write(data.join("themes/win95/theme.conf"), BASE).unwrap();
        // The user's own theme, built on the installed one.
        std::fs::create_dir_all(config.join("themes/mine")).unwrap();
        std::fs::write(config.join("themes/mine/theme.conf"), "inherit win95\nflyout { chart_line #ff8800 }\n").unwrap();
        std::fs::write(config.join("color-scheme"), scheme).unwrap();
        (config, vec![data])
    }

    #[test]
    fn active_theme_with_inheritance_and_dark_variant() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = setup(dir.path(), "dark\n");
        assert_eq!(load_from(&config, &data), None);
        std::fs::write(config.join("current-theme"), "mine\n").unwrap();
        let t = load_from(&config, &data).unwrap();
        assert_eq!(t.get("flyout.chart_line"), Some("#ff8800"));
        assert_eq!(t.get("flyout.chart_bg"), Some("#000000"));
        assert_eq!(t.get("decoration.shadow"), Some("#202020"));
        assert!(t.classic());
        std::fs::write(config.join("color-scheme"), "light\n").unwrap();
        assert_eq!(load_from(&config, &data).unwrap().get("decoration.shadow"), Some("#808080"));
        // A theme that is not there.
        std::fs::write(config.join("current-theme"), "gone\n").unwrap();
        assert_eq!(load_from(&config, &data), None);
    }

    #[test]
    fn chart_styles() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = setup(dir.path(), "light\n");
        std::fs::write(config.join("current-theme"), "win95\n").unwrap();
        let t = load_from(&config, &data).unwrap();
        let grey = Rgba::from_u32(0xc0c0c0ff);
        let s = ChartStyle::from_theme(&t, grey);
        // Green on black, square and sunken, as the Task Manager drew it.
        assert_eq!(s.bg, Rgba::from_u32(0x000000ff));
        assert_eq!(s.line, Rgba::from_u32(0x00ff00ff));
        assert_eq!(s.grid, Rgba::from_u32(0x008040ff));
        assert_eq!(s.radius, 0.0);
        assert_eq!(s.bevel.unwrap()[2], Rgba::from_u32(0x808080ff));
        assert!(s.text.luma() > 0.5);
        // A modern theme without chart colors: the accent on a faint overlay, rounded.
        let modern = Theme { keys: [("style", "win10"), ("accent.color", "#0078d7"), ("flyout.accent", "$accent"), ("flyout.bg", "#1f1f1ff2")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
        let s = ChartStyle::from_theme(&modern, Rgba::from_u32(0x202020ff));
        assert_eq!(s.line, Rgba::parse("#0078d7").unwrap());
        assert_eq!(s.radius, 6.0);
        assert!(s.bevel.is_none());
        assert!(s.bg.3 < 0.2 && s.bg.luma() > 0.9);
        assert!(s.text.luma() > 0.5);
    }
}

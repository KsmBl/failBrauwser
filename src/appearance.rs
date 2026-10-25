//! Light or dark, as the desktop says: the colour scheme (the `org.freedesktop.appearance`
//! portal setting, GNOME's `color-scheme`) decides, and the GTK theme follows it — its dark
//! variant, or the light one a "-dark" theme comes from.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// No preference: the theme decides.
    Default,
    Dark,
    Light,
}

impl Scheme {
    /// The portal's value: 0 no preference, 1 dark, 2 light.
    pub fn from_portal(v: u32) -> Scheme {
        match v {
            1 => Scheme::Dark,
            2 => Scheme::Light,
            _ => Scheme::Default,
        }
    }

    /// GNOME's `color-scheme`: "default", "prefer-dark", "prefer-light".
    pub fn from_gsettings(s: &str) -> Scheme {
        match s {
            "prefer-dark" => Scheme::Dark,
            "prefer-light" => Scheme::Light,
            _ => Scheme::Default,
        }
    }
}

/// How GTK is set up for a scheme: the theme to use and whether to ask for its dark variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub theme: String,
    pub prefer_dark: bool,
}

/// Whether a theme's name says it is dark ("Adwaita-dark", "Arc-Dark", "Yaru_dark").
pub fn is_dark_name(theme: &str) -> bool {
    let t = theme.to_ascii_lowercase();
    ["-dark", "_dark", " dark", "-darker"].iter().any(|s| t.ends_with(s))
}

/// The light theme a dark one belongs to: "Adwaita-dark" → "Adwaita".
pub fn light_name(theme: &str) -> Option<String> {
    let t = theme.to_ascii_lowercase();
    ["-darker", "-dark", "_dark", " dark"].iter().find(|s| t.ends_with(*s)).map(|s| theme[..theme.len() - s.len()].to_string())
}

/// The look for `theme` (the desktop's GTK theme) under `scheme`. `exists` says whether a
/// theme is installed, `has_dark` whether it has a dark variant (`gtk-dark.css`).
pub fn look_for(theme: &str, scheme: Scheme, exists: impl Fn(&str) -> bool, has_dark: impl Fn(&str) -> bool) -> Look {
    let light = light_name(theme).filter(|l| exists(l));
    match scheme {
        Scheme::Default => Look { theme: theme.into(), prefer_dark: false },
        Scheme::Light => Look { theme: light.unwrap_or_else(|| theme.into()), prefer_dark: false },
        Scheme::Dark if is_dark_name(theme) && exists(theme) => Look { theme: theme.into(), prefer_dark: false },
        // A dark theme that is not installed would fall back to a light one.
        Scheme::Dark if light.is_some() => Look { theme: light.unwrap_or_default(), prefer_dark: true },
        Scheme::Dark => {
            // The theme's own dark variant, else a "-dark" sibling, else ask GTK for dark.
            if has_dark(theme) {
                return Look { theme: theme.into(), prefer_dark: true };
            }
            let sibling = ["-dark", "-Dark", "_dark"].iter().map(|s| format!("{theme}{s}")).find(|n| exists(n));
            match sibling {
                Some(s) => Look { theme: s, prefer_dark: false },
                None => Look { theme: theme.into(), prefer_dark: true },
            }
        }
    }
}

/// Themes GTK has built in.
const BUILTIN: &[&str] = &["Adwaita", "HighContrast", "HighContrastInverse"];

/// Folders a GTK 3 theme can be in.
fn theme_dirs(name: &str) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![gtk::glib::user_data_dir().join("themes").join(name), gtk::glib::home_dir().join(".themes").join(name)];
    dirs.extend(gtk::glib::system_data_dirs().into_iter().map(|d| d.join("themes").join(name)));
    dirs
}

/// Whether a GTK 3 theme is installed (or built in).
pub fn theme_exists(name: &str) -> bool {
    BUILTIN.contains(&name) || theme_dirs(name).iter().any(|d| d.join("gtk-3.0").join("gtk.css").is_file())
}

/// Whether a theme brings a dark variant of its own (Adwaita has one built in).
pub fn theme_has_dark(name: &str) -> bool {
    name == "Adwaita" || theme_dirs(name).iter().any(|d| has_dark_css(d))
}

fn has_dark_css(dir: &Path) -> bool {
    dir.join("gtk-3.0").join("gtk-dark.css").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_dark_name("Adwaita-dark") && is_dark_name("Arc-Dark") && is_dark_name("Yaru_dark"));
        assert!(!is_dark_name("Adwaita") && !is_dark_name("Darkly"));
        assert_eq!(light_name("Adwaita-dark").as_deref(), Some("Adwaita"));
        assert_eq!(light_name("Arc-Darker").as_deref(), Some("Arc"));
        assert_eq!(light_name("Adwaita"), None);
        assert_eq!(Scheme::from_portal(1), Scheme::Dark);
        assert_eq!(Scheme::from_gsettings("prefer-light"), Scheme::Light);
    }

    #[test]
    fn the_scheme_picks_the_variant() {
        let installed = ["Adwaita", "Adwaita-dark", "Arc", "Arc-Dark", "Plain"];
        let exists = |n: &str| installed.contains(&n);
        let has_dark = |n: &str| n == "Adwaita";
        let look = |t: &str, s| look_for(t, s, exists, has_dark);
        // Light: a dark theme goes back to its light one, and nothing asks for dark.
        assert_eq!(look("Adwaita-dark", Scheme::Light), Look { theme: "Adwaita".into(), prefer_dark: false });
        assert_eq!(look("Adwaita", Scheme::Light), Look { theme: "Adwaita".into(), prefer_dark: false });
        // Dark: the built-in dark Adwaita, a "-Dark" sibling, or GTK's dark preference.
        assert_eq!(look("Adwaita", Scheme::Dark), Look { theme: "Adwaita".into(), prefer_dark: true });
        assert_eq!(look("Arc", Scheme::Dark), Look { theme: "Arc-Dark".into(), prefer_dark: false });
        assert_eq!(look("Plain", Scheme::Dark), Look { theme: "Plain".into(), prefer_dark: true });
        assert_eq!(look("Adwaita-dark", Scheme::Dark), Look { theme: "Adwaita-dark".into(), prefer_dark: false });
        // No preference: the theme as it is.
        assert_eq!(look("Arc-Dark", Scheme::Default), Look { theme: "Arc-Dark".into(), prefer_dark: false });
        // "Adwaita-dark" is not built into GTK 3: without the theme, dark Adwaita it is.
        let only_light = |n: &str| n == "Adwaita";
        assert_eq!(look_for("Adwaita-dark", Scheme::Dark, only_light, has_dark), Look { theme: "Adwaita".into(), prefer_dark: true });
        // A dark theme without a light one stays.
        assert_eq!(look("Only-dark", Scheme::Light).theme, "Only-dark");
    }
}

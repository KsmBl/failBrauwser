//! How a new archive is written: the choices a format offers (its compression methods and
//! the settings it describes itself, as the helper reports them), the settings picked, and
//! splitting the result into parts.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// One setting a format describes: a version, dictionary size, file system type, …
#[derive(Debug, Clone, PartialEq)]
pub struct OptionDesc {
    pub key: String,
    pub name: String,
    pub kind: OptionKind,
    pub default: String,
    /// The values to pick from (for a number: suggestions; empty: free input).
    pub values: Vec<String>,
    pub description: Option<String>,
    /// Shown only while another setting has one of some values: `"Key=a|b"`.
    pub depends: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Text,
    Number,
    Switch,
    Choice,
}

/// What can be chosen for a format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatOptions {
    /// Compression methods: id and name, the format's default first.
    pub methods: Vec<(String, String)>,
    pub options: Vec<OptionDesc>,
}

/// Keys that have their own place in the dialog rather than in the advanced section.
pub const VERSION_KEYS: &[&str] = &["TargetCompatibility", "Format"];
pub const LEVEL_KEYS: &[&str] = &["Level", "CompressionLevel"];
/// Settings asked for elsewhere (method, password, encryption).
const HANDLED_KEYS: &[&str] = &["Method", "Password", "EncryptionMethod"];

impl FormatOptions {
    /// From the helper's answer to `options`.
    pub fn from_json(v: &Value) -> FormatOptions {
        let s = |o: &Value, k: &str| o[k].as_str().unwrap_or_default().to_string();
        let methods = v["methods"].as_array().map(|a| a.iter().map(|m| (s(m, "id"), s(m, "name"))).collect()).unwrap_or_default();
        let options = v["options"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|o| OptionDesc {
                        key: s(o, "key"),
                        name: s(o, "name"),
                        kind: match o["kind"].as_str() {
                            Some("integer") => OptionKind::Number,
                            Some("boolean") => OptionKind::Switch,
                            Some("enum") => OptionKind::Choice,
                            _ => OptionKind::Text,
                        },
                        default: s(o, "default"),
                        values: o["values"].as_array().map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                        description: o["description"].as_str().map(str::to_string),
                        depends: o["depends"].as_str().map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default();
        FormatOptions { methods, options }
    }

    fn find(&self, keys: &[&str]) -> Option<&OptionDesc> {
        self.options.iter().find(|o| keys.contains(&o.key.as_str()) && (o.kind == OptionKind::Choice || !o.values.is_empty()))
    }

    /// The setting that picks which version of the format is written.
    pub fn version(&self) -> Option<&OptionDesc> {
        self.find(VERSION_KEYS)
    }

    /// The format's own compression level setting.
    pub fn level(&self) -> Option<&OptionDesc> {
        self.find(LEVEL_KEYS)
    }

    /// The methods worth offering: for RAR the versions double as methods ("rar5", …),
    /// which the version setting covers, so only compressing or storing is left.
    pub fn method_choices(&self, format: &str) -> Vec<(String, String)> {
        if format == "Rar" {
            return self.methods.iter().filter(|(id, _)| id == "store").cloned().collect();
        }
        self.methods.clone()
    }

    /// The settings for the advanced section.
    pub fn advanced(&self) -> Vec<&OptionDesc> {
        let version = self.version().map(|o| o.key.as_str());
        let level = self.level().map(|o| o.key.as_str());
        self.options
            .iter()
            .filter(|o| !HANDLED_KEYS.contains(&o.key.as_str()) && Some(o.key.as_str()) != version && Some(o.key.as_str()) != level)
            .collect()
    }
}

/// Whether a setting is shown, given the current values of the others.
pub fn depends_met(o: &OptionDesc, values: &HashMap<String, String>) -> bool {
    let Some(dep) = &o.depends else { return true };
    let Some((key, allowed)) = dep.split_once('=') else { return true };
    let current = values.get(key.trim()).map(String::as_str).unwrap_or_default();
    allowed.split('|').any(|a| a.trim().eq_ignore_ascii_case(current))
}

/// A version value as people know it.
pub fn version_label(format: &str, value: &str) -> String {
    match (format, value) {
        ("Rar", "Rar5") => "RAR 5 (WinRAR 5.0 and later)".into(),
        ("Rar", "Rar4") => "RAR 4 (WinRAR 2.9 – 4.x, also 3.5)".into(),
        ("Rar", "Rar1_5") => "RAR 1.5 (stored only, very old programs)".into(),
        (_, "ustar") => "POSIX ustar (every tar)".into(),
        (_, "gnu") => "GNU tar (long names)".into(),
        (_, "pax") => "POSIX pax (long names, exact times)".into(),
        _ => value.into(),
    }
}

/// A method's name, for ZIP with the ZIP version a program needs to open it.
pub fn method_label(format: &str, id: &str, name: &str) -> String {
    if format != "Zip" {
        return name.into();
    }
    let version = match id {
        "store" => "ZIP 1.0, every unzip program",
        "deflate" => "ZIP 2.0, every unzip program",
        "deflate64" => "ZIP 2.1",
        "bzip2" => "ZIP 4.6",
        "lzma" => "ZIP 6.3",
        "ppmd" => "ZIP 6.3",
        "zstd" => "ZIP 6.3.7",
        _ => return name.into(),
    };
    format!("{name} ({version})")
}

/// A level value as words.
pub fn level_label(value: &str) -> String {
    match value {
        "0" => "0 – store".into(),
        "1" => "1 – fastest".into(),
        "3" => "3 – fast".into(),
        "5" => "5 – normal".into(),
        "7" => "7 – maximum".into(),
        "9" => "9 – ultra".into(),
        v => v.into(),
    }
}

/// A value of a setting as words: sizes in bytes as "64 KB", a solid block size of 0 as one
/// block for everything.
pub fn value_label(key: &str, value: &str) -> String {
    if LEVEL_KEYS.contains(&key) {
        return level_label(value);
    }
    if key.ends_with("Size") {
        if let Ok(n) = value.parse::<u64>() {
            if key == "SolidSize" && n == 0 {
                return "One block (fully solid)".into();
            }
            if n >= 1024 {
                return binary_size(n);
            }
        }
    }
    value.into()
}

/// "64 KB", "16 MB", "1.5 GB" (1 KB = 1024 bytes, as archivers count).
pub fn binary_size(n: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v.fract() == 0.0 { format!("{v} {}", units[u]) } else { format!("{v:.1} {}", units[u]) }
}

/// Levels for formats that take the general level setting but do not describe one: (value,
/// label), and the one selected at first. RAR names its six steps like WinRAR does.
pub fn general_levels(format: &str) -> Option<(Vec<(&'static str, &'static str)>, &'static str)> {
    match format {
        "Rar" => Some((vec![("0", "Store"), ("1", "Fastest"), ("2", "Fast"), ("3", "Normal"), ("5", "Good"), ("7", "Best")], "3")),
        _ => None,
    }
}

/// How a new archive is written; `None` and 0 leave it to the format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateSettings {
    pub method: Option<String>,
    pub level: Option<i32>,
    pub dict_size: u64,
    /// 0: one solid block (or the format's default); 1: every file on its own.
    pub solid_size: u64,
    pub threads: u32,
    pub force_compress: bool,
    pub encrypt_names: bool,
    /// ZIP: "aes256" or "zipcrypto".
    pub zip_encryption: Option<String>,
    /// The format's own settings by key.
    pub specific: Vec<(String, String)>,
    /// Bytes per part when the archive is split (0: one file).
    pub split: u64,
}

impl CreateSettings {
    /// The `settings` object of an `add` request.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(x) = &self.method {
            m.insert("method".into(), json!(x));
        }
        if let Some(l) = self.level {
            m.insert("level".into(), json!(l));
        }
        m.insert("dict_size".into(), json!(self.dict_size));
        m.insert("solid_size".into(), json!(self.solid_size));
        m.insert("threads".into(), json!(self.threads.max(1)));
        m.insert("force_compress".into(), json!(self.force_compress));
        m.insert("encrypt_names".into(), json!(self.encrypt_names));
        if let Some(z) = &self.zip_encryption {
            m.insert("zip_encryption".into(), json!(z));
        }
        let specific: Map<String, Value> = self.specific.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        m.insert("specific".into(), Value::Object(specific));
        Value::Object(m)
    }
}

/// Sizes to split into, as offered: (label, bytes).
pub const SPLIT_PRESETS: &[(&str, u64)] = &[
    ("10 MB", 10 * MB),
    ("100 MB", 100 * MB),
    ("650 MB (CD)", 650 * MB),
    ("700 MB (CD)", 700 * MB),
    ("1 GB", 1024 * MB),
    ("4 GB (FAT32 limit)", 4095 * MB),
    ("4.7 GB (DVD)", 4482 * MB),
    ("8.5 GB (DVD DL)", 8106 * MB),
    ("25 GB (Blu-ray)", 23866 * MB),
];

const MB: u64 = 1024 * 1024;

/// Reads a size typed in: "700 MB", "1.5G", "64k", "2048" (bytes); 1 KB = 1024 bytes.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim().to_ascii_lowercase().replace(',', ".");
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let unit = unit.trim().trim_end_matches("ib").trim_end_matches('b');
    let mult = match unit {
        "" => 1u64,
        "k" => 1 << 10,
        "m" => 1 << 20,
        "g" => 1 << 30,
        "t" => 1 << 40,
        _ => return None,
    };
    let bytes = (n * mult as f64).round();
    (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
}

/// The name of part `n` (from 1) of a split file: `x.7z.001`.
pub fn part_path(whole: &Path, n: usize) -> PathBuf {
    let mut s = whole.as_os_str().to_os_string();
    s.push(format!(".{n:03}"));
    PathBuf::from(s)
}

/// Cuts a file into parts of `size` bytes (`x.7z.001`, `.002`, …, the 7-Zip way) and removes
/// it; a file that fits into one part stays as it is. Gives the files there are afterwards.
pub fn split_file(whole: &Path, size: u64) -> io::Result<Vec<PathBuf>> {
    let len = std::fs::metadata(whole)?.len();
    if size == 0 || len <= size {
        return Ok(vec![whole.to_path_buf()]);
    }
    let mut src = std::fs::File::open(whole)?;
    let mut parts = Vec::new();
    let mut left = len;
    let result = (|| {
        while left > 0 {
            let p = part_path(whole, parts.len() + 1);
            let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&p)?;
            parts.push(p);
            let n = left.min(size);
            let copied = io::copy(&mut (&mut src).take(n), &mut out)?;
            if copied != n {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the archive got shorter while it was split"));
            }
            out.flush()?;
            left -= n;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    std::fs::remove_file(whole)?;
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rar_and_zip() -> (FormatOptions, FormatOptions) {
        let rar = FormatOptions::from_json(&json!({
            "methods": [{"id": "rar5", "name": "RAR 5"}, {"id": "rar4", "name": "RAR 4"}, {"id": "store", "name": "Store"}],
            "options": [{"key": "TargetCompatibility", "name": "Target compatibility", "kind": "enum", "default": "Rar5", "values": ["Rar5", "Rar4", "Rar1_5"]}]
        }));
        let zip = FormatOptions::from_json(&json!({
            "methods": [{"id": "deflate", "name": "Deflate"}, {"id": "lzma", "name": "LZMA"}],
            "options": [
                {"key": "Method", "name": "Compression method", "kind": "enum", "default": "deflate", "values": ["deflate", "lzma"]},
                {"key": "Level", "name": "Compression level", "kind": "integer", "default": "5", "values": ["0", "5", "9"]},
                {"key": "Password", "name": "Password", "kind": "string", "default": "", "values": []},
                {"key": "Journal", "name": "Journal", "kind": "boolean", "default": "true", "values": [], "depends": "Version=ext3|ext4"}
            ]
        }));
        (rar, zip)
    }

    #[test]
    fn settings_find_their_place() {
        let (rar, zip) = rar_and_zip();
        assert_eq!(rar.version().unwrap().key, "TargetCompatibility");
        assert_eq!(rar.method_choices("Rar"), vec![("store".to_string(), "Store".to_string())]);
        assert!(rar.advanced().is_empty());
        assert!(zip.version().is_none());
        assert_eq!(zip.level().unwrap().default, "5");
        assert_eq!(zip.advanced().iter().map(|o| o.key.as_str()).collect::<Vec<_>>(), ["Journal"]);
        assert_eq!(zip.advanced()[0].kind, OptionKind::Switch);
        let mut values = HashMap::new();
        assert!(!depends_met(zip.advanced()[0], &values));
        values.insert("Version".to_string(), "EXT4".to_string());
        assert!(depends_met(zip.advanced()[0], &values));
    }

    #[test]
    fn labels_say_what_opens_it() {
        assert_eq!(version_label("Rar", "Rar4"), "RAR 4 (WinRAR 2.9 – 4.x, also 3.5)");
        assert_eq!(version_label("Iso", "Other"), "Other");
        assert_eq!(method_label("Zip", "lzma", "LZMA"), "LZMA (ZIP 6.3)");
        assert_eq!(method_label("SevenZip", "lzma", "LZMA"), "LZMA");
        assert_eq!(level_label("9"), "9 – ultra");
        assert_eq!(value_label("DictSize", "16777216"), "16 MB");
        assert_eq!(value_label("SolidSize", "0"), "One block (fully solid)");
        assert_eq!(value_label("BlockingFactor", "20"), "20");
        assert_eq!(binary_size(1536 * 1024), "1.5 MB");
        assert_eq!(general_levels("Rar").unwrap().1, "3");
    }

    #[test]
    fn settings_become_a_request() {
        let s = CreateSettings { method: Some("lzma".into()), level: Some(9), specific: vec![("DictSize".into(), "65536".into())], ..Default::default() };
        let v = s.to_json();
        assert_eq!(v["method"], "lzma");
        assert_eq!(v["level"], 9);
        assert_eq!(v["threads"], 1);
        assert_eq!(v["specific"]["DictSize"], "65536");
        assert!(v.get("zip_encryption").is_none());
    }

    #[test]
    fn sizes_are_read_like_people_write_them() {
        assert_eq!(parse_size("700 MB"), Some(700 * MB));
        assert_eq!(parse_size("1.5G"), Some(1536 * MB));
        assert_eq!(parse_size("64 KiB"), Some(64 * 1024));
        assert_eq!(parse_size("2048"), Some(2048));
        assert_eq!(parse_size("4,7 gb"), Some((4.7 * 1024.0 * MB as f64).round() as u64));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("lots"), None);
        assert_eq!(parse_size("5 parsecs"), None);
    }

    #[test]
    fn files_split_into_numbered_parts() {
        let dir = tempfile::tempdir().unwrap();
        let whole = dir.path().join("a.7z");
        let data: Vec<u8> = (0..2500u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&whole, &data).unwrap();
        let parts = split_file(&whole, 1000).unwrap();
        assert_eq!(parts, vec![dir.path().join("a.7z.001"), dir.path().join("a.7z.002"), dir.path().join("a.7z.003")]);
        assert!(!whole.exists());
        let joined: Vec<u8> = parts.iter().flat_map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(joined, data);
        assert_eq!(std::fs::metadata(&parts[2]).unwrap().len(), 500);
        // Small enough: stays one file.
        let small = dir.path().join("b.zip");
        std::fs::write(&small, b"tiny").unwrap();
        assert_eq!(split_file(&small, 1000).unwrap(), vec![small.clone()]);
        // A part in the way: nothing is left behind, the archive stays.
        let c = dir.path().join("c.zip");
        std::fs::write(&c, &data).unwrap();
        std::fs::write(part_path(&c, 2), b"x").unwrap();
        assert!(split_file(&c, 1000).is_err());
        assert!(c.exists() && !part_path(&c, 1).exists());
    }
}

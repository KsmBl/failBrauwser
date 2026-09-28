//! Human-readable sizes, dates and permissions, and the natural name order.

use std::cmp::Ordering;

/// "0 bytes", "1 byte", "999 bytes", "1.0 kB", "12.3 MB" — decimal units like GIO/Thunar.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes == 1 {
        return "1 byte".into();
    }
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Like `ls -l`: "drwxr-xr-x".
pub fn permissions(mode: u32) -> String {
    let t = match mode & libc::S_IFMT {
        libc::S_IFDIR => 'd',
        libc::S_IFLNK => 'l',
        libc::S_IFCHR => 'c',
        libc::S_IFBLK => 'b',
        libc::S_IFIFO => 'p',
        libc::S_IFSOCK => 's',
        _ => '-',
    };
    let mut s = String::with_capacity(10);
    s.push(t);
    let bits = [
        (libc::S_IRUSR, 'r'),
        (libc::S_IWUSR, 'w'),
        (libc::S_IXUSR, 'x'),
        (libc::S_IRGRP, 'r'),
        (libc::S_IWGRP, 'w'),
        (libc::S_IXGRP, 'x'),
        (libc::S_IROTH, 'r'),
        (libc::S_IWOTH, 'w'),
        (libc::S_IXOTH, 'x'),
    ];
    for (bit, c) in bits {
        s.push(if mode & bit != 0 { c } else { '-' });
    }
    let special = |s: &mut String, idx: usize, set: bool, exec_c: char, noexec_c: char| {
        if set {
            let exec = s.as_bytes()[idx] == b'x';
            s.replace_range(idx..idx + 1, &(if exec { exec_c } else { noexec_c }).to_string());
        }
    };
    special(&mut s, 3, mode & libc::S_ISUID != 0, 's', 'S');
    special(&mut s, 6, mode & libc::S_ISGID != 0, 's', 'S');
    special(&mut s, 9, mode & libc::S_ISVTX != 0, 't', 'T');
    s
}

/// Formats a Unix time relative to `now`: "Today 14:03", "Yesterday 09:12", else a date.
pub fn human_time(unix: i64, now: i64) -> String {
    use gtk::glib::DateTime;
    let (Ok(t), Ok(n)) = (DateTime::from_unix_local(unix), DateTime::from_unix_local(now)) else {
        return String::new();
    };
    let same_day = |a: &DateTime, b: &DateTime| a.year() == b.year() && a.day_of_year() == b.day_of_year();
    let yesterday = n.add_days(-1).ok();
    let fmt = if same_day(&t, &n) {
        "Today %H:%M"
    } else if yesterday.as_ref().is_some_and(|y| same_day(&t, y)) {
        "Yesterday %H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };
    t.format(fmt).map(|s| s.to_string()).unwrap_or_default()
}

/// Case-insensitive order in which "file2" comes before "file10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 bytes");
        assert_eq!(human_size(1), "1 byte");
        assert_eq!(human_size(999), "999 bytes");
        assert_eq!(human_size(1000), "1.0 kB");
        assert_eq!(human_size(1_234_567), "1.2 MB");
        assert_eq!(human_size(999_999), "1.0 MB");
        assert_eq!(human_size(u64::MAX), "18.4 EB");
    }

    #[test]
    fn perms() {
        assert_eq!(permissions(libc::S_IFDIR | 0o755), "drwxr-xr-x");
        assert_eq!(permissions(libc::S_IFREG | 0o644), "-rw-r--r--");
        assert_eq!(permissions(libc::S_IFREG | 0o4755), "-rwsr-xr-x");
        assert_eq!(permissions(libc::S_IFDIR | 0o1777), "drwxrwxrwt");
        assert_eq!(permissions(libc::S_IFREG | 0o2644), "-rw-r-Sr--");
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "file02b", "B"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "B", "file1", "File2", "file02b", "file10"]);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
    }

    #[test]
    fn relative_times() {
        let now = gtk::glib::DateTime::now_local().unwrap().to_unix();
        assert!(human_time(now, now).starts_with("Today"));
        assert!(human_time(now - 86_400, now).starts_with("Yesterday"));
        assert!(human_time(0, now).starts_with("1970") || human_time(0, now).starts_with("1969"));
    }
}

//! The local network as the Network page shows it: the computers sharing files (found by
//! asking every address for SMB, announced by GVfs's network browsing, or opened before) and
//! the folders each one shares.

use crate::remote::{self, Host};
use gtk::glib;

/// A computer sharing files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computer {
    /// What to call it: its own name if known, else its address.
    pub name: String,
    /// What to connect to (`nas.local`, `192.168.1.198`).
    pub address: String,
    /// Its IP address, when known and different from the name.
    pub ip: Option<String>,
}

impl Computer {
    /// `smb://address/`, the folders it shares.
    pub fn uri(&self) -> String {
        format!("smb://{}/", self.address)
    }
}

/// A folder a computer shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub comment: String,
}

/// The short name of a host: `nas.fritz.box` is `nas`; addresses stay as they are.
pub fn short_name(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_string();
    }
    host.split('.').next().filter(|s| !s.is_empty()).unwrap_or(host).to_string()
}

/// One list from everything found: SMB servers that answered (with their reverse DNS names),
/// computers GVfs announces (name, address), and servers of addresses opened before. Each
/// computer once (by address or name), sorted by name.
pub fn merge(scanned: &[Host], announced: &[(String, String)], history: &[String]) -> Vec<Computer> {
    let mut out: Vec<Computer> = Vec::new();
    let mut add = |name: String, address: String, ip: Option<String>| {
        let same = |c: &Computer| {
            c.address.eq_ignore_ascii_case(&address)
                || c.ip.as_deref().is_some_and(|i| ip.as_deref() == Some(i) || i == address)
                || ip.as_deref().is_some_and(|i| i == c.address)
                || (!name.is_empty() && c.name.eq_ignore_ascii_case(&name))
        };
        match out.iter_mut().find(|c| same(c)) {
            Some(c) => {
                // A name beats an address.
                if c.name.parse::<std::net::IpAddr>().is_ok() && name.parse::<std::net::IpAddr>().is_err() && !name.is_empty() {
                    c.name = name;
                }
                if c.ip.is_none() {
                    c.ip = ip.filter(|i| *i != c.address);
                }
            }
            None => out.push(Computer { name: if name.is_empty() { address.clone() } else { name }, ip: ip.filter(|i| *i != address), address }),
        }
    };
    for (name, address) in announced {
        add(short_name(name), address.clone(), None);
    }
    for h in scanned {
        let ip = h.addr.to_string();
        match &h.name {
            Some(n) => add(short_name(n), ip.clone(), Some(ip)),
            None => add(String::new(), ip.clone(), Some(ip)),
        }
    }
    for uri in history {
        if let Some(host) = remote::smb_host(uri).or_else(|| uri.strip_prefix("smb://").map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty() && !h.contains('/'))) {
            let ip = host.parse::<std::net::IpAddr>().is_ok().then(|| host.to_string());
            add(short_name(host), host.to_string(), ip);
        }
    }
    out.sort_by(|a, b| crate::fs::format::natural_cmp(&a.name.to_lowercase(), &b.name.to_lowercase()));
    out
}

/// The host of a GVfs network entry's target (`smb://NAS/`), for announced computers.
pub fn announced_host(target_uri: &str) -> Option<String> {
    let rest = target_uri.strip_prefix("smb://")?;
    let host = rest.split('/').next()?.rsplit('@').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// Shares that are meant to be seen: not the hidden ones (`IPC$`, `C$`, `print$`).
pub fn visible_share(name: &str) -> bool {
    !name.is_empty() && !name.ends_with('$')
}

/// The shares in `smbclient -g -L` output (`Disk|name|comment` lines), folders only.
pub fn parse_smbclient(text: &str) -> Vec<Share> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '|');
            let (kind, name) = (f.next()?, f.next()?);
            (kind == "Disk" && visible_share(name)).then(|| Share { name: name.to_string(), comment: f.next().unwrap_or_default().to_string() })
        })
        .collect()
}

/// The address of a shared folder: `smb://nas/My%20Files`.
pub fn share_uri(address: &str, share: &str) -> String {
    format!("smb://{}/{}", address, glib::uri_escape_string(share, None::<&str>, false))
}

/// The names the side panel and links use for a network address: a share is "Music on nas",
/// a computer its name.
pub fn uri_name(uri: &str) -> String {
    if let Some(host) = uri.strip_prefix(crate::location::NETWORK_URI) {
        return short_name(host.trim_matches('/'));
    }
    let Some(rest) = uri.split_once("://").map(|(_, r)| r) else { return uri.to_string() };
    let rest = rest.trim_end_matches('/');
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = short_name(host.rsplit('@').next().unwrap_or(host));
    let share = path.rsplit('/').next().unwrap_or_default();
    let share = glib::uri_unescape_string(share, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| share.to_string());
    if share.is_empty() { host } else { format!("{share} on {host}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn computers_are_listed_once_by_name() {
        let scanned = [
            Host { addr: Ipv4Addr::new(192, 168, 1, 198), name: Some("nas.fritz.box".into()) },
            Host { addr: Ipv4Addr::new(192, 168, 1, 20), name: None },
        ];
        let announced = [("NAS".to_string(), "NAS.local".to_string()), ("printer-box".to_string(), "printer-box.local".to_string())];
        let history = ["smb://192.168.1.198/GamingCrypt-Drive".to_string(), "smb://192.168.1.197/Old".to_string(), "sftp://x/y".to_string()];
        let all = merge(&scanned, &announced, &history);
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["192.168.1.20", "192.168.1.197", "NAS", "printer-box"]);
        // The announced NAS and the scanned nas.fritz.box are the same computer by name.
        let nas = all.iter().find(|c| c.name == "NAS").unwrap();
        assert_eq!(nas.address, "NAS.local");
        assert_eq!(nas.ip.as_deref(), Some("192.168.1.198"));
        assert_eq!(nas.uri(), "smb://NAS.local/");
        assert_eq!(all.iter().find(|c| c.name == "192.168.1.20").unwrap().ip, None);
    }

    #[test]
    fn share_lists_and_names() {
        let out = "Disk|Music|Songs\nDisk|IPC$|IPC Service\nDisk|My Files|\nPrinter|Laser|\nWorkgroup|WORKGROUP|NAS\n";
        assert_eq!(parse_smbclient(out).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Music", "My Files"]);
        assert_eq!(parse_smbclient(out)[0].comment, "Songs");
        assert_eq!(share_uri("nas", "My Files"), "smb://nas/My%20Files");
        assert_eq!(uri_name("smb://nas.fritz.box/My%20Files"), "My Files on nas");
        assert_eq!(uri_name("smb://me@192.168.1.5/data/"), "data on 192.168.1.5");
        assert_eq!(uri_name("network:///nas"), "nas");
        assert_eq!(announced_host("smb://NAS/"), Some("NAS".into()));
        assert_eq!(announced_host("dnssd://x/"), None);
        assert!(!visible_share("print$"));
        assert_eq!(short_name("10.0.0.1"), "10.0.0.1");
    }
}

//! Network addresses offered while typing a location: the ones opened before and the SMB
//! servers answering on the local network.

use gtk::glib;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How many opened addresses are kept.
pub const HISTORY_MAX: usize = 50;
/// The SMB port (SMB over TCP, no NetBIOS).
const SMB_PORT: u16 = 445;

pub fn history_path() -> PathBuf {
    glib::user_config_dir().join("failbrauwser").join("remote-history")
}

/// The same address however it was typed: no surrounding spaces, no trailing slash.
pub fn normalize(uri: &str) -> String {
    uri.trim().trim_end_matches('/').to_string()
}

/// Opened addresses, most recent first.
pub fn load_history_from(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(normalize).filter(|l| !l.is_empty()).take(HISTORY_MAX).collect())
        .unwrap_or_default()
}

/// Puts `uri` first in the history file (once) and keeps the newest [`HISTORY_MAX`].
pub fn remember_in(path: &Path, uri: &str) -> io::Result<Vec<String>> {
    let uri = normalize(uri);
    let mut list = load_history_from(path);
    list.retain(|u| !u.eq_ignore_ascii_case(&uri));
    list.insert(0, uri);
    list.truncate(HISTORY_MAX);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, list.iter().map(|u| format!("{u}\n")).collect::<String>())?;
    Ok(list)
}

/// The server part of `smb://host/share/…`, when the text has one followed by a slash.
pub fn smb_host(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("smb://")?;
    let (host, _) = rest.split_once('/')?;
    let host = host.rsplit('@').next().unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

/// Whether a suggestion fits what is typed: from the start, or from the server name on so
/// that typing `192.168` finds `smb://192.168.1.198`.
pub fn suggestion_matches(candidate: &str, typed: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return false;
    }
    let c = candidate.to_lowercase();
    if c == typed {
        return false;
    }
    c.starts_with(&typed) || c.split_once("://").is_some_and(|(_, rest)| rest.starts_with(&typed))
}

/// An SMB server found on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub addr: Ipv4Addr,
    /// From reverse DNS, when the network has it.
    pub name: Option<String>,
}

impl Host {
    pub fn uri(&self) -> String {
        format!("smb://{}", self.addr)
    }
}

/// Addresses the kernel saw recently (`/proc/net/arp`), complete entries only.
pub fn parse_arp(text: &str) -> Vec<Ipv4Addr> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            // Flags 0x0 is an incomplete entry: nobody answered.
            (f.len() >= 4 && f[2] != "0x0").then(|| f[0].parse().ok()).flatten()
        })
        .collect()
}

/// This machine's IPv4 addresses on private networks, with their prefix length.
pub fn local_networks() -> Vec<(Ipv4Addr, u32)> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list we only read and free with freeifaddrs.
    unsafe {
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() || i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let addr = Ipv4Addr::from(u32::from_be((*(ifa.ifa_addr as *const libc::sockaddr_in)).sin_addr.s_addr));
            let mask = u32::from_be((*(ifa.ifa_netmask as *const libc::sockaddr_in)).sin_addr.s_addr);
            if addr.is_private() {
                out.push((addr, mask.count_ones()));
            }
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// The addresses worth asking: the neighbours first, then the rest of each local network.
/// Networks wider than /24 are only searched in this machine's /24.
pub fn scan_targets(networks: &[(Ipv4Addr, u32)], neighbours: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let own: Vec<Ipv4Addr> = networks.iter().map(|(a, _)| *a).collect();
    let mut out: Vec<Ipv4Addr> = Vec::new();
    let in_networks = |a: &Ipv4Addr| networks.iter().any(|(n, p)| same_net(*n, *a, (*p).max(24)));
    for a in neighbours.iter().filter(|a| in_networks(a)) {
        if !out.contains(a) && !own.contains(a) {
            out.push(*a);
        }
    }
    for (addr, prefix) in networks {
        let prefix = (*prefix).max(24);
        let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
        let base = u32::from(*addr) & mask;
        for host in 1..(!mask) {
            let a = Ipv4Addr::from(base | host);
            if !out.contains(&a) && !own.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

fn same_net(a: Ipv4Addr, b: Ipv4Addr, prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(a) & mask == u32::from(b) & mask
}

/// The name reverse DNS gives an address, if any.
fn reverse_name(addr: Ipv4Addr) -> Option<String> {
    // SAFETY: plain C call on a stack sockaddr_in and buffer.
    unsafe {
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_addr.s_addr = u32::from(addr).to_be();
        let mut buf = [0 as libc::c_char; 256];
        let r = libc::getnameinfo(
            &sa as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            buf.as_mut_ptr(),
            buf.len() as libc::socklen_t,
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        );
        if r != 0 {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
        Some(name.trim_end_matches('.').to_string()).filter(|n| !n.is_empty())
    }
}

/// Asks every address on the local networks whether it takes SMB connections. Blocks for
/// about a second (many addresses are tried at once).
pub fn find_smb_servers() -> Vec<Host> {
    let neighbours = std::fs::read_to_string("/proc/net/arp").map(|t| parse_arp(&t)).unwrap_or_default();
    let targets = scan_targets(&local_networks(), &neighbours);
    let timeout = Duration::from_millis(400);
    let found: Vec<Ipv4Addr> = std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .chunks(targets.len().div_ceil(64).max(1))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .copied()
                        .filter(|a| TcpStream::connect_timeout(&SocketAddr::from((*a, SMB_PORT)), timeout).is_ok())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    std::thread::scope(|s| {
        let handles: Vec<_> = found.iter().map(|a| s.spawn(move || Host { addr: *a, name: reverse_name(*a) })).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}


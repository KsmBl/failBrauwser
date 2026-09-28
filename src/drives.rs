//! Drives and volumes: from UDisks2 (what `udisksctl` and every desktop use) or, without it,
//! from the kernel's mount table. Plus fill levels via `statvfs`.

use gtk::glib;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Volume {
    /// `/dev/sda1`
    pub device: String,
    pub label: String,
    /// ext4, vfat, ntfs, crypto_LUKS, …
    pub fs_type: String,
    pub size: u64,
    pub mount_points: Vec<PathBuf>,
    /// Model of the drive it is on ("SanDisk Ultra").
    pub drive_name: String,
    pub removable: bool,
    pub usb: bool,
    pub optical: bool,
    pub ejectable: bool,
    /// UDisks2 object paths, for mounting and ejecting.
    pub block_object: String,
    pub drive_object: String,
    /// Has a filesystem UDisks2 can mount.
    pub mountable: bool,
    /// A system volume (root, boot, …) as UDisks2 sees it.
    pub system: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

impl Usage {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.used as f64 / self.total as f64 }
    }
}

impl Volume {
    /// What to call it: label, else drive model plus size, else device.
    pub fn title(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        if self.mount_points.iter().any(|m| m == Path::new("/")) {
            return "File System".into();
        }
        if self.size > 0 {
            let size = crate::fs::format::human_size(self.size);
            if !self.drive_name.is_empty() {
                return format!("{size} {}", self.drive_name);
            }
            return format!("{size} Volume");
        }
        self.device.clone()
    }

    pub fn icon_names(&self) -> Vec<&'static str> {
        if self.optical {
            vec!["drive-optical", "media-optical", "drive-harddisk"]
        } else if self.usb {
            vec!["drive-removable-media-usb", "drive-removable-media", "drive-harddisk-usb", "drive-harddisk"]
        } else if self.removable {
            vec!["drive-removable-media", "media-flash", "drive-harddisk"]
        } else {
            vec!["drive-harddisk", "drive-harddisk-solidstate"]
        }
    }

    pub fn mount_point(&self) -> Option<&PathBuf> {
        self.mount_points.first()
    }
}

/// Fill level of a mounted filesystem.
pub fn usage(mount_point: &Path) -> Option<Usage> {
    let c = CString::new(mount_point.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let f = st.f_frsize as u64;
    let total = st.f_blocks as u64 * f;
    Some(Usage { total, used: (st.f_blocks as u64 - st.f_bfree as u64) * f, free: st.f_bavail as u64 * f })
}

fn bytes_to_string(v: &glib::Variant) -> String {
    // UDisks2 byte strings are NUL terminated.
    let bytes: Vec<u8> = v.fixed_array::<u8>().map(|b| b.to_vec()).unwrap_or_default();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

type Props = HashMap<String, glib::Variant>;

fn props(v: &glib::Variant) -> Props {
    let mut m = HashMap::new();
    for i in 0..v.n_children() {
        let entry = v.child_value(i);
        let key: String = entry.child_value(0).get().unwrap_or_default();
        let val = entry.child_value(1);
        let val = val.as_variant().unwrap_or(val);
        m.insert(key, val);
    }
    m
}

fn s(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.get::<String>()).unwrap_or_default()
}

fn b(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false)
}

fn u(p: &Props, k: &str) -> u64 {
    p.get(k).and_then(|v| v.get::<u64>()).unwrap_or(0)
}

fn o(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.str().map(str::to_string)).unwrap_or_default()
}

/// Reads the answer of UDisks2's `GetManagedObjects` (`(a{oa{sa{sv}}})`).
pub fn parse_udisks(reply: &glib::Variant) -> Vec<Volume> {
    let objects = if reply.type_().as_str().starts_with('(') { reply.child_value(0) } else { reply.clone() };
    // object path → interface name → properties
    let mut all: HashMap<String, HashMap<String, Props>> = HashMap::new();
    for i in 0..objects.n_children() {
        let entry = objects.child_value(i);
        let path = entry.child_value(0).str().unwrap_or_default().to_string();
        let ifaces = entry.child_value(1);
        let mut map = HashMap::new();
        for j in 0..ifaces.n_children() {
            let ie = ifaces.child_value(j);
            let name: String = ie.child_value(0).get().unwrap_or_default();
            map.insert(name, props(&ie.child_value(1)));
        }
        all.insert(path, map);
    }
    let mut out = Vec::new();
    for (path, ifaces) in &all {
        let Some(block) = ifaces.get("org.freedesktop.UDisks2.Block") else { continue };
        let usage = s(block, "IdUsage");
        let fs = ifaces.get("org.freedesktop.UDisks2.Filesystem");
        if usage != "filesystem" && usage != "crypto" {
            continue;
        }
        let mount_points: Vec<PathBuf> = fs
            .and_then(|f| f.get("MountPoints"))
            .map(|mp| (0..mp.n_children()).map(|i| PathBuf::from(bytes_to_string(&mp.child_value(i)))).filter(|p| !p.as_os_str().is_empty()).collect())
            .unwrap_or_default();
        // Hidden by policy (EFI partitions, recovery…): still shown while mounted.
        if b(block, "HintIgnore") && mount_points.is_empty() {
            continue;
        }
        let drive_object = o(block, "Drive");
        let drive = all.get(&drive_object).and_then(|d| d.get("org.freedesktop.UDisks2.Drive"));
        let (drive_name, removable, usb, optical, ejectable) = match drive {
            Some(d) => {
                let name = [s(d, "Vendor"), s(d, "Model")].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" ");
                let media: String = s(d, "Media");
                (name, b(d, "Removable") || b(d, "MediaRemovable"), s(d, "ConnectionBus") == "usb", b(d, "Optical") || media.starts_with("optical"), b(d, "Ejectable"))
            }
            None => (String::new(), false, false, false, false),
        };
        // Loop devices only matter while something is mounted from them.
        let device = block.get("PreferredDevice").or_else(|| block.get("Device")).map(bytes_to_string).unwrap_or_default();
        if device.starts_with("/dev/loop") && mount_points.is_empty() {
            continue;
        }
        out.push(Volume {
            device,
            label: s(block, "IdLabel"),
            fs_type: s(block, "IdType"),
            size: u(block, "Size"),
            mount_points,
            drive_name,
            removable,
            usb,
            optical,
            ejectable,
            block_object: path.clone(),
            drive_object,
            mountable: fs.is_some(),
            system: b(block, "HintSystem"),
        });
    }
    sort_volumes(&mut out);
    out
}

/// Fixed first (root first), then removable; by device name within.
pub fn sort_volumes(v: &mut [Volume]) {
    v.sort_by(|a, b| {
        let root = |x: &Volume| !x.mount_points.iter().any(|m| m == Path::new("/"));
        (root(a), a.removable || a.usb, &a.device).cmp(&(root(b), b.removable || b.usb, &b.device))
    });
}

/// Unescapes the octal escapes of /proc/self/mountinfo ("\040" is a space).
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(((b[i + 1] - b'0') << 6) | ((b[i + 2] - b'0') << 3) | (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Volumes from the mount table (without UDisks2): real block devices only.
pub fn parse_mountinfo(text: &str) -> Vec<Volume> {
    let mut by_dev: HashMap<String, Volume> = HashMap::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 5 || r.len() < 2 {
            continue;
        }
        let (mount, fs_type, source) = (unescape_mount(l[4]), r[0], unescape_mount(r[1]));
        if !source.starts_with("/dev/") || source.starts_with("/dev/loop") {
            continue;
        }
        let v = by_dev.entry(source.clone()).or_insert_with(|| Volume {
            device: source.clone(),
            fs_type: fs_type.to_string(),
            mountable: true,
            ..Default::default()
        });
        v.mount_points.push(PathBuf::from(mount));
    }
    let mut v: Vec<Volume> = by_dev.into_values().collect();
    for vol in &mut v {
        vol.mount_points.sort_by_key(|p| p.as_os_str().len());
    }
    sort_volumes(&mut v);
    v
}

pub fn read_mountinfo() -> Vec<Volume> {
    std::fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_real_devices_only() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 259:1 / /boot rw - vfat /dev/nvme0n1p1 rw
24 22 0:21 / /proc rw - proc proc rw
25 22 8:17 / /run/media/me/My\\040Stick rw - exfat /dev/sdb1 rw
26 22 7:0 / /snap/x rw - squashfs /dev/loop0 ro
27 22 259:2 /home /bind rw - ext4 /dev/nvme0n1p2 rw
";
        let v = parse_mountinfo(text);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].device, "/dev/nvme0n1p2");
        assert_eq!(v[0].mount_points, vec![PathBuf::from("/"), PathBuf::from("/bind")]);
        assert!(v.iter().any(|x| x.mount_points == vec![PathBuf::from("/run/media/me/My Stick")]));
    }

    #[test]
    fn udisks_reply_is_parsed() {
        let text = "({objectpath '/org/freedesktop/UDisks2/block_devices/sdb1': {'org.freedesktop.UDisks2.Block': {'Device': <b'/dev/sdb1'>, 'IdLabel': <'STICK'>, 'IdType': <'vfat'>, 'IdUsage': <'filesystem'>, 'Size': <uint64 8000000000>, 'HintIgnore': <false>, 'HintSystem': <false>, 'Drive': <objectpath '/org/freedesktop/UDisks2/drives/SanDisk'>}, 'org.freedesktop.UDisks2.Filesystem': {'MountPoints': <[b'/run/media/me/STICK']>}}, objectpath '/org/freedesktop/UDisks2/drives/SanDisk': {'org.freedesktop.UDisks2.Drive': {'Vendor': <'SanDisk'>, 'Model': <'Ultra'>, 'Removable': <true>, 'ConnectionBus': <'usb'>, 'Ejectable': <true>, 'Optical': <false>, 'Media': <''>}}, objectpath '/org/freedesktop/UDisks2/block_devices/sda': {'org.freedesktop.UDisks2.Block': {'Device': <b'/dev/sda'>, 'IdUsage': <''>, 'HintIgnore': <false>}}, objectpath '/org/freedesktop/UDisks2/block_devices/loop0': {'org.freedesktop.UDisks2.Block': {'Device': <b'/dev/loop0'>, 'IdUsage': <'filesystem'>, 'HintIgnore': <false>}, 'org.freedesktop.UDisks2.Filesystem': {'MountPoints': <@aay []>}}},)";
        let v = glib::Variant::parse(Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()), text).unwrap();
        let vols = parse_udisks(&v);
        assert_eq!(vols.len(), 1);
        let s = &vols[0];
        assert_eq!(s.device, "/dev/sdb1");
        assert_eq!(s.title(), "STICK");
        assert_eq!(s.mount_points, vec![PathBuf::from("/run/media/me/STICK")]);
        assert!(s.usb && s.removable && s.ejectable && s.mountable);
        assert_eq!(s.drive_name, "SanDisk Ultra");
        assert_eq!(s.icon_names()[0], "drive-removable-media-usb");
    }

    #[test]
    fn titles_and_usage() {
        let mut v = Volume { device: "/dev/sdc1".into(), size: 32_000_000_000, drive_name: "Kingston".into(), ..Default::default() };
        assert_eq!(v.title(), "32.0 GB Kingston");
        v.mount_points = vec!["/".into()];
        assert_eq!(v.title(), "File System");
        let u = usage(Path::new("/")).unwrap();
        assert!(u.total > 0 && u.used <= u.total);
        assert!((0.0..=1.0).contains(&u.fraction()));
        assert!(usage(Path::new("/definitely/not/mounted")).is_none());
    }
}

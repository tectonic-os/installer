use super::*;
use std::io::Write as _;

pub(crate) fn partitions(disk: &str) -> Result<Vec<Partition>, String> {
    let out = Command::new("lsblk")
        .args([
            "--json",
            "--paths",
            "--output",
            "NAME,SIZE,FSTYPE,LABEL,TYPE,PARTTYPE,UUID",
            disk,
        ])
        .output()
        .map_err(|err| format!("lsblk {disk}: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    partition_rows(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn partition_rows(raw: &str) -> Result<Vec<Partition>, String> {
    fn visit(node: &Json, found: &mut Vec<Partition>) {
        if json::text(node, "type").as_deref() == Some("part") {
            found.push(Partition {
                device: json::text(node, "name").unwrap_or_default(),
                size: json::text(node, "size").unwrap_or_default(),
                fstype: json::text(node, "fstype").unwrap_or_default(),
                label: json::text(node, "label").unwrap_or_default(),
                parttype: json::text(node, "parttype").unwrap_or_default(),
                uuid: json::text(node, "uuid").unwrap_or_default(),
            });
        }
        for child in json::items(node, "children") {
            visit(child, found);
        }
    }

    let doc = Json::parse(raw).map_err(|err| format!("lsblk wrote invalid JSON: {err}"))?;
    let mut found = Vec::new();
    for block in json::items(&doc, "blockdevices") {
        visit(block, &mut found);
    }
    Ok(found)
}

/// Holds one old system the walk mounted and read. The mount stays up while
/// the keys its crypttab names are read.
#[derive(Debug)]
pub(crate) struct OldSystem {
    pub(crate) at: PathBuf,
    pub(crate) crypttab: String,
    pub(crate) fstab: String,
}

/// Names where one crypttab line says a container's key is. No key is read
/// to decide it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum KeySource {
    /// Holds a path inside the old system that names the volume.
    Inside(PathBuf),
    /// Takes the systemd default, `/etc/cryptsetup-keys.d/<name>.key` inside
    /// the old root.
    Default,
    /// Holds a path on another filesystem. The old system names that
    /// filesystem by an fstab-style device spec, and the walk mounts it
    /// read-only.
    OnDevice { device: String, path: PathBuf },
    /// Holds a path the old system waits for at boot under
    /// `keyfile-timeout=`. Removable media carries a key this way.
    Waited,
    /// Names a keyscript this installer cannot run.
    Unreadable,
}

/// Holds one line of an old system's `/etc/crypttab`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Crypttab {
    /// Names the container as the old system opens it. The old fstab refers
    /// to it as `/dev/mapper/<name>`.
    pub(crate) name: String,
    /// Names the container as the crypttab line spells it.
    pub(crate) device: String,
    pub(crate) key: KeySource,
}

fn device_spec(said: &str) -> bool {
    said.starts_with("/dev/")
        || ["UUID=", "PARTUUID=", "LABEL="]
            .iter()
            .any(|prefix| said.starts_with(prefix))
}

/// Reads a `path:device` third field, where the path is relative to that
/// device's filesystem root. Debian's `passdev` keyscript writes the two
/// halves the other way round, as `device:path[:timeout]`.
fn on_device(said: &str, passdev: bool) -> Option<KeySource> {
    let (first, rest) = said.split_once(':')?;
    let (device, path, timed) = match (device_spec(first), passdev) {
        (true, _) => (first, rest, true),
        (false, false) if device_spec(rest) => (rest, first, false),
        _ => return None,
    };
    let path = match timed {
        true => path.split_once(':').map_or(path, |(path, _timeout)| path),
        false => path,
    };
    Some(KeySource::OnDevice {
        device: device.to_string(),
        path: PathBuf::from(path),
    })
}

/// Reads the lines of `/etc/crypttab` as the volumes they open and where
/// their keys are. A line naming a mechanism this installer cannot read
/// keeps its volume name, so the refusal can say which volume it was.
pub(crate) fn crypttabs(raw: &str) -> Vec<Crypttab> {
    let mut found = Vec::new();
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let (Some(name), Some(device)) = (words.next(), words.next()) else {
            continue;
        };
        let field = words.next().unwrap_or("none");
        let options = words.next().unwrap_or("");
        let keyscript = options
            .split(',')
            .find_map(|option| option.strip_prefix("keyscript="));
        // systemd waits at boot for the key file to appear under this
        // option. Removable media carries a key that way.
        let timed = options
            .split(',')
            .any(|option| option.starts_with("keyfile-timeout="));
        let key = match keyscript {
            Some(script) if script.rsplit('/').next() == Some("passdev") => {
                on_device(field, true).unwrap_or(KeySource::Unreadable)
            }
            // Any other keyscript derives its key in a way this installer
            // cannot repeat. The third field is not a path to read.
            Some(_) => KeySource::Unreadable,
            None if field == "none" || field == "-" => KeySource::Default,
            None => match on_device(field, false) {
                Some(device) => device,
                None if timed => KeySource::Waited,
                None => KeySource::Inside(PathBuf::from(field)),
            },
        };
        found.push(Crypttab {
            name: name.to_string(),
            device: device.to_string(),
            key,
        });
    }
    found
}

/// Whether one crypttab line names this container.
pub(crate) fn names(entry: &Crypttab, container: &Partition) -> bool {
    let said = entry.device.as_str();
    if let Some(uuid) = said.strip_prefix("UUID=") {
        return !container.uuid.is_empty() && uuid.eq_ignore_ascii_case(&container.uuid);
    }
    if let Some(uuid) = said.strip_prefix("/dev/disk/by-uuid/") {
        return !container.uuid.is_empty() && uuid.eq_ignore_ascii_case(&container.uuid);
    }
    said == container.device
}

/// Reads where the old system mounted a container, by the mapper name its
/// crypttab gave it. An old fstab that does not say returns empty.
fn fstab_target(raw: &str, name: &str) -> String {
    let mapper = format!("/dev/mapper/{name}");
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let (Some(device), Some(target)) = (words.next(), words.next()) else {
            continue;
        };
        if device == mapper {
            return target.to_string();
        }
    }
    String::new()
}

/// Holds the walk's own mounts. `/run` is RAM, so reading an old system
/// writes to no disk. `$TECT_MOUNT_ROOT` overrides the path, for the same
/// reason `$TECT_SYS_BLOCK` does. The drawn golden and a developer who is
/// not root have no `/run` of their own.
const MOUNTS: &str = "/run/tect-old";

fn mounts_root() -> PathBuf {
    std::env::var_os("TECT_MOUNT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(MOUNTS))
}

/// Holds every mount the walk made and unmounts them when it drops. No
/// reader keeps a mount afterwards, so a failed key unmounts everything and
/// a panic does the same.
#[derive(Default)]
pub(crate) struct Mounts(Vec<PathBuf>);

impl Drop for Mounts {
    fn drop(&mut self) {
        for at in &self.0 {
            let _ = Command::new("umount").arg(at).status();
        }
    }
}

/// Chooses the options that keep a read of an old filesystem from writing to
/// it. A read-only mount of a journaling filesystem still replays its
/// journal, so the four filesystems the editor knows take `norecovery`. A
/// filesystem with no journal mounts plain.
pub(crate) fn mount_options(fstype: &str) -> &'static str {
    match LINUX_FILESYSTEMS.contains(&fstype) {
        true => "ro,norecovery",
        false => "ro",
    }
}

fn mount_ro(device: &Path, fstype: &str, mounts: &mut Mounts) -> Result<PathBuf, String> {
    let at = mounts_root().join(format!("old-{}", mounts.0.len() + 1));
    std::fs::create_dir_all(&at).map_err(|err| format!("{}: {err}", at.display()))?;
    let options = mount_options(fstype);
    let out = Command::new("mount")
        .args(["-o", options])
        .arg(device)
        .arg(&at)
        .output()
        .map_err(|err| format!("mount: {err}, and it is what reads {}", device.display()))?;
    if !out.status.success() {
        return Err(format!(
            "mounting {} read-only: {}",
            device.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mounts.0.push(at.clone());
    Ok(at)
}

/// Reads the filesystems the walk may take an old system from. The opened
/// containers and logical volumes above the editor's partitions count too,
/// which `lsblk` types as `crypt` and `lvm`.
pub(crate) fn mountable_rows(raw: &str) -> Result<Vec<(String, String)>, String> {
    fn visit(node: &Json, found: &mut Vec<(String, String)>) {
        let kind = json::text(node, "type").unwrap_or_default();
        let fstype = json::text(node, "fstype").unwrap_or_default();
        if ["part", "crypt", "lvm"].contains(&kind.as_str())
            && !fstype.is_empty()
            && fstype != "crypto_LUKS"
        {
            found.push((json::text(node, "name").unwrap_or_default(), fstype));
        }
        for child in json::items(node, "children") {
            visit(child, found);
        }
    }
    let doc = Json::parse(raw).map_err(|err| format!("lsblk wrote invalid JSON: {err}"))?;
    let mut found = Vec::new();
    for block in json::items(&doc, "blockdevices") {
        visit(block, &mut found);
    }
    Ok(found)
}

fn mountables(disk: &str) -> Result<Vec<(String, String)>, String> {
    let out = Command::new("lsblk")
        .args(["--json", "--paths", "--output", "NAME,FSTYPE,TYPE", disk])
        .output()
        .map_err(|err| format!("lsblk {disk}: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mountable_rows(&String::from_utf8_lossy(&out.stdout))
}

/// Mounts every filesystem the disk has and keeps the ones carrying
/// `/etc/fstab`, which is what makes one an old system. A failed mount is
/// reported rather than passed over. A disk the walk could not read must not
/// look like a disk with nothing on it.
fn old_systems(disk: &str, mounts: &mut Mounts) -> (Vec<OldSystem>, Vec<String>) {
    let mut systems = Vec::new();
    let mut unread = Vec::new();
    match mountables(disk) {
        Ok(devices) => {
            for (device, fstype) in devices {
                let at = match mount_ro(Path::new(&device), &fstype, mounts) {
                    Ok(at) => at,
                    Err(err) => {
                        unread.push(err);
                        continue;
                    }
                };
                let fstab = at.join("etc/fstab");
                if !fstab.is_file() {
                    continue;
                }
                // Each file is opened only where it is a regular file. A
                // hostile old system otherwise names a FIFO, and the walk
                // blocks on it with nothing drawn.
                let crypttab = at.join("etc/crypttab");
                systems.push(OldSystem {
                    crypttab: match crypttab.is_file() {
                        true => std::fs::read_to_string(&crypttab).unwrap_or_default(),
                        false => String::new(),
                    },
                    fstab: std::fs::read_to_string(&fstab).unwrap_or_default(),
                    at,
                });
            }
        }
        Err(err) => unread.push(err),
    }
    (systems, unread)
}

/// Holds one key an old system carries and the mount point it used.
#[derive(Debug, PartialEq)]
pub(crate) struct OldKey {
    /// Names the mount point the old fstab gives it. An old fstab that says
    /// nothing leaves it empty.
    pub(crate) target: String,
    /// Holds the key. The walk proved it against the container.
    pub(crate) key: Key,
}

/// Holds what the walk came back with. Each container carries either a
/// proved key or the reason the walk has none for it.
#[derive(Default)]
pub(crate) struct Discovered {
    pub(crate) found: Vec<(String, OldKey)>,
    pub(crate) why: Vec<(String, String)>,
}

impl Discovered {
    /// Returns the key an old system holds for one container.
    pub(crate) fn key(&self, partition: &str) -> Option<&Key> {
        self.found
            .iter()
            .find(|(device, _)| device == partition)
            .map(|(_, old)| &old.key)
    }
}

/// Reads one key file the old system names, only where that old system holds
/// it. The path resolves under the mount, and a symlink or `..` may not leave
/// it. Only a regular file is read, so a hostile old system cannot name
/// `/etc/shadow`, or a FIFO that blocks the installer, and have it opened.
pub(crate) fn key_file(volume: &str, root: &Path, path: &Path) -> Result<Vec<u8>, String> {
    let said = root.join(path.strip_prefix("/").unwrap_or(path));
    let file = std::fs::canonicalize(&said).map_err(|err| {
        copy::old_root_key_missing(volume, &path.display().to_string(), &err.to_string())
    })?;
    let at = std::fs::canonicalize(root).map_err(|err| format!("{}: {err}", root.display()))?;
    if !file.starts_with(&at) || !file.is_file() {
        return Err(copy::old_root_key_outside(&path.display().to_string()));
    }
    std::fs::read(&file).map_err(|err| {
        copy::old_root_key_missing(volume, &path.display().to_string(), &err.to_string())
    })
}

/// Reads the bytes of one key where the old system says they are. A key on
/// removable media is refused, and so is a key the old system waits for at
/// boot. The user keeps that media apart from the machine, and copying the
/// bytes onto the root would silently join the two. A key on a fixed second
/// device is read, because it sits on the machine already and the key
/// ladder's top rung carries it to the new root.
fn key_bytes(system: &OldSystem, entry: &Crypttab, mounts: &mut Mounts) -> Result<Vec<u8>, String> {
    match &entry.key {
        KeySource::Inside(path) => key_file(&entry.name, &system.at, path),
        KeySource::Default => key_file(
            &entry.name,
            &system.at,
            &PathBuf::from(format!("/etc/cryptsetup-keys.d/{}.key", entry.name)),
        ),
        KeySource::OnDevice { device, path } => {
            let resolved = resolve_device(device)?;
            if removable(&resolved) {
                return Err(copy::old_root_key_removable(&entry.name));
            }
            let fstype = fstype_of(&resolved);
            let at = mount_ro(&resolved, &fstype, mounts)?;
            key_file(&entry.name, &at, path)
        }
        KeySource::Waited => Err(copy::old_root_key_waited(&entry.name)),
        KeySource::Unreadable => Err(copy::old_root_key_unreadable(&entry.name)),
    }
}

fn resolve_device(said: &str) -> Result<PathBuf, String> {
    if said.starts_with('/') {
        return Ok(PathBuf::from(said));
    }
    let Some((tag, value)) = ["UUID", "LABEL", "PARTUUID"].iter().find_map(|tag| {
        said.strip_prefix(&format!("{tag}="))
            .map(|value| (*tag, value))
    }) else {
        return Err(format!("{said} is not a device"));
    };
    let out = Command::new("blkid")
        .args(["-t", &format!("{tag}={value}"), "-o", "device"])
        .output()
        .map_err(|err| format!("blkid: {err}, and it is what resolves {said}"))?;
    labelled(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{said} is not present"))
}

/// Reads the filesystem a device carries. A `blkid` that does not say
/// returns empty, and the mount then adds no `norecovery`.
fn fstype_of(device: &Path) -> String {
    Command::new("blkid")
        .args(["-s", "TYPE", "-o", "value"])
        .arg(device)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Holds the kernel's per-device flags. The `removable` flag sits on the
/// disk, and a partition's directory sits one level below it.
const SYS_CLASS_BLOCK: &str = "/sys/class/block";

/// Whether a resolved device is removable media. A partition carries
/// `partition` in its own sysfs directory and the flag on the disk above it.
/// A device the kernel does not answer for counts as fixed, which keeps the
/// key it holds readable. The device canonicalizes first, because a udev name
/// such as `/dev/disk/by-uuid/…` is the kernel's node under another name and
/// sysfs is looked up by the kernel's.
pub(crate) fn removable_at(class: &Path, device: &Path) -> bool {
    let device = std::fs::canonicalize(device).unwrap_or_else(|_| device.to_path_buf());
    let Some(name) = device.file_name() else {
        return false;
    };
    let Ok(at) = std::fs::canonicalize(class.join(name)) else {
        return false;
    };
    let disk = match at.join("partition").exists() {
        true => match at.parent() {
            Some(parent) => parent.to_path_buf(),
            None => at,
        },
        false => at,
    };
    std::fs::read_to_string(disk.join("removable")).is_ok_and(|flag| flag.trim() == "1")
}

pub(crate) fn removable(device: &Path) -> bool {
    removable_at(Path::new(SYS_CLASS_BLOCK), device)
}

/// Whether a key opens a container, asked without opening it. `cryptsetup`
/// tests the key and sets up no mapper, so a key that fails leaves nothing
/// behind. A `cryptsetup` that cannot run at all returns an error, which
/// tells the user something different from a wrong key.
pub(crate) fn test_key(container: &str, key: &[u8]) -> Result<bool, String> {
    let mut command = Command::new("cryptsetup");
    command
        .args(["-q", "luksOpen", "--test-passphrase", "--key-file", "-"])
        .arg(container)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what tests a key"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(key);
    }
    let status = child.wait().map_err(|err| format!("cryptsetup: {err}"))?;
    Ok(status.success())
}

/// Proves each key the old systems hold against the editor's container it
/// opens. `opens` does the proving, so an install passes `cryptsetup` and a
/// test passes a closure.
pub(crate) fn discover(
    partitions: &[Partition],
    systems: &[OldSystem],
    unread: &[String],
    mounts: &mut Mounts,
    opens: &dyn Fn(&Partition, &[u8]) -> Result<bool, String>,
) -> Discovered {
    let mut found = Discovered::default();
    for partition in partitions
        .iter()
        .filter(|part| part.fstype == "crypto_LUKS")
    {
        let mut named = false;
        let mut why = None;
        let mut key = None;
        'systems: for system in systems {
            for entry in crypttabs(&system.crypttab) {
                if !names(&entry, partition) {
                    continue;
                }
                named = true;
                match key_bytes(system, &entry, mounts)
                    .and_then(|bytes| opens(partition, &bytes).map(|opens| (bytes, opens)))
                {
                    Ok((bytes, true)) => {
                        key = Some(OldKey {
                            target: fstab_target(&system.fstab, &entry.name),
                            key: Key::Data(bytes),
                        });
                        break 'systems;
                    }
                    Ok((_, false)) => why = Some(copy::old_root_key_wrong(&partition.device)),
                    Err(err) => why = Some(err),
                }
            }
        }
        match key {
            Some(key) => found.found.push((partition.device.clone(), key)),
            None => {
                let why = why.unwrap_or_else(|| match named {
                    // A named container always set `why` above, so this arm
                    // never runs. It repeats the wrong-key reason rather
                    // than claiming no crypttab named the container.
                    true => copy::old_root_key_wrong(&partition.device),
                    // An unread device is named before a read system,
                    // because the unread device may hold the key.
                    false if !unread.is_empty() => copy::old_root_partly(&unread[0]),
                    // A system was read and names no key for this
                    // container. Saying nothing was read would be false.
                    false if !systems.is_empty() => copy::old_root_unnamed(&partition.device),
                    false => copy::OLD_ROOT_NONE.to_string(),
                });
                found.why.push((partition.device.clone(), why));
            }
        }
    }
    found
}

/// Runs the whole walk over one disk. A disk with no LUKS container is left
/// alone, so nothing is mounted.
pub(crate) fn old_keys(disk: &str, partitions: &[Partition]) -> Discovered {
    if !partitions.iter().any(|part| part.fstype == "crypto_LUKS") {
        return Discovered::default();
    }
    let mut mounts = Mounts::default();
    let (systems, unread) = old_systems(disk, &mut mounts);
    discover(
        partitions,
        &systems,
        &unread,
        &mut mounts,
        &|container, key| test_key(&container.device, key),
    )
}

/// Stops the machine from the installer's screen. The live environment runs
/// systemd, which is what put the installer on a console.
pub(crate) fn power_off() -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("poweroff")
        .output()
        .map_err(|err| format!("systemctl: {err}, and it is what stops a machine"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "systemctl poweroff: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Whether systemd started this process under one of the media's installer
/// units. An installer started by hand from a getty, a serial console or the
/// handed-over shell has no console of its own to leave. `Switch to shell`
/// then means the shell the user ran it from.
pub(crate) fn under_media_installer() -> bool {
    std::fs::read_to_string("/proc/self/cgroup")
        .map(|cgroup| {
            cgroup.contains("tect-installer.service")
                || cgroup.contains("tect-installer-vt.service")
        })
        .unwrap_or(false)
}

/// Reads from the kernel which VT the installer is on. `copy::switch_note`
/// prints that number as the key the user presses to come back, so a
/// constant would misname it.
pub(crate) fn console_vt() -> Option<u32> {
    let active = std::fs::read_to_string(tty0_active()).ok()?;
    active.trim().strip_prefix("tty")?.parse().ok()
}

/// Switches to another VT, where the media autologins root. The installer
/// keeps running on its own VT, so the note's key brings the user back to the
/// form where they left it.
pub(crate) fn switch_to_shell() -> Result<(), String> {
    // The media autologins root on every getty, and its installer unit owns
    // tty1. The media's own documentation names `Ctrl-Alt-F2`.
    let shell = match console_vt() {
        Some(1) | None => 2,
        Some(vt) => {
            if vt == 2 {
                3
            } else {
                2
            }
        }
    };
    let out = Command::new("chvt")
        .arg(shell.to_string())
        .output()
        .map_err(|err| format!("chvt: {err}, and it is what switches to the shell"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "chvt: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

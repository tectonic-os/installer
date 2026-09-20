use super::*;
use std::io::{Read as _, Write as _};

/// Where `write_etc` mounts the installed root while it writes the key files,
/// the crypttab and the enrolment units.
const ROOT_MOUNT: &str = "/run/tect-root";

/// Use this x86-64 root partition type when the custom-layout path lacks one.
const ROOT_GUID: &str = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";

/// One file `write_etc` writes under the installed system's own `/etc`. A key
/// file takes mode 0600 and a systemd unit takes 0644.
struct EtcWrite {
    pub(crate) at: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) mode: u32,
}

/// Writes how the installed machine opens every container the layout opened,
/// and the one addition the user chose. It runs after fisherman and before the
/// boot chain is configured, because the BLS options the menu bakes in take
/// the LUKS argument written here. A root container cannot boot without it.
/// Nothing else writes the initrd argument for a layout the user chose, and
/// nothing else retags the root for the sealed UKI.
pub(crate) fn arrange(payload: &Payload, answers: &Answers) -> Result<Vec<String>, String> {
    let Some(layout) = &answers.layout else {
        return Ok(Vec::new());
    };
    if layout.opens.is_empty() {
        return Ok(Vec::new());
    }
    let mut writes: Vec<EtcWrite> = Vec::new();
    let mut crypttab: Vec<(String, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut root: Option<(String, String)> = None;
    let encrypted_root = layout.opens_the_root();
    let pcr_policy = answers.opened == Opened::Tpm2 && pcr_policy_in(&payload.image);
    let planned = (|| {
        for (mapper, open) in layout.mappers() {
            let uuid = luks_uuid(&open.partition)?;
            if open.target == "/" {
                // A root takes no key file, because the file would live on
                // the filesystem the key opens. A sealed UKI finds the root
                // by partition type. Every other boot chain is handed the
                // container on the kernel command line.
                match payload.boot.is_empty() {
                    true => root = Some((uuid.clone(), "root".to_string())),
                    false => retag_root(&answers.disk, open, &mapper)?,
                }
            } else {
                // A key written to an unencrypted root sits readable beside
                // the volume it opens. That holds for a boot key file and for
                // the key a first-boot TPM2 enrolment is staged with. The key
                // question and the editor rows keep those answers out. This
                // branch refuses an answer that reached here anyway.
                if !encrypted_root
                    && (answers.opened != Opened::Keep || !matches!(open.key, Key::Passphrase(_)))
                {
                    return Err(copy::OPENED_KEYFILE_PLAIN.to_string());
                }
                let name = crypttab_name(&open.target);
                let (line, note) =
                    data_volume(&name, open, &uuid, answers.opened, &mut writes, &mut added)?;
                if let Some(note) = note {
                    notes.extend(note);
                }
                crypttab.push((name, line));
            }
            if answers.opened == Opened::Tpm2 {
                let name = match open.target.as_str() {
                    "/" => "root".to_string(),
                    target => crypttab_name(target),
                };
                stage_tpm2(&name, open, &uuid, pcr_policy, &mut writes)?;
            }
        }
        // A layout that opens only the root, with no TPM2 answer, leaves no
        // file and no crypttab line to write. Mounting the root to find a
        // deployment would then refuse an install that had no work to do.
        if !writes.is_empty() || !crypttab.is_empty() {
            write_etc(layout, &writes, &crypttab)?;
        }
        if let Some((uuid, name)) = root {
            inject_luks_args(&answers.disk, &uuid, &name)?;
        }
        Ok(())
    })();
    if let Err(err) = planned {
        return Err(naming_added(err, &added));
    }
    Ok(notes)
}

/// A failure after a key was added names the container. The new slot is in
/// the LUKS header, and the key file that would have used it may not be
/// written, so the user opens that container with the key it had before.
fn naming_added(err: String, added: &[String]) -> String {
    match added.is_empty() {
        true => err,
        false => format!(
            "{err}; a key was added to {} and the install did not finish",
            added.join(", ")
        ),
    }
}

/// The name the installed machine opens a data volume as, taken from where
/// the volume mounts. `/var` becomes `var`, which is the name its key file
/// takes too.
fn crypttab_name(target: &str) -> String {
    target.trim_start_matches('/').replace('/', "-")
}

/// One data volume's crypttab line and the key it opens from. The installed
/// machine carries the old system's key file over, or asks the user for a
/// passphrase at boot, or uses a key this install added so it needs no
/// passphrase.
fn data_volume(
    name: &str,
    open: &LuksOpen,
    uuid: &str,
    opened: Opened,
    writes: &mut Vec<EtcWrite>,
    added: &mut Vec<String>,
) -> Result<(String, Option<Vec<String>>), String> {
    let path = format!("/etc/cryptsetup-keys.d/{name}.key");
    match (&open.key, opened) {
        (Key::Passphrase(_), Opened::AddKey) => {
            let before = slots(&open.partition)?.keys;
            let key = random_key()?;
            add_key(open, &key)?;
            if !test_key(&open.partition, &key)? {
                return Err(copy::added_key_wrong(&open.partition));
            }
            // The container is recorded before the note is built, because a
            // header that already carries the new key must be named by any
            // failure after this point.
            added.push(open.partition.clone());
            writes.push(EtcWrite {
                at: path.clone(),
                bytes: key,
                mode: 0o600,
            });
            let note = redundant_slot_note(&open.partition, &before)?;
            Ok((copy::crypttab_line(name, uuid, Some(&path)), Some(note)))
        }
        (Key::Passphrase(_), _) => Ok((copy::crypttab_line(name, uuid, None), None)),
        (key, _) => {
            writes.push(EtcWrite {
                at: path.clone(),
                bytes: key_bytes_of(key)?,
                mode: 0o600,
            });
            Ok((copy::crypttab_line(name, uuid, Some(&path)), None))
        }
    }
}

/// The bytes of a key, however the editor holds it. A key file the live
/// system read is read again here, because the installed machine will not
/// carry the path it came from.
fn key_bytes_of(key: &Key) -> Result<Vec<u8>, String> {
    match key {
        Key::Passphrase(passphrase) => Ok(passphrase.as_bytes().to_vec()),
        Key::Data(bytes) => Ok(bytes.clone()),
        Key::File(path) => std::fs::read(path).map_err(|err| format!("{}: {err}", path.display())),
    }
}

/// What the last screen owes the user where a key was added. The note names
/// the slot the installed machine no longer needs, the slot count the header
/// now holds, and the `luksKillSlot` command. The installer states that
/// command and never runs it, because the old system still opens the volume
/// with its own key until the user decides otherwise.
fn redundant_slot_note(partition: &str, before: &[u32]) -> Result<Vec<String>, String> {
    let after = slots(partition)?;
    let count = after.keys.len();
    Ok(match before {
        [only] => copy::kill_slot(partition, *only, count),
        _ => copy::kill_a_slot(partition, count),
    })
}

/// Stages the first-boot enrolment a TPM2 answer asks for. The installed
/// system's PCRs are not the live environment's, so the enrolment cannot run
/// here. The key that opens the container is written beside the unit, and the
/// unit shreds that key once the TPM2 token is in the header. `pcr_policy`
/// says the image carries a signed PCR 11 policy, and the first boot then
/// binds to that policy as well as to PCR 7. Without the policy the unit
/// binds to PCR 7 alone.
fn stage_tpm2(
    name: &str,
    open: &LuksOpen,
    uuid: &str,
    pcr_policy: bool,
    writes: &mut Vec<EtcWrite>,
) -> Result<(), String> {
    let key = format!("/etc/tect/tpm2-enroll-{name}.key");
    writes.push(EtcWrite {
        at: key.clone(),
        bytes: key_bytes_of(&open.key)?,
        mode: 0o600,
    });
    writes.push(EtcWrite {
        at: format!("/etc/systemd/system/tect-tpm2-enroll-{name}.service"),
        bytes: copy::tpm2_unit(name, &key, uuid, pcr_policy).into_bytes(),
        mode: 0o644,
    });
    Ok(())
}

/// Writes the staged files into the deployment's own `/etc`. ostree
/// three-way-merges that directory against `/usr/etc` on every upgrade, so a
/// key file and a crypttab written here survive each upgrade. The mount is
/// released on every return from this function.
fn write_etc(
    layout: &CustomLayout,
    writes: &[EtcWrite],
    crypttab: &[(String, String)],
) -> Result<(), String> {
    let Some(device) = root_device(layout) else {
        return Err("the layout names no / to write into".to_string());
    };
    let at = PathBuf::from(ROOT_MOUNT);
    std::fs::create_dir_all(&at).map_err(|err| format!("{ROOT_MOUNT}: {err}"))?;
    let mounted = Command::new("mount")
        .arg(&device)
        .arg(&at)
        .output()
        .map_err(|err| format!("mount: {err}, and it is what holds the installed root"))?;
    if !mounted.status.success() {
        return Err(format!(
            "mounting {device} at {ROOT_MOUNT}: {}",
            String::from_utf8_lossy(&mounted.stderr).trim()
        ));
    }
    let written = (|| {
        let etc = deployment_etc(&at)?;
        for write in writes {
            let path = etc.join(write.at.trim_start_matches('/'));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("{}: {err}", parent.display()))?;
            }
            std::fs::write(&path, &write.bytes)
                .map_err(|err| format!("{}: {err}", path.display()))?;
            set_mode(&path, write.mode)?;
        }
        merge_crypttab(&etc, crypttab)?;
        // The unit is enabled by writing the symlink systemd itself would
        // write. `systemctl --root` would look in the physical root's `/etc`,
        // and on bootc the deployment's `/etc` is a different directory.
        for write in writes {
            let Some(unit) = write.at.rsplit('/').next() else {
                continue;
            };
            if !unit.ends_with(".service") {
                continue;
            }
            let wants = etc
                .join("systemd/system/multi-user.target.wants")
                .join(unit);
            if let Some(parent) = wants.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("{}: {err}", parent.display()))?;
            }
            let _ = std::fs::remove_file(&wants);
            std::os::unix::fs::symlink(format!("/etc/systemd/system/{unit}"), &wants)
                .map_err(|err| format!("{}: {err}", wants.display()))?;
        }
        Ok(())
    })();
    let _ = Command::new("umount").arg(&at).output();
    written
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// The writable `/etc` of the deployment just installed. ostree keeps it
/// under `ostree/deploy/<os>/deploy/<name>/etc`, and the composefs backend
/// keeps it under `state/deploy/<name>/etc`. A fresh install carries exactly
/// one, so a root with none and a root with several are both refused instead
/// of guessed at.
pub(crate) fn deployment_etc(root: &Path) -> Result<PathBuf, String> {
    let mut found = Vec::new();
    if let Ok(stateroots) = std::fs::read_dir(root.join("ostree/deploy")) {
        for stateroot in stateroots.flatten() {
            if let Ok(deployments) = std::fs::read_dir(stateroot.path().join("deploy")) {
                for deployment in deployments.flatten() {
                    let etc = deployment.path().join("etc");
                    if etc.is_dir() {
                        found.push(etc);
                    }
                }
            }
        }
    }
    if let Ok(deployments) = std::fs::read_dir(root.join("state/deploy")) {
        for deployment in deployments.flatten() {
            let etc = deployment.path().join("etc");
            if etc.is_dir() {
                found.push(etc);
            }
        }
    }
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!(
            "{}: no installed deployment carries an /etc",
            root.display()
        )),
        many => Err(format!(
            "{}: {} deployments carry an /etc, so which one to write is not clear",
            root.display(),
            many.len()
        )),
    }
}

/// The installed system's crypttab. The file keeps the lines already in it,
/// drops any line naming a volume this install decides, and gains this
/// install's lines. The volume name is the first field, which is what systemd
/// keys a crypttab entry by.
pub(crate) fn merge_crypttab(etc: &Path, lines: &[(String, String)]) -> Result<(), String> {
    let at = etc.join("crypttab");
    let held = std::fs::read_to_string(&at).unwrap_or_default();
    let mut out: Vec<&str> = held
        .lines()
        .filter(|line| {
            let name = line.split_whitespace().next().unwrap_or("");
            !lines.iter().any(|(ours, _)| ours == name)
        })
        .collect();
    let ours: Vec<&str> = lines.iter().map(|(_, line)| line.as_str()).collect();
    out.extend(ours);
    if out.is_empty() {
        return Ok(());
    }
    std::fs::write(&at, format!("{}\n", out.join("\n")))
        .map_err(|err| format!("{}: {err}", at.display()))
}

/// The device the installed root is on. An opened `/` gives its mapper, and a
/// plain mount answer at `/` gives its partition.
pub(crate) fn root_device(layout: &CustomLayout) -> Option<String> {
    if let Some((name, _)) = layout
        .mappers()
        .into_iter()
        .find(|(_, open)| open.target == "/")
    {
        return Some(mapper_path(&name));
    }
    layout
        .mounts
        .iter()
        .find(|mount| mount.target == "/")
        .map(|mount| mount.partition.clone())
}

/// The LUKS UUID a container's header carries, which is how the installed
/// machine names the container. The crypttab line writes it as `UUID=` and the
/// TPM2 unit as `/dev/disk/by-uuid/`. Both forms survive the device
/// renumbering between the live environment and the installed system.
fn luks_uuid(partition: &str) -> Result<String, String> {
    let out = Command::new("cryptsetup")
        .args(["luksUUID", partition])
        .output()
        .map_err(|err| format!("cryptsetup: {err}, and it is what names a container"))?;
    let uuid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.success() && !uuid.is_empty() {
        true => Ok(uuid),
        false => Err(format!(
            "cryptsetup luksUUID {partition}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// One key added to a container, authenticated with the key that already
/// opens it. The new key is passed in a file only `cryptsetup` reads.
/// `data_volume` tests the new key before it treats the addition as done.
fn add_key(open: &LuksOpen, key: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    // `create_new` fails if the path already exists, and `mode` applies from
    // the file's first instant, so no other user can read the key at any
    // moment.
    let at = std::env::temp_dir().join(format!(
        "tect-luks-key.{}.{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&at)
        .map_err(|err| format!("{}: {err}", at.display()))?;
    // `create_new` above proves this process made the key file, so the guard
    // can own it. The guard is taken before the first write, because a write
    // that fails part-way leaves a prefix of the key on disk. The guard is a
    // value rather than a later statement, because a panic unwinds past a
    // statement.
    let at = Staged(at);
    file.write_all(key)
        .map_err(|err| format!("{}: {err}", at.0.display()))?;
    drop(file);
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksAddKey"]);
    match &open.key {
        Key::File(path) => {
            command.arg("--key-file").arg(path);
        }
        _ => {
            command.args(["--key-file", "-"]);
        }
    }
    command.arg(&open.partition).arg(&at.0);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    if open.key.bytes().is_some() {
        command.stdin(Stdio::piped());
    }
    let added = (|| {
        let mut child = command
            .spawn()
            .map_err(|err| format!("cryptsetup: {err}, and it is what adds a key"))?;
        if let Some(bytes) = open.key.bytes() {
            child
                .stdin
                .take()
                .ok_or("cryptsetup: no stdin")?
                .write_all(bytes)
                .map_err(|err| format!("cryptsetup: {err}"))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|err| format!("cryptsetup: {err}"))?;
        match out.status.success() {
            true => Ok(()),
            false => Err(format!(
                "adding a key to {}: {}",
                open.partition,
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    })();
    added
}

/// A new key that opens a container with no user present. The key is 32
/// random bytes written as hex, read from the kernel and taken from no
/// dependency.
fn random_key() -> Result<Vec<u8>, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|err| format!("/dev/urandom: {err}"))?;
    let mut said = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        said.push_str(&format!("{byte:02x}"));
    }
    Ok(said.into_bytes())
}

/// Sets an opened root container's partition type to `ROOT_GUID`, so the
/// sealed UKI's initrd finds it. dm-crypt holds the partition busy, so this
/// closes the container around the table write and opens it again with the
/// same key.
fn retag_root(disk: &str, open: &LuksOpen, mapper: &str) -> Result<(), String> {
    let number = partition_number(&open.partition)?;
    close_volume(mapper)?;
    let out = Command::new("sfdisk")
        .args(["--part-type", disk, &number, ROOT_GUID])
        .output()
        .map_err(|err| format!("sfdisk: {err}, and it is what retags a root"))?;
    if !out.status.success() {
        return Err(format!(
            "retagging {} as the root partition: {}",
            open.partition,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    open_volume(open, mapper)
}

/// The GPT number of a partition device, taken as the trailing digits.
/// `/dev/vda2`, `/dev/nvme0n1p2` and `/dev/mmcblk0p2` all give 2.
pub(crate) fn partition_number(device: &str) -> Result<String, String> {
    let number: String = device
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    match number.is_empty() {
        true => Err(format!("{device} names no partition number")),
        false => Ok(number),
    }
}

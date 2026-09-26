use super::*;
use std::io::{Read as _, Write as _};

/// Use this x86-64 root partition type when the custom-layout path lacks one.
pub(crate) const ROOT_GUID: &str = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";

/// One file `write_deployment` writes under the installed system's own `/etc`. A key
/// file takes mode 0600 and a systemd unit takes 0644.
struct EtcWrite {
    pub(crate) at: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) mode: u32,
}

/// Writes the account and every installed-system file derived from the disk
/// layout. It runs before `Prepared` drops the target mounts.
pub(crate) fn arrange(
    payload: &Payload,
    answers: &Answers,
    layout: &CustomLayout,
    password_hash: &str,
) -> Result<Vec<String>, String> {
    let mut writes: Vec<EtcWrite> = Vec::new();
    let mut crypttab: Vec<(String, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let encrypted_root = layout.opens_the_root();
    let created_tpm = answers.encryption.kind.starts_with("tpm2-");
    let opened_tpm = answers.opened == Opened::Tpm2;
    let pcr_policy = (created_tpm || opened_tpm) && pcr_policy_in(&payload.image);
    let planned = (|| {
        for (_, open) in layout.mappers() {
            let uuid = luks_uuid(&open.partition)?;
            if open.target != "/" {
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
            let created = layout
                .creates
                .iter()
                .any(|create| create.encrypt && create.device == open.partition);
            if (created && created_tpm) || (!created && opened_tpm) {
                let name = match open.target.as_str() {
                    "/" => "root".to_string(),
                    target => crypttab_name(target),
                };
                let pin = (created && Encryption::wants_pin(&answers.encryption.kind))
                    .then_some(answers.encryption.pin.as_str());
                stage_tpm2(&name, open, &uuid, pcr_policy, pin, &mut writes)?;
            }
        }
        write_deployment(payload, answers, layout, password_hash, &writes, &crypttab)?;
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
/// the volume mounts. The key file takes the same name.
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
            add_key(open, &key, free_slot(&before)?)?;
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
    pin: Option<&str>,
    writes: &mut Vec<EtcWrite>,
) -> Result<(), String> {
    let key = format!("/etc/tect/tpm2-enroll-{name}.key");
    writes.push(EtcWrite {
        at: key.clone(),
        bytes: key_bytes_of(&open.key)?,
        mode: 0o600,
    });
    let pin_path = pin.map(|pin| {
        let path = format!("/etc/tect/tpm2-enroll-{name}.pin");
        writes.push(EtcWrite {
            at: path.clone(),
            bytes: pin.as_bytes().to_vec(),
            mode: 0o600,
        });
        path
    });
    writes.push(EtcWrite {
        at: format!("/etc/systemd/system/tect-tpm2-enroll-{name}.service"),
        bytes: copy::tpm2_unit(name, &key, pin_path.as_deref(), uuid, pcr_policy).into_bytes(),
        mode: 0o644,
    });
    Ok(())
}

/// Writes all post-install state into the deployment and labels every changed
/// `/etc` path with the target's own SELinux policy.
fn write_deployment(
    payload: &Payload,
    answers: &Answers,
    layout: &CustomLayout,
    password_hash: &str,
    writes: &[EtcWrite],
    crypttab: &[(String, String)],
) -> Result<(), String> {
    let root = Path::new(SYSROOT);
    let (deployment, composefs) = deployment(root)?;
    let etc = deployment.join("etc");
    let mut changed = configure(
        root,
        &deployment,
        composefs,
        answers,
        &payload.install.groups,
        password_hash,
    )?;
    for write in writes {
        let relative = write
            .at
            .strip_prefix("/etc/")
            .expect("EtcWrite paths live under /etc");
        let path = etc.join(relative);
        if let Some(parent) = path.parent().filter(|parent| !parent.exists()) {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
            // A directory this install creates takes the live environment's
            // label unless the target's policy relabels it.
            changed.push(parent.to_path_buf());
        }
        std::fs::write(&path, &write.bytes).map_err(|err| format!("{}: {err}", path.display()))?;
        set_mode(&path, write.mode)?;
        changed.push(path);
    }
    if !crypttab.is_empty() {
        merge_crypttab(&etc, crypttab)?;
        changed.push(etc.join("crypttab"));
    }
    // `systemctl --root` would look in the physical root's `/etc`, while a
    // bootc deployment keeps its writable `/etc` below the deployment.
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
        match std::fs::remove_file(&wants) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("{}: {err}", wants.display())),
        }
        std::os::unix::fs::symlink(format!("/etc/systemd/system/{unit}"), &wants)
            .map_err(|err| format!("{}: {err}", wants.display()))?;
        changed.push(wants);
    }
    if let Some(device) = swap_device(layout) {
        let uuid = filesystem_uuid(&device)?;
        merge_fstab(&etc, &uuid)?;
        changed.push(etc.join("fstab"));
    }
    changed.sort();
    changed.dedup();
    label_paths(&deployment, &changed)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Finds the one deployment bootc just installed and whether it is composefs.
/// A root with none or several is refused instead of guessed at.
pub(crate) fn deployment(root: &Path) -> Result<(PathBuf, bool), String> {
    let mut found = Vec::new();
    if let Ok(stateroots) = std::fs::read_dir(root.join("ostree/deploy")) {
        for stateroot in stateroots.flatten() {
            if let Ok(deployments) = std::fs::read_dir(stateroot.path().join("deploy")) {
                for entry in deployments.flatten() {
                    let deployment = entry.path();
                    if deployment.join("etc").is_dir() {
                        found.push((deployment, false));
                    }
                }
            }
        }
    }
    if let Ok(deployments) = std::fs::read_dir(root.join("state/deploy")) {
        for entry in deployments.flatten() {
            let deployment = entry.path();
            if deployment.join("etc").is_dir() {
                found.push((deployment, true));
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

pub(crate) fn swap_device(layout: &CustomLayout) -> Option<String> {
    volumes(layout, "")
        .into_iter()
        .find(|volume| volume.target == "/swap")
        .map(|volume| volume.device)
}

fn filesystem_uuid(device: &str) -> Result<String, String> {
    let out = Command::new("blkid")
        .args(["-s", "UUID", "-o", "value", device])
        .output()
        .map_err(|err| format!("blkid: {err}, and it is what names swap {device}"))?;
    let uuid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.success() && !uuid.is_empty() {
        true => Ok(uuid),
        false => Err(format!(
            "blkid could not name swap {device}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn merge_fstab(etc: &Path, uuid: &str) -> Result<(), String> {
    let at = etc.join("fstab");
    let source = format!("UUID={uuid}");
    let held = match std::fs::read_to_string(&at) {
        Ok(held) => held,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("{}: {err}", at.display())),
    };
    let mut lines: Vec<&str> = held
        .lines()
        .filter(|line| line.split_whitespace().next() != Some(source.as_str()))
        .collect();
    let ours = format!("{source} none swap defaults 0 0");
    lines.push(&ours);
    std::fs::write(&at, format!("{}\n", lines.join("\n")))
        .map_err(|err| format!("{}: {err}", at.display()))
}

/// Applies the target's own SELinux policy instead of the live environment's
/// policy. A target without SELinux needs no labels.
pub(crate) fn label_paths(deployment: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let Some(contexts) = target_contexts(deployment)? else {
        return Ok(());
    };
    let out = Command::new("setfiles")
        .arg("-r")
        .arg(deployment)
        .arg(&contexts)
        .args(paths)
        .output()
        .map_err(|err| format!("setfiles: {err}, and it is what labels the installed system"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "setfiles with {}: {}",
            contexts.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn target_contexts(deployment: &Path) -> Result<Option<PathBuf>, String> {
    let config = deployment.join("etc/selinux/config");
    if !config.is_file() {
        return Ok(None);
    }
    let raw =
        std::fs::read_to_string(&config).map_err(|err| format!("{}: {err}", config.display()))?;
    let policy = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix("SELINUXTYPE="))
        .map(|value| value.trim_matches(|character| matches!(character, '\'' | '"')))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{}: no SELINUXTYPE", config.display()))?;
    let contexts = deployment
        .join("etc/selinux")
        .join(policy)
        .join("contexts/files/file_contexts");
    if !contexts.is_file() {
        return Err(format!(
            "{}: missing target file contexts",
            contexts.display()
        ));
    }
    Ok(Some(contexts))
}

/// The LUKS UUID a container's header carries, which is how the installed
/// machine names the container. The crypttab line writes it as `UUID=` and the
/// TPM2 unit as `/dev/disk/by-uuid/`. Both forms survive the device
/// renumbering between the live environment and the installed system.
pub(crate) fn luks_uuid(partition: &str) -> Result<String, String> {
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
/// opens it. The new key is passed in a file only `cryptsetup` reads, and the
/// slot is explicit, so a caller can name the key it added. `data_volume`
/// tests the new key before it treats the addition as done.
pub(crate) fn add_key(open: &LuksOpen, key: &[u8], slot: u32) -> Result<(), String> {
    let at = staged_key(key)?;
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksAddKey"]);
    command.arg(format!("--key-slot={slot}"));
    command.args(auth_args(&open.key));
    command.arg(&open.partition).arg(&at.0);
    run_with_key(command, &open.key, "adds a key")
        .map_err(|why| format!("adding a key to {}: {why}", open.partition))
}

/// Names the first key slot a container can take. LUKS2 stops at slot 31, and
/// an explicit slot is what lets a caller name the key it added even when the
/// header cannot be read back.
pub(crate) fn free_slot(keys: &[u32]) -> Result<u32, String> {
    const SLOTS: u32 = 32;
    (0..SLOTS)
        .find(|at| !keys.contains(at))
        .ok_or_else(|| format!("all {SLOTS} key slots are in use"))
}

/// Builds `cryptsetup`'s arguments that authenticate one header operation.
/// A passphrase and a data volume's key both arrive on stdin, so neither
/// reaches the process list. A key file is named by its path, which
/// `cryptsetup` reads itself.
pub(crate) fn auth_args(key: &Key) -> Vec<std::ffi::OsString> {
    match key {
        Key::File(path) => vec!["--key-file".into(), path.as_os_str().to_os_string()],
        _ => vec!["--key-file".into(), "-".into()],
    }
}

/// Runs a `cryptsetup` command that authenticates with `key`. The calling
/// command has already built the argument vector, and `what` completes the
/// sentence a spawn failure prints.
pub(crate) fn run_with_key(mut command: Command, key: &Key, what: &str) -> Result<(), String> {
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    if key.bytes().is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what {what}"))?;
    if let Some(bytes) = key.bytes() {
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
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

pub(crate) struct StagedKey(pub(crate) PathBuf);

impl Drop for StagedKey {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.0) {
            eprintln!("{PROGRAM}: removing {}: {err}", self.0.display());
        }
    }
}

/// Writes one key to a file only its reader can open. A secret that reaches
/// the disk must live under a path the process owns and removes.
pub(crate) fn staged_key(key: &[u8]) -> Result<StagedKey, String> {
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
    let at = StagedKey(at);
    file.write_all(key)
        .map_err(|err| format!("{}: {err}", at.0.display()))?;
    drop(file);
    Ok(at)
}

/// A new key that opens a container with no user present. The key is 32
/// random bytes written as hex, read from the kernel and taken from no
/// dependency.
pub(crate) fn random_key() -> Result<Vec<u8>, String> {
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
pub(crate) fn retag_root(layout: &CustomLayout) -> Result<(), String> {
    let opened = layout
        .mappers()
        .into_iter()
        .find(|(_, open)| open.target == "/");
    let partition = opened
        .as_ref()
        .map(|(_, open)| open.partition.as_str())
        .or_else(|| {
            layout
                .mounts
                .iter()
                .find(|mount| mount.target == "/")
                .map(|mount| mount.partition.as_str())
        })
        .or_else(|| {
            layout
                .creates
                .iter()
                .find(|create| create.target == "/")
                .map(|create| create.device.as_str())
        })
        .ok_or("the layout names no root partition")?;
    let number = partition_number(partition)?.to_string();
    if let Some((mapper, _)) = &opened {
        close_volume(mapper)?;
    }
    let out = Command::new("sfdisk")
        .args(["--part-type", &layout.disk, &number, ROOT_GUID])
        .output()
        .map_err(|err| format!("sfdisk: {err}, and it is what retags a root"))?;
    let tagged = match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "retagging {partition} as the root partition: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    };
    let reopened = match opened {
        Some((mapper, open)) => open_volume(open, &mapper),
        None => Ok(()),
    };
    match (tagged, reopened) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(tagged), Ok(())) => Err(tagged),
        (Ok(()), Err(opened)) => Err(opened),
        (Err(tagged), Err(opened)) => Err(format!("{tagged}; and {opened}")),
    }
}

/// The GPT number of a partition device, taken as the trailing digits.
/// `/dev/vda2`, `/dev/nvme0n1p2` and `/dev/mmcblk0p2` all give 2.
///
/// This answers a number rather than the digits it read, because every caller
/// but two needs a number and the two that format it back are naming an
/// `sfdisk` argument. While the digits and the number were separate steps the
/// callers disagreed about which devices were numberable: the readers in
/// `table.rs` dropped an entry whose digits overflow `usize` and `apply_cuts`
/// accepted it, so a delete the editor screen had ignored was cut anyway and
/// `sfdisk` refused it part way through the plan.
pub(crate) fn partition_number(device: &str) -> Result<usize, String> {
    let digits: String = device
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    digits
        .parse()
        .map_err(|_| format!("{device} names no partition number"))
}

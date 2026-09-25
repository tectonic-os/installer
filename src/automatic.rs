//! The automatic action the completion screen offers: a one-time key sealed
//! into an ESP credential, so the first boot of the installed machine asks
//! for nothing. The first-boot enrolment unit kills the one-time slot and
//! removes the credential once the TPM2 token is in.

use super::*;

/// Names the public key the image's own boot/uki build wrote for the signed
/// PCR 11 policy. The seal is that policy, so the credential decrypts only
/// under a boot of the same signed UKI.
const PCR_POLICY: &str = "/usr/share/secureboot/pcr-policy.pem";

/// Names the credential `systemd-cryptsetup` asks for and the directory
/// `systemd-stub` packs from the ESP into the initrd.
pub(crate) const CREDENTIAL: &str = "cryptsetup.passphrase";
pub(crate) const CREDENTIAL_DIR: &str = "loader/credentials";

/// Names the file the first-boot unit reads the one-time slot from. Its
/// presence means the automatic action ran and its cleanup is owed.
pub(crate) const SLOT_FILE: &str = "cryptsetup.slot";

/// Holds an install the automatic action can finish.
pub(crate) struct Ready {
    pub(crate) image: String,
    /// Names the disk whose boot partition receives the credential.
    pub(crate) disk: String,
    /// Names the encrypted root the one-time key is added to.
    pub(crate) partition: String,
    /// Opens that root, and authenticates the one-time addition.
    pub(crate) key: Key,
    /// States that the root's token needs a PIN at every later unlock, so the
    /// window the credential opens states the PIN it does not ask for.
    pub(crate) pin: bool,
}

/// Names what the completion screen offers.
pub(crate) enum Offered {
    /// The automatic action can finish this install.
    Ready(Ready),
    /// A first-boot enrolment is staged and the automatic action cannot carry
    /// it. The screen draws the action dim with this reason.
    Unavailable(String),
    /// This install stages no first-boot enrolment, so there is nothing to
    /// finalize and the screen offers the restart alone.
    None,
}

/// Decides what the completion screen offers. Every refusal here is a fact
/// the screen states, rather than a failure the user meets after choosing.
pub(crate) fn offered(payload: &Payload, answers: &Answers, recovery: Option<&str>) -> Offered {
    let Some((key, pin)) = unlocking_key(answers, recovery) else {
        return Offered::None;
    };
    // The credential reaches the initrd through systemd-stub, which only the
    // signed UKI chains boot through.
    if payload.boot.is_empty() {
        return Offered::Unavailable(copy::AUTO_NO_STUB.to_string());
    }
    let partition = match encrypted_root(answers) {
        Ok(partition) => partition,
        Err(why) => return Offered::Unavailable(why),
    };
    if !pcr_policy_in(&payload.image) {
        return Offered::Unavailable(copy::AUTO_NO_POLICY.to_string());
    }
    Offered::Ready(Ready {
        image: payload.image.clone(),
        disk: answers.disk.clone(),
        partition,
        key,
        pin,
    })
}

/// Holds the key that authenticates the one-time addition and whether the
/// token asks for a PIN later. `None` means this install stages no first-boot
/// enrolment, which is what the answer kinds and the layout row decide.
pub(crate) fn unlocking_key(answers: &Answers, recovery: Option<&str>) -> Option<(Key, bool)> {
    if let Some(layout) = &answers.layout {
        if answers.opened != Opened::Tpm2 {
            return None;
        }
        let open = layout.opens.iter().find(|open| open.target == "/")?;
        return Some((open.key.clone(), false));
    }
    match answers.encryption.kind.as_str() {
        // fisherman generates the recovery key of both kinds and reports it
        // once, on the completion screen.
        "tpm2-luks" | "tpm2-luks-pin" => recovery.map(|key| {
            (
                Key::Passphrase(key.to_string()),
                answers.encryption.kind.ends_with("pin"),
            )
        }),
        "tpm2-luks-passphrase" => Some((
            Key::Passphrase(answers.encryption.passphrase.clone()),
            false,
        )),
        _ => None,
    }
}

/// Names the encrypted root the one-time key is added to. A layout names its
/// root outright. The automatic layout's root is the partition fisherman
/// retags for the sealed UKI's GPT auto-discovery.
pub(crate) fn encrypted_root(answers: &Answers) -> Result<String, String> {
    if let Some(layout) = &answers.layout {
        let open = layout
            .opens
            .iter()
            .find(|open| open.target == "/")
            .ok_or_else(|| copy::AUTO_NO_ROOT.to_string())?;
        return Ok(open.partition.clone());
    }
    let listed = partitions(&answers.disk)?;
    let found = partitions_of_type(&listed, ROOT_GUID);
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(copy::AUTO_NO_ROOT.to_string()),
        many => Err(format!(
            "{} root partitions are on {}: {}",
            many.len(),
            answers.disk,
            many.join(", ")
        )),
    }
}

/// Reads one disk's partition names and GPT types.
fn partitions(disk: &str) -> Result<String, String> {
    let out = Command::new("lsblk")
        .args(["-rno", "NAME,PARTTYPE"])
        .arg(disk)
        .output()
        .map_err(|err| format!("lsblk: {err}, and it is what names a partition"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk {disk}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Reads one partition type's devices out of an `lsblk NAME,PARTTYPE`
/// listing. The compare ignores case, because util-linux and the disk agree
/// on the identifier and not always on its case.
pub(crate) fn partitions_of_type(listed: &str, kind: &str) -> Vec<String> {
    listed
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let found = words.next().unwrap_or("");
            found
                .eq_ignore_ascii_case(kind)
                .then(|| format!("/dev/{name}"))
        })
        .collect()
}

/// Writes the automatic action's two changes: a one-time key on the root and
/// the credential on the ESP that opens it on the first boot. The image's own
/// sealing tool runs, so the credential format matches the initrd that reads
/// it. The key's slot is chosen here, so every failure after the addition can
/// remove that one slot again.
pub(crate) fn finalize(ready: &Ready) -> Result<(), String> {
    require_credential_tool(&ready.image)?;
    let before = slots(&ready.partition)?.keys;
    let slot = free_slot(&before)?;
    let key = random_key()?;
    let open = LuksOpen {
        partition: ready.partition.clone(),
        target: "/".to_string(),
        key: ready.key.clone(),
    };
    add_key(&open, &key, slot)?;
    let tested = test_key(&ready.partition, &key);
    if !matches!(tested, Ok(true)) {
        let why = match tested {
            Ok(_) => copy::added_key_wrong(&ready.partition),
            Err(why) => why,
        };
        return Err(discarded(ready, slot, why));
    }
    with_esp(&ready.disk, |esp| credential(&ready.image, esp, &key, slot))
        .map_err(|why| discarded(ready, slot, why))
}

/// Refuses an image with no `systemd-creds` before a key is added. The tool
/// seals the one-time key, and the image's own systemd matches the initrd
/// that decrypts it.
fn require_credential_tool(image: &str) -> Result<(), String> {
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
        ])
        .arg(image)
        .args(["/usr/bin/test", "-x", "/usr/bin/systemd-creds"])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what checks {image} for systemd-creds"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} has no /usr/bin/systemd-creds, so the one-time key cannot be sealed for the first boot"
    ))
}

/// Removes the one-time slot after a failed finalize, and names it when the
/// removal fails too. A failed finalize must not leave a slot whose key
/// reached no file.
fn discarded(ready: &Ready, slot: u32, why: String) -> String {
    discarded_message(slot, why, kill_slot(ready, slot))
}

/// States what a failed finalize left behind. The one-time slot has to be
/// named when its removal also fails, so the user knows which slot to wipe.
pub(crate) fn discarded_message(slot: u32, why: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => why,
        Err(cleanup) => format!("{why}; and slot {slot} was left behind: {cleanup}"),
    }
}

/// Builds the removal of one keyslot, authenticated with the key that opens
/// the container.
pub(crate) fn kill_slot_command(ready: &Ready, slot: u32) -> Command {
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksKillSlot"]);
    command.args(auth_args(&ready.key));
    command.arg(&ready.partition).arg(slot.to_string());
    command
}

/// Removes one keyslot, authenticated with the key that opens the container.
fn kill_slot(ready: &Ready, slot: u32) -> Result<(), String> {
    run_with_key(kill_slot_command(ready, slot), &ready.key, "removes a key")
        .map_err(|why| format!("removing slot {slot} from {}: {why}", ready.partition))
}

/// Reports whether a mounted partition holds the boot files `systemd-stub`
/// reads. A credential on a partition the stub did not load from never
/// reaches the initrd.
pub(crate) fn carries_boot_files(at: &Path) -> bool {
    at.join("loader/entries").is_dir() || at.join("EFI/Linux").is_dir()
}

/// Mounts the ESP carrying the installed boot files and runs `write` over its
/// mount point. The search takes a partition whose GPT type is the ESP type,
/// because only the partition `systemd-stub` loaded from reaches the initrd;
/// a content match on another filesystem would write a credential nothing
/// reads. Every return unmounts what it mounted, and a failed write is
/// reported even when the unmount also fails.
fn with_esp<T>(disk: &str, write: impl FnOnce(&Path) -> Result<T, String>) -> Result<T, String> {
    let listed = partitions(disk)?;
    let mut mounts = Mounts::default();
    for device in partitions_of_type(&listed, copy::ESP_TYPE) {
        let Ok(at) = mount_rw(&device, &mut mounts) else {
            continue;
        };
        if !carries_boot_files(&at) {
            mounts.release(&at)?;
            continue;
        }
        let written = write(&at);
        let released = mounts.release(&at);
        return match (written, released) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(why), Ok(())) => Err(why),
            (Ok(_), Err(unmount)) => Err(unmount),
            (Err(why), Err(unmount)) => Err(format!("{why}; and {unmount}")),
        };
    }
    Err(format!(
        "{disk} carries no EFI system partition with the installed boot files"
    ))
}

/// Writes the sealed one-time key and the slot number the first-boot unit
/// needs. `systemd-stub` packs `loader/credentials/*.cred` from the ESP into
/// the initrd, where `systemd-cryptsetup` asks for a credential named
/// `cryptsetup.passphrase`. The seal is the image's signed PCR 11 policy. The
/// marker is written first, so a sealing failure still leaves the first boot
/// the slot number that has to go.
fn credential(image: &str, esp: &Path, key: &[u8], slot: u32) -> Result<(), String> {
    credential_with(esp, slot, |_| seal(image, esp, key))
}

/// Takes the seal as a parameter, so a test can fail it without a TPM and
/// read the marker back.
pub(crate) fn credential_with(
    esp: &Path,
    slot: u32,
    seal: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let at = esp.join(CREDENTIAL_DIR);
    std::fs::create_dir_all(&at).map_err(|err| format!("{}: {err}", at.display()))?;
    let marker = at.join(SLOT_FILE);
    std::fs::write(&marker, format!("{slot}\n"))
        .map_err(|err| format!("{}: {err}", marker.display()))?;
    seal(esp)
}

/// Seals the one-time key into the credential the initrd asks for, with the
/// image's own `systemd-creds` and the TPM.
fn seal(image: &str, esp: &Path, key: &[u8]) -> Result<(), String> {
    let staged = staged_key(key)?;
    let key_mount = format!("{}:/run/one-time-key:ro", staged.0.display());
    let esp_mount = format!("{}:/esp", esp.display());
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
        ])
        .arg("--device")
        .arg(tpm())
        .args(["--entrypoint", ""])
        .args(["-v", &key_mount, "-v", &esp_mount])
        .arg(image)
        .args(["/usr/bin/systemd-creds", "encrypt", "--with-key=tpm2"])
        .arg(format!("--name={CREDENTIAL}"))
        .arg(format!("--tpm2-public-key={PCR_POLICY}"))
        .args(["--tpm2-public-key-pcrs=11", "/run/one-time-key"])
        .arg(format!("/esp/{CREDENTIAL_DIR}/{CREDENTIAL}.cred"))
        .output()
        .map_err(|err| format!("podman: {err}, and it is what seals the one-time key"))?;
    if !out.status.success() {
        return Err(format!(
            "sealing the one-time key in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

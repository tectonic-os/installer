use super::*;

/// Each entry pairs the flag's name, the screen's description and the detail
/// drawn under its menu choice. A kind whose name ends in `passphrase`
/// needs a passphrase. `tpm2-luks-pin` needs a PIN. `tpm2-luks` and
/// `tpm2-luks-pin` take a recovery key that `layout::seal` generates.
pub(crate) const KINDS: [(&str, &str, &str); 5] = [
    (NONE, copy::ENC_NONE, ""),
    ("tpm2-luks", copy::ENC_TPM2, ""),
    ("luks-passphrase", copy::ENC_PASSPHRASE, ""),
    ("tpm2-luks-passphrase", copy::ENC_BOTH, ""),
    ("tpm2-luks-pin", copy::ENC_TPM2_PIN, ""),
];

pub(crate) const NONE: &str = "none";

/// Marks a machine that has a TPM, which the three `tpm2-` kinds need.
/// `$TECT_TPM` overrides the path, so the drawn golden does not depend on the
/// machine running it having a TPM.
pub(crate) const TPM: &str = "/dev/tpmrm0";

pub(crate) fn tpm() -> PathBuf {
    std::env::var_os("TECT_TPM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(TPM))
}

/// Holds the whole disks the form offers.
const SYS_BLOCK: &str = "/sys/block";

/// `$TECT_SYS_BLOCK` overrides the path, for the same reason `$TECT_TPM`
/// does. The form draws the disks this machine has, and no two machines
/// running the drawn golden carry the same disks.
pub(crate) fn sys_block() -> PathBuf {
    std::env::var_os("TECT_SYS_BLOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SYS_BLOCK))
}

/// Names the directory the installer opens a device path under.
const DEV: &str = "/dev";

/// `$TECT_DEV` overrides the directory, for the same reason `$TECT_SYS_BLOCK`
/// does. The drawn golden names its disks from a fixture `/sys/block`, and
/// the table reader opens the device instead of running a command, so the
/// fixture must hold a file for a disk that has no node on the rig. Only
/// `device_path` in `table.rs` reads this, and it reads it in a debug build
/// alone.
pub(crate) fn dev() -> PathBuf {
    std::env::var_os("TECT_DEV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEV))
}

/// Names the VT the kernel currently shows.
const TTY0_ACTIVE: &str = "/sys/class/tty/tty0/active";

/// `$TECT_TTY0_ACTIVE` overrides the path, for the same reason
/// `$TECT_SYS_BLOCK` does. The note names the VT the installer is on, and a
/// machine with no console reads nothing here.
pub(crate) fn tty0_active() -> PathBuf {
    std::env::var_os("TECT_TTY0_ACTIVE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(TTY0_ACTIVE))
}

/// Holds the firmware's own variables.
const EFIVARS: &str = "/sys/firmware/efi/efivars";

/// `$TECT_EFIVARS` overrides the path, for the same reason `$TECT_SYS_BLOCK`
/// does. The panel states the firmware of the machine running it, and the
/// drawn golden must state one fixture's.
pub(crate) fn efivars() -> PathBuf {
    std::env::var_os("TECT_EFIVARS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(EFIVARS))
}

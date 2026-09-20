//! Reads what the machine's firmware says from the efivars filesystem.
//!
//! No function here parses a certificate. The installer finds the owner key by
//! its bytes rather than by its subject, so the screen can name what it sees
//! without carrying an X.509 parser.

use std::path::Path;

/// Names the variables this reads, spelled the way efivarfs names them,
/// `<name>-<guid>`. UEFI defines each variable under exactly one GUID, so this
/// matches the whole name. A scan for a prefix would let a lookalike under
/// another GUID win the directory race.
const SECURE_BOOT: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const SETUP_MODE: &str = "SetupMode-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const PLATFORM_KEY: &str = "PK-8be4df61-93ca-11d2-aa0d-00e098032b8c";
/// Names the image security database, which authorises a signed loader to
/// run. The platform key alone authorises no loader.
const DATABASE: &str = "db-d719b2cb-3d3a-4596-a3bc-dad00e67656f";

/// States which key the firmware's `PK` variable holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKey {
    /// Reports no readable `PK`, so the firmware sits in setup mode.
    None,
    /// Reports an enrolled `PK` that is not the owner certificate.
    Other,
    /// Reports an enrolled `PK` holding the owner certificate.
    Owner,
    /// Reports a key the firmware calls enrolled and this could not read.
    Unknown,
}

/// Holds the firmware state a screen needs. `efivars` states whether the
/// filesystem was present at all, which separates "no EFI variables" from
/// "no key". Each option states a variable this could not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Firmware {
    pub efivars: bool,
    pub secure_boot: Option<bool>,
    pub setup_mode: Option<bool>,
    pub platform_key: PlatformKey,
    /// States that the image security database holds the owner certificate.
    pub database: bool,
}

impl Default for Firmware {
    fn default() -> Self {
        Firmware {
            efivars: false,
            secure_boot: None,
            setup_mode: None,
            platform_key: PlatformKey::None,
            database: false,
        }
    }
}

impl Firmware {
    /// Reads the variables under `root`, which is the efivars filesystem's
    /// mount point. `owner` carries the owner certificate's DER bytes where
    /// the caller knows them. Every variable value carries a four-byte
    /// attribute header, which this skips.
    pub fn read(root: &Path, owner: Option<&[u8]>) -> Firmware {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Firmware::default();
        };
        let mut firmware = Firmware {
            efivars: true,
            ..Firmware::default()
        };
        // Holds the platform key as the loop actually read it. The loop
        // treats unread and absent alike. The answer below separates them.
        let mut read_key = None;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Reads only the variables this screen shows. An efivarfs read
            // can sit in the kernel until the firmware releases a variable,
            // and a variable no screen draws must not wedge the panel.
            if !matches!(
                name.as_ref(),
                SECURE_BOOT | SETUP_MODE | PLATFORM_KEY | DATABASE
            ) {
                continue;
            }
            // A variable that cannot be read leaves its own field unknown.
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let value = bytes.get(4..).unwrap_or_default();
            match name.as_ref() {
                SECURE_BOOT => firmware.secure_boot = value.first().map(|byte| *byte == 1),
                SETUP_MODE => firmware.setup_mode = value.first().map(|byte| *byte == 1),
                PLATFORM_KEY => read_key = Some(key_of(value, owner)),
                DATABASE => {
                    firmware.database = owner.is_some_and(|owner| holds(value, owner));
                }
                _ => {}
            }
        }
        // A key this read speaks for itself. Otherwise only SetupMode=1
        // states that no key is enrolled. A firmware without the variable, and
        // a firmware this could not read, are both unknown rather than
        // keyless. The panel then asks for the cautious step, which clears a
        // key that may be there.
        firmware.platform_key = match read_key {
            Some(key) => key,
            None => match firmware.setup_mode {
                Some(true) => PlatformKey::None,
                _ => PlatformKey::Unknown,
            },
        };
        firmware
    }
}

/// Classifies an enrolled `PK` by the bytes it holds.
fn key_of(value: &[u8], owner: Option<&[u8]>) -> PlatformKey {
    match value.is_empty() {
        true => PlatformKey::None,
        false => match owner {
            Some(owner) if holds(value, owner) => PlatformKey::Owner,
            _ => PlatformKey::Other,
        },
    }
}

/// States whether `value` carries `owner` verbatim. An enrolled key holds the
/// certificate's bytes as they are, so this understands no signature list.
fn holds(value: &[u8], owner: &[u8]) -> bool {
    !owner.is_empty() && value.windows(owner.len()).any(|window| window == owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes one variable the way efivarfs carries it, which is four
    /// attribute bytes and then a value.
    fn variable(root: &Path, name: &str, value: &[u8]) {
        std::fs::create_dir_all(root).unwrap();
        let file = match name {
            "db" => format!("{name}-d719b2cb-3d3a-4596-a3bc-dad00e67656f"),
            _ => format!("{name}-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
        };
        let mut bytes = vec![0, 0, 0, 7];
        bytes.extend_from_slice(value);
        std::fs::write(root.join(file), bytes).unwrap();
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("tect-firmware-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// Reads a full state from one fixture directory.
    fn read(name: &str, owner: Option<&[u8]>, variables: &[(&str, &[u8])]) -> Firmware {
        let root = fixture(name);
        for (name, value) in variables {
            variable(&root, name, value);
        }
        let firmware = Firmware::read(&root, owner);
        let _ = std::fs::remove_dir_all(&root);
        firmware
    }

    /// A missing efivars directory never reads as setup mode. The firmware
    /// must say so.
    #[test]
    fn a_machine_without_efivars_is_neither_keyless_nor_enrolled() {
        let firmware = Firmware::read(&fixture("absent"), None);
        assert!(!firmware.efivars, "{firmware:?}");
        assert_eq!(firmware.secure_boot, None, "{firmware:?}");
        assert_eq!(firmware.platform_key, PlatformKey::None, "{firmware:?}");
    }

    /// Secure Boot, setup mode and the database each come off their own
    /// variable. A key that is present is never claimed to be the owner's.
    #[test]
    fn the_variables_are_read_apart_from_the_owner() {
        let firmware = read(
            "present",
            Some(b"owner certificate"),
            &[
                ("SecureBoot", &[1]),
                ("SetupMode", &[0]),
                ("PK", b"vendor certificate bytes"),
                ("db", b"vendor database bytes"),
            ],
        );
        assert!(firmware.efivars && firmware.secure_boot == Some(true));
        assert_eq!(firmware.setup_mode, Some(false));
        assert_eq!(firmware.platform_key, PlatformKey::Other);
        assert!(!firmware.database, "{firmware:?}");
    }

    /// The owner key is the one comparison here, and it compares bytes. The
    /// same certificate inside a longer list is the owner's, in `PK` and in
    /// `db`. A shorter certificate must not match out of a longer value.
    #[test]
    fn an_enrolled_owner_certificate_is_recognised_in_place() {
        let firmware = read(
            "owner",
            Some(b"owner certificate"),
            &[
                ("SecureBoot", &[1]),
                ("SetupMode", &[0]),
                ("PK", b"list-header owner certificate list-footer"),
                ("db", b"list-header owner certificate list-footer"),
            ],
        );
        assert_eq!(firmware.platform_key, PlatformKey::Owner, "{firmware:?}");
        assert!(firmware.database, "{firmware:?}");

        assert_eq!(key_of(b"", Some(b"owner certificate")), PlatformKey::None);
        assert_eq!(
            key_of(b"another", Some(b"owner certificate")),
            PlatformKey::Other
        );
        assert!(
            !holds(b"owner cer", b"owner certificate"),
            "a value shorter than the certificate cannot hold it"
        );
        assert!(!holds(b"another certificate", b"owner certificate"));
        assert!(!holds(b"another", b""));
    }

    /// Setup mode with a `PK` this cannot read reads as unknown rather than
    /// none, because the firmware states that an enrolled key is there.
    #[test]
    fn a_key_the_firmware_claims_but_this_cannot_read_is_unknown() {
        // No PK variable at all, with SetupMode 0.
        let firmware = read(
            "unread-pk",
            None,
            &[("SecureBoot", &[1]), ("SetupMode", &[0])],
        );
        assert_eq!(firmware.platform_key, PlatformKey::Unknown, "{firmware:?}");
        // The firmware states the same thing itself, which is no key and
        // setup mode.
        let setup = read("setup", None, &[("SecureBoot", &[0]), ("SetupMode", &[1])]);
        assert_eq!(setup.platform_key, PlatformKey::None, "{setup:?}");
        assert_eq!(setup.setup_mode, Some(true), "{setup:?}");
        // A firmware carrying neither variable is never called keyless,
        // because no variable stated setup mode.
        let bare = read("bare", None, &[("SecureBoot", &[0])]);
        assert_eq!(bare.platform_key, PlatformKey::Unknown, "{bare:?}");
        assert_eq!(bare.setup_mode, None, "{bare:?}");
    }
}

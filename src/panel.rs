use super::*;

/// Builds the panel above the form, which states the firmware and what the
/// image needs from it. The installer reads both once, because neither changes
/// while the form is up.
pub(crate) fn panel(payload: &Payload) -> Vec<common::ui::HeaderLine> {
    panel_lines(&firmware(payload), payload)
}

/// Reads the firmware this machine states, and whether the image's own
/// certificate holds the platform key. The platform key only changes what the
/// panel says for a UKI image, and asking the image costs a container run, so
/// the probe runs where it decides between "a key" and "our key". The last
/// screen reads the same state for the steps it asks for.
pub(crate) fn firmware(payload: &Payload) -> crate::firmware::Firmware {
    let mut firmware = crate::firmware::Firmware::read(&efivars(), None);
    if firmware.platform_key == crate::firmware::PlatformKey::Other && !payload.boot.is_empty() {
        let owner = owner_certificate(&payload.image);
        firmware = crate::firmware::Firmware::read(&efivars(), owner.as_deref());
    }
    firmware
}

/// Returns the steps the last screen asks for, from the reading the panel
/// used. An image that needs no step gets none. If a foreign key still holds
/// the platform key, the user gets the vendor's setup-mode steps. Firmware
/// that is not UEFI gets no promise at all.
pub(crate) fn next_steps(boot: &str, firmware: &crate::firmware::Firmware) -> Option<&'static str> {
    use crate::firmware::PlatformKey;
    if !firmware.efivars {
        return None;
    }
    match boot {
        "uki-shim" => Some(copy::shim_steps()),
        // The owner certificate holds both the platform key and the
        // database, or the firmware sits in setup mode and the image enrols
        // its own. Neither state asks the user for a step.
        "uki-db" => match (firmware.platform_key, firmware.database) {
            (PlatformKey::Owner, true) | (PlatformKey::None, _) => None,
            _ => Some(copy::next_steps_setup()),
        },
        _ => None,
    }
}

/// Returns the rows the erase confirmation carries in the warning colour.
/// They state the platform-key fact that `next_steps` asks the vendor's steps
/// for after the install, said before it instead. The shim chain touches no
/// firmware, so it carries no row.
pub(crate) fn erase_warning(boot: &str, firmware: &crate::firmware::Firmware) -> Vec<&'static str> {
    match boot {
        "uki-db" if next_steps(boot, firmware).is_some() => {
            vec![
                copy::ERASE_WARNING_SECURE_BOOT,
                copy::ERASE_WARNING_UEFI_SETUP,
            ]
        }
        _ => Vec::new(),
    }
}

/// Writes the panel's words from a firmware reading. This is separate from
/// the reading so a test can check the wording without a machine's firmware
/// and without a container run.
pub(crate) fn panel_lines(
    firmware: &crate::firmware::Firmware,
    payload: &Payload,
) -> Vec<common::ui::HeaderLine> {
    use crate::firmware::PlatformKey;
    use common::ui::HeaderLine::{Good, Pair, Row, Section, Warn};

    let mut lines = vec![
        Section(copy::PANEL_IMAGE.to_string()),
        Pair("name".to_string(), image_name(&payload.image)),
        Pair("repo".to_string(), payload.image.clone()),
        Pair("bootloader".to_string(), bootloader_said(payload)),
        Section(copy::PANEL_FIRMWARE.to_string()),
    ];
    if !firmware.efivars {
        lines.push(Row(copy::FIRMWARE_NO_EFIVARS.to_string()));
    } else {
        // On reads green and off reads amber. A firmware already in setup
        // mode says so, because setup mode is the state the key enrolment
        // needs, and the user reads it differently from an off they must
        // change. A state the firmware did not answer pairs with its label,
        // untinted.
        lines.push(match firmware.secure_boot {
            Some(true) => Good(
                copy::FIRMWARE_SECURE.to_string(),
                copy::FIRMWARE_ON.to_string(),
            ),
            Some(false) => match firmware.setup_mode {
                Some(true) => Warn(
                    copy::FIRMWARE_SECURE.to_string(),
                    copy::FIRMWARE_SETUP.to_string(),
                ),
                _ => Warn(
                    copy::FIRMWARE_SECURE.to_string(),
                    copy::FIRMWARE_OFF.to_string(),
                ),
            },
            None => Pair(
                copy::FIRMWARE_SECURE.to_string(),
                copy::FIRMWARE_UNREADABLE.to_string(),
            ),
        });
        lines.push(Pair(
            copy::PLATFORM_KEY.to_string(),
            match firmware.platform_key {
                PlatformKey::Owner => copy::PLATFORM_KEY_OWNER,
                PlatformKey::Other => copy::PLATFORM_KEY_PRESENT,
                PlatformKey::Unknown => copy::PLATFORM_KEY_UNKNOWN,
                PlatformKey::None => copy::PLATFORM_KEY_SETUP,
            }
            .to_string(),
        ));
    }
    if payload.boot.is_empty() {
        return lines;
    }
    lines.push(Section(copy::PANEL_REQUIRED.to_string()));
    // Firmware boots a UKI chain. Without efivars this screen can promise
    // the user nothing about enrolment.
    if !firmware.efivars {
        lines.push(Row(copy::UKI_NEEDS_UEFI.to_string()));
        return lines;
    }
    let mut rows: Vec<&'static str> = Vec::new();
    match (
        payload.boot.as_str(),
        firmware.platform_key,
        firmware.database,
    ) {
        // The key that updates the databases and the database that
        // authorises the loader must both hold the owner certificate. The
        // platform key alone loads no signed image.
        ("uki-db", PlatformKey::Owner, true) => {
            lines.push(Row(copy::UKI_DB_OWNER.to_string()));
            return lines;
        }
        ("uki-db", PlatformKey::None, _) => {
            lines.push(Row(copy::UKI_DB_ENROLL.to_string()));
            return lines;
        }
        ("uki-db", _, _) => rows.extend(copy::required_uki_db()),
        ("uki-shim", _, _) => rows.push(copy::shim_steps()),
        _ => {}
    }
    lines.extend(rows.into_iter().map(|line| Row(line.to_string())));
    lines
}

/// Returns the image's name inside its reference, which is the last path
/// segment without a tag or a digest. That segment is what the user calls the
/// image.
pub(crate) fn image_name(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    let name = last.split([':', '@']).next().unwrap_or(last);
    match name.is_empty() {
        true => reference.to_string(),
        false => name.to_string(),
    }
}

/// Names the boot chain the payload declares, for the OS Image section.
fn bootloader_said(payload: &Payload) -> String {
    match (payload.boot.is_empty(), payload.bootloader.as_str()) {
        (false, "systemd") => "systemd-boot with a signed UKI".to_string(),
        (false, other) => format!("{other} with a signed UKI"),
        (true, "" | "grub2") => "grub2".to_string(),
        (true, other) => other.to_string(),
    }
}

/// Reads the owner certificate the payload enrols, as DER. The module
/// installs the PEM under this path and openssl re-encodes it. Matching an
/// enrolled platform key stays a byte comparison rather than a subject
/// comparison.
fn owner_certificate(image: &str) -> Option<Vec<u8>> {
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
            image,
            "openssl",
            "x509",
            "-in",
            "/usr/share/secureboot/sb_cert.pem",
            "-outform",
            "DER",
        ])
        .output()
        .ok()?;
    match out.status.success() && !out.stdout.is_empty() {
        true => Some(out.stdout),
        false => None,
    }
}

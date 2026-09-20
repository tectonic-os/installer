use super::*;

/// `ROW_ROOM` is an option-list row's width, which keeps two columns for the
/// cursor marker beside the prose. The kernel-VT console the installer media
/// falls back to is 80 columns, and every widget clips rather than reflows.
#[test]
fn every_installer_screen_string_fits_the_narrowest_console() {
    let rows = [
        ("INSTALL_DONE", copy::INSTALL_DONE),
        ("DONE_KEYS", copy::DONE_KEYS),
        ("RECOVERY_HEADING", copy::RECOVERY_HEADING),
        ("NEXT_STEPS_HEADING", copy::NEXT_STEPS_HEADING),
        ("write_down", copy::write_down()),
        ("RESTART", copy::RESTART),
        ("SHUT_DOWN", copy::SHUT_DOWN),
        ("INSTALL", copy::INSTALL),
        ("GO_BACK", copy::GO_BACK),
        ("INSTALLATION_SUMMARY", copy::INSTALLATION_SUMMARY),
        ("READY", copy::READY),
        ("START_INSTALLATION", copy::START_INSTALLATION),
        ("ERASE_WARNING_SECURE_BOOT", copy::ERASE_WARNING_SECURE_BOOT),
        ("ERASE_WARNING_UEFI_SETUP", copy::ERASE_WARNING_UEFI_SETUP),
        ("OPENED_ADD_KEY", copy::OPENED_ADD_KEY),
        ("OPENED_KEYFILE_PLAIN", copy::OPENED_KEYFILE_PLAIN),
        ("PASSWORD_SET", copy::PASSWORD_SET),
        ("ROW_ENCRYPTION", copy::ROW_ENCRYPTION),
    ];
    for (name, text) in rows {
        let wide = text.chars().count();
        assert!(
            wide <= common::ui::ROW_ROOM,
            "{name} is {wide} columns and a list row has {}",
            common::ui::ROW_ROOM
        );
    }
}

/// Each case below pairs one firmware reading with one payload, because the
/// panel's wording turns on the platform key and the boot chain together.
#[test]
fn the_panel_states_the_firmware_and_what_the_image_needs() {
    use crate::firmware::{Firmware, PlatformKey};
    use common::ui::HeaderLine;
    let payload = |boot: &str| Payload {
        recipe: "/mnt/tect/install-recipe.json".into(),
        image: "ghcr.io/tectonic-os/deb2:latest".to_string(),
        hostname: "deb2".to_string(),
        filesystem: "ext4".to_string(),
        bootloader: "grub2".to_string(),
        boot: boot.to_string(),
        composefs: false,
        reserve: std::sync::OnceLock::new(),
        luks_initramfs: true,
    };
    let payload_of = |boot: &str, bootloader: &str| Payload {
        bootloader: bootloader.to_string(),
        ..payload(boot)
    };
    let state = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    let setup = |platform_key| Firmware {
        setup_mode: Some(true),
        ..state(platform_key, false)
    };
    let text = |lines: &[HeaderLine]| -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                HeaderLine::Section(title) | HeaderLine::Row(title) => title.clone(),
                HeaderLine::Pair(label, value)
                | HeaderLine::Good(label, value)
                | HeaderLine::Warn(label, value) => format!("{label}: {value}"),
            })
            .collect()
    };
    let said = |lines: Vec<String>, phrase: &str| lines.iter().any(|line| line == phrase);

    // The payload's `boot` is empty here, which is the base's vendor chain.
    let lines = text(&panel_lines(&state(PlatformKey::None, false), &payload("")));
    assert!(said(lines.clone(), copy::PANEL_IMAGE), "{lines:?}");
    assert!(said(lines.clone(), "name: deb2"), "{lines:?}");
    assert!(
        said(lines.clone(), "repo: ghcr.io/tectonic-os/deb2:latest"),
        "{lines:?}"
    );
    assert!(
        said(lines.clone(), "bootloader: grub2 (the base's vendor chain)"),
        "{lines:?}"
    );
    // A vendor chain asks the firmware for nothing, so the panel drops the
    // Required section.
    assert!(!said(lines.clone(), copy::PANEL_REQUIRED), "{lines:?}");

    // A vendor key holding the platform key asks for the after-install
    // steps. `required_uki_db` states what enrolling costs a dual boot.
    let lines = text(&panel_lines(
        &state(PlatformKey::Other, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(lines.clone(), "bootloader: systemd-boot with a signed UKI"),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::PANEL_REQUIRED), "{lines:?}");
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");
    assert!(
        said(lines.clone(), copy::required_uki_db()[2]),
        "the dual-boot cost is on the panel: {lines:?}"
    );
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_PRESENT),
        ),
        "{lines:?}"
    );
    assert!(!said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");

    // `PlatformKey::None` is what makes the panel read `setup mode` and
    // offer the enrol row. The fixture's `setup_mode` flag is not read on
    // this path, because secure boot answers `Some(true)`.
    let lines = text(&panel_lines(
        &setup(PlatformKey::None),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_SETUP),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::UKI_DB_ENROLL), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    // The owner certificate holds the platform key and the database, so the
    // panel asks for no step.
    let lines = text(&panel_lines(
        &state(PlatformKey::Owner, true),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_OWNER),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    // The platform key alone authorises no loader, so an owner key without
    // the database entry still asks for the steps.
    let lines = text(&panel_lines(
        &state(PlatformKey::Owner, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");
    assert!(!said(lines.clone(), copy::UKI_DB_OWNER), "{lines:?}");

    // An unread platform key is judged like a foreign one. The panel asks
    // for the steps that clear it rather than reading it as absent.
    let lines = text(&panel_lines(
        &state(PlatformKey::Unknown, false),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(
        said(
            lines.clone(),
            &format!("{}: {}", copy::PLATFORM_KEY, copy::PLATFORM_KEY_UNKNOWN),
        ),
        "{lines:?}"
    );
    assert!(said(lines.clone(), copy::required_uki_db()[0]), "{lines:?}");

    // The shim chain asks for one MokManager enrolment and leaves the
    // firmware keys alone.
    let lines = text(&panel_lines(
        &state(PlatformKey::Other, false),
        &payload_of("uki-shim", "systemd"),
    ));
    assert!(said(lines.clone(), copy::shim_steps()), "{lines:?}");
    assert!(
        !said(lines.clone(), copy::required_uki_db()[0]),
        "{lines:?}"
    );

    // Without efivars the panel states that fact and stops. It promises no
    // first-boot enrolment, because it cannot read what the firmware holds.
    let lines = text(&panel_lines(
        &Firmware::default(),
        &payload_of("uki-db", "systemd"),
    ));
    assert!(said(lines.clone(), copy::PANEL_FIRMWARE), "{lines:?}");
    assert!(said(lines.clone(), copy::FIRMWARE_NO_EFIVARS), "{lines:?}");
    assert!(said(lines.clone(), copy::UKI_NEEDS_UEFI), "{lines:?}");
    assert!(!said(lines.clone(), copy::UKI_DB_ENROLL), "{lines:?}");
}

/// `next_steps` reads the same `Firmware` the panel read. An image with no
/// UKI chain and a machine with no efivars are both answered `None`, which a
/// wildcard arm over the boot chain would get wrong.
#[test]
fn the_last_screen_asks_for_the_firmware_steps_the_panel_saw() {
    use crate::firmware::{Firmware, PlatformKey};
    let with = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    assert_eq!(
        next_steps("uki-db", &with(PlatformKey::Other, false)),
        Some(copy::next_steps_setup())
    );
    assert_eq!(next_steps("uki-db", &with(PlatformKey::None, false)), None);
    assert_eq!(next_steps("uki-db", &with(PlatformKey::Owner, true)), None);
    // An owner platform key without the database entry loads no signed image.
    assert_eq!(
        next_steps("uki-db", &with(PlatformKey::Owner, false)),
        Some(copy::next_steps_setup())
    );
    assert_eq!(
        next_steps("uki-shim", &with(PlatformKey::Other, false)),
        Some(copy::shim_steps())
    );
    assert_eq!(next_steps("", &with(PlatformKey::Other, false)), None);
    assert_eq!(next_steps("uki-db", &Firmware::default()), None);
}

/// `erase_warning` calls `next_steps` and warns on the `uki-db` chain alone.
/// The shim chain asks for a step and still carries no warning row, because
/// it changes no firmware key.
#[test]
fn the_erase_warning_matches_the_cases_next_steps_asks_for() {
    use crate::firmware::{Firmware, PlatformKey};
    let with = |platform_key, database| Firmware {
        efivars: true,
        secure_boot: Some(true),
        setup_mode: Some(false),
        platform_key,
        database,
    };
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Other, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Owner, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    // An unread platform key warns like a foreign one. The wildcard arm in
    // `next_steps` covers it, so it never drops out silently.
    assert_eq!(
        erase_warning("uki-db", &with(PlatformKey::Unknown, false)),
        vec![
            copy::ERASE_WARNING_SECURE_BOOT,
            copy::ERASE_WARNING_UEFI_SETUP
        ]
    );
    assert!(erase_warning("uki-db", &with(PlatformKey::None, false)).is_empty());
    assert!(erase_warning("uki-db", &with(PlatformKey::Owner, true)).is_empty());
    assert!(erase_warning("uki-shim", &with(PlatformKey::Other, false)).is_empty());
    assert!(erase_warning("", &with(PlatformKey::Other, false)).is_empty());
    assert!(erase_warning("uki-db", &Firmware::default()).is_empty());
}

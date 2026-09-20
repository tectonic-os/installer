use super::*;

/// Two of the five kinds fisherman takes end in `passphrase`. The other
/// three are answered without one.
#[test]
fn only_the_forms_named_for_a_passphrase_are_asked_for_one() {
    let wants: Vec<&str> = KINDS
        .iter()
        .map(|(name, _, _)| *name)
        .filter(|name| Encryption::wants_passphrase(name))
        .collect();
    assert_eq!(wants, ["luks-passphrase", "tpm2-luks-passphrase"]);
}

/// Walks one passphrase kind from no answer to both halves agreeing. The
/// empty confirmation is the case this test exists to hold.
#[test]
fn the_luks_window_refuses_a_passphrase_nobody_confirmed() {
    assert_eq!(luks_short_of(NONE, "", "", "", "", true), None);
    assert_eq!(
        luks_short_of("luks-passphrase", "", "", "", "", true),
        Some(copy::still_needs(&[copy::ROW_PASSPHRASE]))
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "", "", "", true),
        Some(copy::still_needs(&[copy::CONFIRM_PASSPHRASE]))
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesam", "", "", true),
        Some(String::new())
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesame", "", "", true),
        None
    );
    assert_eq!(
        luks_short_of("luks-passphrase", "opensesame", "opensesame", "", "", false),
        Some(copy::NO_LUKS_INITRAMFS.to_string())
    );
}

/// Walks `tpm2-luks-pin` through the same cases as the passphrase kind
/// above. The PIN pair is held to the same rule.
#[test]
fn the_luks_window_refuses_a_pin_nobody_confirmed() {
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "", "", true),
        Some(copy::still_needs(&[copy::ROW_PIN]))
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "", true),
        Some(copy::still_needs(&[copy::CONFIRM_PIN]))
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4320", true),
        Some(String::new())
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4321", true),
        None
    );
    assert_eq!(
        luks_short_of("tpm2-luks-pin", "", "", "4321", "4321", false),
        Some(copy::NO_LUKS_INITRAMFS.to_string())
    );
    // A passphrase held against a kind that owes none does not gate it.
    assert_eq!(
        luks_short_of("tpm2-luks", "opensesame", "", "", "", true),
        None
    );
}

/// The `--encryption tpm2-luks` flag seeds this row without opening the
/// window, so `short_of` is the only place left to refuse it.
#[test]
fn the_whole_disk_form_refuses_a_seeded_tpm2_kind_without_a_tpm() {
    use common::ui::{Choice, Field};
    let fields = vec![
        Field::text(copy::ROW_HOSTNAME, "deb2"),
        Field::text(copy::ROW_ACCOUNT, "tect"),
        Field::secret(copy::ROW_PASSWORD, "hunter2"),
        Field::secret(copy::ROW_CONFIRM, "hunter2"),
        Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(0),
        ),
        Field::pick(
            copy::ROW_ENCRYPTION,
            vec![Choice::new(shown("tpm2-luks"), "")],
            Some(0),
        ),
        Field::pick(copy::ROW_DATA, vec![Choice::new(copy::NONE, "")], Some(0)),
        Field::text(copy::ROW_SIZE, ""),
        Field::table("disk", "disk", &[], vec![], vec![], vec![], 0, false),
        Field::secret(copy::ROW_PASSPHRASE, ""),
    ];
    assert_eq!(
        short_of(&fields, "/dev/vda", None, &a_payload(), false, None).as_deref(),
        Some(copy::NO_TPM)
    );
    // A TPM makes the same seeded row acceptable.
    assert_ne!(
        short_of(&fields, "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::NO_TPM)
    );
}

/// The `--encryption tpm2-luks-pin` flag seeds the row with no PIN, so the
/// missing PIN has to block Install here as well.
#[test]
fn the_whole_disk_form_refuses_a_seeded_pin_kind_with_no_pin() {
    use common::ui::{Choice, Field};
    let fields = vec![
        Field::text(copy::ROW_HOSTNAME, "deb2"),
        Field::text(copy::ROW_ACCOUNT, "tect"),
        Field::secret(copy::ROW_PASSWORD, "hunter2"),
        Field::secret(copy::ROW_CONFIRM, "hunter2"),
        Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(0),
        ),
        Field::pick(
            copy::ROW_ENCRYPTION,
            vec![Choice::new(shown("tpm2-luks-pin"), "")],
            Some(0),
        ),
        Field::pick(copy::ROW_DATA, vec![Choice::new(copy::NONE, "")], Some(0)),
        Field::text(copy::ROW_SIZE, ""),
        Field::table("disk", "disk", &[], vec![], vec![], vec![], 0, false),
        Field::secret(copy::ROW_PASSPHRASE, ""),
        Field::secret(copy::ROW_PIN, ""),
    ];
    assert_eq!(
        short_of(&fields, "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::still_needs(&[copy::ROW_PIN]).as_str())
    );
}

/// The summary is the last screen before the disk is written. It names the
/// password as set and never prints it back.
#[test]
fn the_confirm_screen_shows_what_is_about_to_be_erased() {
    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "deb2".to_string(),
        user: "tect".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: NONE.to_string(),
            passphrase: String::new(),
            pin: String::new(),
        },
        data: Data::default(),
        layout: None,
    };
    assert_eq!(
        answers.summary(&Payload {
            recipe: "/mnt/tect/install-recipe.json".into(),
            image: "ghcr.io/tectonic-os/deb2:latest".to_string(),
            hostname: "deb2".to_string(),
            filesystem: "ext4".to_string(),
            bootloader: "grub2".to_string(),
            boot: String::new(),
            composefs: false,
            reserve: std::sync::OnceLock::new(),
            luks_initramfs: true,
        }),
        vec![
            (copy::ROW_DISK.to_string(), "/dev/vda".to_string()),
            (copy::ROW_HOSTNAME.to_string(), "deb2".to_string()),
            (copy::ROW_ACCOUNT.to_string(), "tect".to_string()),
            (
                copy::ROW_PASSWORD.to_string(),
                copy::PASSWORD_SET.to_string()
            ),
            (copy::ROW_ENCRYPTION.to_string(), copy::ENC_NONE.to_string()),
            (copy::ROW_DATA.to_string(), copy::DATA_TOGETHER.to_string()),
            // These three rows are the default layout the user never
            // answered. The summary is the only screen that spells it out.
            ("esp".to_string(), "2.0 GB  fat32".to_string()),
            ("/boot".to_string(), "2.0 GB  ext4".to_string()),
            ("root".to_string(), "the rest  ext4".to_string()),
        ]
    );
    let question = copy::erasing(&answers.disk);
    assert!(
        question.contains("/dev/vda") && question.contains("erased"),
        "{question}"
    );
    assert!(!question.contains("hunter2"));
}

/// The ESP and `/boot` take their room before home does, so the room home
/// is measured against is smaller than the disk row says.
#[test]
fn a_home_size_the_disk_cannot_hold_is_refused() {
    use common::ui::{Cell, Choice, Field};
    let disk = |answered: bool| {
        let name = match answered {
            true => Cell::set("\u{25c9} QEMU HARDDISK (/dev/vda)"),
            false => Cell::new("\u{25c9} QEMU HARDDISK (/dev/vda)"),
        };
        let row = vec![
            name,
            Cell::new("68.7 GB"),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
        ];
        Field::table(
            copy::DISK_SELECTION,
            copy::ROW_DISK,
            &copy::layout_headings(),
            vec![row],
            vec![true],
            vec![Vec::new()],
            0,
            answered,
        )
    };
    let form = |size: &str| {
        vec![
            Field::text(copy::ROW_HOSTNAME, "deb2"),
            Field::text(copy::ROW_ACCOUNT, "tect"),
            Field::secret(copy::ROW_PASSWORD, "hunter2"),
            Field::secret(copy::ROW_CONFIRM, "hunter2"),
            Field::pick(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(0),
            ),
            Field::action(copy::ROW_ENCRYPTION, shown(NONE)),
            Field::pick(
                copy::ROW_DATA,
                vec![Choice::new(copy::DATA_SEPARATE, "")],
                Some(0),
            ),
            Field::measure(copy::ROW_SIZE, size, "GB"),
            disk(true),
            Field::secret(copy::ROW_PASSPHRASE, ""),
        ]
    };
    // The ESP takes 2 GB, the GRUB `/boot` 2 GB and the root's reserve
    // 10 GB. A 40 GB home fits, because 40 plus 14 is under the disk's 68.
    assert!(short_of(&form("40"), "/dev/vda", None, &a_payload(), true, None).is_none());
    // A home under 1 GB is refused before the disk room is measured.
    assert_eq!(
        short_of(&form("0"), "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::home_too_small("0.0 GB").as_str())
    );
    // A home that leaves the root no room is refused.
    assert_eq!(
        short_of(&form("500"), "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::home_too_big("500.0 GB", "68.7 GB").as_str())
    );
    assert_eq!(
        short_of(&form("65"), "/dev/vda", None, &a_payload(), true, None).as_deref(),
        Some(copy::home_too_big("65.0 GB", "68.7 GB").as_str())
    );
    // With no disk row answered `chosen_disk` returns `None`, so the size
    // check is skipped and the missing list names the disk instead.
    let mut unchosen = form("500");
    unchosen[ROW_TABLE] = disk(false);
    let said = short_of(&unchosen, "", None, &a_payload(), true, None).unwrap_or_default();
    assert!(said.contains(copy::INSTALLATION_DISK), "{said}");
}

/// Holds `asked` to the rows it draws as the layout answer changes. The
/// passphrase row is never among them.
#[test]
fn the_passphrase_is_never_a_row_and_the_manual_rows_come_and_go() {
    use common::ui::{Choice, Field};
    let form = |manual: bool, encryption: Field| {
        vec![
            Field::text(copy::ROW_HOSTNAME, "deb2"),
            Field::text(copy::ROW_ACCOUNT, "tect"),
            Field::secret(copy::ROW_PASSWORD, "hunter2"),
            Field::secret(copy::ROW_CONFIRM, "hunter2"),
            Field::pick(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(usize::from(manual)),
            ),
            encryption,
            Field::pick(copy::ROW_DATA, vec![Choice::new(copy::NONE, "")], Some(0)),
            Field::text(copy::ROW_SIZE, ""),
            Field::table("disk", "disk", &[], vec![], vec![], vec![], 0, false),
            Field::secret(copy::ROW_PASSPHRASE, ""),
        ]
    };
    let kinds = |kind: &str| Field::action(copy::ROW_ENCRYPTION, shown(kind));
    // No whole-disk kind puts the passphrase on the form.
    for kind in [NONE, "tpm2-luks", "luks-passphrase", "tpm2-luks-passphrase"] {
        assert!(!asked(&form(false, kinds(kind))).contains(&ROW_PASSPHRASE));
    }
    // The size row is asked only once the user answers a separate home.
    let mut sized = form(false, kinds(NONE));
    assert!(!asked(&sized).contains(&ROW_SIZE));
    sized[ROW_DATA] = Field::pick(
        copy::ROW_DATA,
        vec![Choice::new(copy::DATA_SEPARATE, "")],
        Some(0),
    );
    assert!(asked(&sized).contains(&ROW_SIZE));
    // A whole-disk answer asks eight rows, dropping passphrase and size.
    assert_eq!(asked(&form(false, kinds(NONE))).len(), 8);
    // A manual layout with no container open drops the whole-disk rows.
    let bare = asked(&form(true, kinds(NONE)));
    assert!(!bare.contains(&ROW_ENCRYPTION));
    assert!(!bare.contains(&ROW_DATA) && !bare.contains(&ROW_SIZE));
    assert!(bare.contains(&ROW_TABLE));
    let opened = Field::pick(
        copy::ROW_ENCRYPTION,
        vec![Choice::new(copy::OPENED_KEEP, "")],
        Some(0),
    );
    assert!(asked(&form(true, opened)).contains(&ROW_ENCRYPTION));
}

/// Four answers nothing derives block Install. The form also checks the two
/// password halves agree, which a question sequence had to ask twice for.
#[test]
fn the_action_says_what_the_form_is_short_of() {
    use common::ui::{Choice, Field};
    let form = |password: &str, confirm: &str, kind: &str, passphrase: &str| {
        vec![
            Field::text(copy::ROW_HOSTNAME, "deb2"),
            Field::text(copy::ROW_ACCOUNT, "tect"),
            Field::secret(copy::ROW_PASSWORD, password),
            Field::secret(copy::ROW_CONFIRM, confirm),
            Field::pick(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(0),
            ),
            Field::pick(
                copy::ROW_ENCRYPTION,
                vec![Choice::new(shown(kind), "")],
                Some(0),
            ),
            Field::pick(copy::ROW_DATA, vec![Choice::new(copy::NONE, "")], Some(0)),
            Field::text(copy::ROW_SIZE, ""),
            Field::table("disk", "disk", &[], vec![], vec![], vec![], 0, false),
            Field::secret(copy::ROW_PASSPHRASE, passphrase),
        ]
    };
    // Switches the layout row to manual, which is what makes `short_of`
    // read the held layout the cases below pass in.
    let manual = |mut fields: Vec<common::ui::Field>| {
        fields[ROW_LAYOUT] = Field::pick(
            copy::ROW_LAYOUT,
            vec![
                Choice::new(copy::LAYOUT_WHOLE, ""),
                Choice::new(copy::LAYOUT_MANUAL, ""),
            ],
            Some(1),
        );
        fields
    };
    assert!(short_of(
        &form("hunter2", "hunter2", NONE, ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_none());
    // Both password halves sit on screen at once, so the refusal is empty
    // and only the red pair says what is wrong.
    let differ = short_of(
        &form("hunter2", "hunter3", NONE, ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None,
    )
    .unwrap();
    assert_eq!(differ, "");
    // Only a kind named for a passphrase is short of one.
    assert!(short_of(
        &form("hunter2", "hunter2", "luks-passphrase", ""),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_some());
    assert!(short_of(
        &form("hunter2", "hunter2", "luks-passphrase", "x"),
        "/dev/vda",
        None,
        &a_payload(),
        true,
        None
    )
    .is_none());
    assert_eq!(
        short_of(
            &form("hunter2", "hunter2", "luks-passphrase", "x"),
            "/dev/vda",
            None,
            &Payload {
                luks_initramfs: false,
                ..a_payload()
            },
            true,
            None
        )
        .as_deref(),
        Some(copy::NO_LUKS_INITRAMFS)
    );
    // The missing list names every empty answer at once.
    let mut bare = form("", "", NONE, "");
    bare[ROW_ACCOUNT] = Field::text(copy::ROW_ACCOUNT, "");
    let short = short_of(&bare, "/dev/vda", None, &a_payload(), true, None).unwrap();
    assert!(
        short.contains(copy::ROW_ACCOUNT) && short.contains(copy::ROW_PASSWORD),
        "{short}"
    );

    // A separate home answer is short of the size row until it is filled.
    let data = |form: &mut Vec<common::ui::Field>, label: String, size: &str| {
        form[ROW_DATA] = Field::pick(copy::ROW_DATA, vec![Choice::new(label, "")], Some(0));
        form[ROW_SIZE] = Field::text(copy::ROW_SIZE, size);
    };
    let mut sized = form("hunter2", "hunter2", NONE, "");
    data(&mut sized, copy::DATA_SEPARATE.to_string(), "");
    assert!(short_of(&sized, "/dev/vda", None, &a_payload(), true, None)
        .unwrap()
        .contains(copy::ROW_SIZE));
    data(&mut sized, copy::DATA_SEPARATE.to_string(), "200 GB");
    assert!(short_of(&sized, "/dev/vda", None, &a_payload(), true, None).is_none());

    // Fisherman wraps the separate home in the root's passphrase, so an
    // encrypted kind and a separate home are accepted together.
    let mut encrypted = form("hunter2", "hunter2", "tpm2-luks", "");
    data(&mut encrypted, copy::DATA_SEPARATE.to_string(), "200 GB");
    assert!(short_of(&encrypted, "/dev/vda", None, &a_payload(), true, None).is_none());

    // A root opened by a key file leaves the machine nothing to read at
    // boot. A TPM2 token staged inside that root cannot be enrolled before
    // the first boot unlocks it, so both answers on the row are refused.
    let root_keyfile = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![CustomMount {
            partition: "/dev/vda1".to_string(),
            target: "/boot/efi".to_string(),
            fstype: "unformatted".to_string(),

            passphrase: String::new(),
        }],
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/".to_string(),
            key: Key::Data(b"key".to_vec()),
        }],
        ..Default::default()
    };
    let opens_row = |label: &str| {
        let mut fields = manual(form("hunter2", "hunter2", NONE, ""));
        fields[ROW_ENCRYPTION] =
            Field::pick(copy::ROW_ENCRYPTION, vec![Choice::new(label, "")], Some(0));
        fields
    };
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_KEEP),
            "/dev/vda",
            Some(&root_keyfile),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_ROOT_KEYFILE)
    );
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_TPM2),
            "/dev/vda",
            Some(&root_keyfile),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_ROOT_KEYFILE)
    );

    // A plain root encrypts nothing, so a key added beside it would sit
    // readable. An answer held from when the root was open is refused
    // here, before fisherman writes the disk.
    let plain_root = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![
            CustomMount {
                partition: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                fstype: "unformatted".to_string(),

                passphrase: String::new(),
            },
            CustomMount {
                partition: "/dev/vda2".to_string(),
                target: "/".to_string(),
                fstype: "ext4".to_string(),

                passphrase: String::new(),
            },
        ],
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/var".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert!(short_of(
        &opens_row(copy::OPENED_KEEP),
        "/dev/vda",
        Some(&plain_root),
        &a_payload(),
        true,
        None
    )
    .is_none());
    for answer in [copy::OPENED_ADD_KEY, copy::OPENED_TPM2] {
        assert_eq!(
            short_of(
                &opens_row(answer),
                "/dev/vda",
                Some(&plain_root),
                &a_payload(),
                true,
                None
            )
            .as_deref(),
            Some(copy::OPENED_KEYFILE_PLAIN),
            "{answer}"
        );
    }
    // A key file the home container already holds is refused the same way.
    // It was admitted while the root was open and does not stay admitted.
    let stale = CustomLayout {
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/var".to_string(),
            key: Key::File("/keys/var.key".into()),
        }],
        ..plain_root.clone()
    };
    assert_eq!(
        short_of(
            &opens_row(copy::OPENED_KEEP),
            "/dev/vda",
            Some(&stale),
            &a_payload(),
            true,
            None
        )
        .as_deref(),
        Some(copy::OPENED_KEYFILE_PLAIN)
    );
}

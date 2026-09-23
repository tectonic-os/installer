use super::*;

/// A plain partition and a container build their popups from different
/// branches, so the case covers both.
#[test]
fn a_planned_removal_offers_only_the_answer_that_undoes_it() {
    let plain = Partition {
        device: "/dev/vda1".to_string(),
        fstype: "ext4".to_string(),
        ..Default::default()
    };
    let container = Partition {
        device: "/dev/vda2".to_string(),
        fstype: "crypto_LUKS".to_string(),
        ..Default::default()
    };
    let mut layout = CustomLayout::empty("/dev/vda");
    for partition in [&plain, &container] {
        let (items, actions) = super::partition_menu(partition, Some(&layout));
        assert!(
            items.iter().any(|item| item.label == copy::DELETE_PART),
            "{:?}",
            items.iter().map(|item| &item.label).collect::<Vec<_>>()
        );
        assert!(matches!(actions.last(), Some(PartAction::Delete)));
    }
    layout.deletes = vec!["/dev/vda1".to_string()];
    let (items, actions) = super::partition_menu(&plain, Some(&layout));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, copy::RESET_CHANGES);
    assert!(matches!(actions[0], PartAction::Reset));
    // The partition beside the removed one keeps its own popup.
    assert!(super::partition_menu(&container, Some(&layout)).0.len() > 1);
}

/// A blank disk still offers `Create partition`. The user cuts partitions
/// on exactly that disk most often.
#[test]
fn the_disk_menu_offers_clearing_only_when_there_is_something_to_clear() {
    let (items, actions) = super::disk_menu(0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, copy::CREATE_PART);
    assert!(matches!(actions[0], PartAction::Create));
    let (items, _) = super::disk_menu(3);
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, [copy::CREATE_PART, copy::CLEAR_PARTS]);
}

#[test]
fn a_created_size_is_refused_against_the_room_that_is_actually_there() {
    assert_eq!(super::size_short_of("50", 100), None);
    assert_eq!(super::size_short_of("100", 100), None);
    assert_eq!(
        super::size_short_of("101", 100).as_deref(),
        Some(copy::custom_too_big(100).as_str())
    );
    // An empty box returns a refusal holding no text. It blocks `Create`
    // and prints no line under the size row.
    assert_eq!(super::size_short_of("", 100).as_deref(), Some(""));
    // A partition of zero GB holds nothing, so zero is refused as well.
    assert_eq!(
        super::size_short_of("0", 100).as_deref(),
        Some(copy::custom_too_big(100).as_str())
    );
    // A disk with no room left says so instead of offering zero GB.
    assert!(copy::custom_too_big(0).contains("no room"));
}

/// The ESP in this layout sits below the reserve too, so the case proves
/// the rule weighs the root alone.
#[test]
fn a_created_root_below_the_image_reserve_is_refused() {
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        creates: vec![
            Created {
                gb: 2,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: String::new(),
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        layout_short_of(&layout, false, None, 20).as_deref(),
        Some(copy::custom_root_too_small(1, 20).as_str())
    );
    // A reserve of zero means no reserve is known, and the rule invents none.
    assert!(layout_short_of(&layout, false, None, 0).is_none());
}

/// `layout_short_of` refuses a manual container by reading the mounts, and
/// a create is not a mount. The create popup must therefore never offer
/// `luks`, or the refusal would never fire.
#[test]
fn a_created_partition_is_not_offered_luks() {
    assert!(copy::format_options().contains(&"luks"));
    assert!(!copy::plain_formats().contains(&"luks"));
    // `luks` is the only format the create popup drops.
    for format in copy::plain_formats() {
        assert!(copy::format_options().contains(&format), "{format}");
    }
}

/// Each refusal a manual layout can earn is paired here with the layout
/// that clears it. A rule that refused every layout would fail the pairing.
#[test]
fn a_manual_layout_is_refused_where_the_recipe_could_not_be_taken() {
    let layout = |mounts: &[(&str, &str)]| CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: mounts
            .iter()
            .enumerate()
            .map(|(at, (target, fstype))| CustomMount {
                partition: format!("/dev/vda{}", at + 1),
                passphrase: String::new(),
                target: target.to_string(),
                fstype: fstype.to_string(),
            })
            .collect(),
        opens: Vec::new(),
        ..Default::default()
    };
    assert_eq!(
        layout_short_of(&layout(&[("/", "ext4")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_ESP)
    );
    assert!(layout_short_of(
        &layout(&[("/boot/efi", "fat32"), ("/", "xfs")]),
        false,
        None,
        0
    )
    .is_none());
    assert_eq!(
        layout_short_of(
            &layout(&[("/boot/efi", "fat32"), ("/", "luks")]),
            false,
            None,
            0
        )
        .as_deref(),
        Some(copy::CUSTOM_LUKS_LATER)
    );
    assert_eq!(
        layout_short_of(
            &layout(&[("/boot/efi", "fat32"), ("/", "xfs")]),
            true,
            None,
            0
        )
        .as_deref(),
        Some(copy::CUSTOM_VERITY)
    );
    assert!(layout_short_of(
        &layout(&[("/boot/efi", "fat32"), ("/", "btrfs")]),
        true,
        None,
        0
    )
    .is_none());
    // The verity rule weighs the filesystems this install writes. A root
    // left unformatted is not weighed, because the install does not write it.
    assert!(layout_short_of(
        &layout(&[("/boot/efi", "fat32"), ("/", "unformatted")]),
        true,
        None,
        0
    )
    .is_none());
}

/// The two fixtures differ only in the format the mount row holds. A mount
/// row holding `unformatted` states nothing once its point is taken away.
#[test]
fn unassign_takes_the_point_away_and_keeps_the_format() {
    let partition = Partition {
        device: "/dev/vda1".to_string(),
        size: "512M".to_string(),
        fstype: "vfat".to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    let layout = |fstype: &str| {
        Some(CustomLayout {
            disk: "/dev/vda".to_string(),
            mounts: vec![CustomMount {
                partition: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                fstype: fstype.to_string(),

                passphrase: String::new(),
            }],
            opens: Vec::new(),
            ..Default::default()
        })
    };
    let mut held = layout("fat32");
    place_unassign(&mut held, &partition);
    let mount = &held.as_ref().expect("the format stays").mounts[0];
    assert_eq!(mount.target, "");
    assert_eq!(mount.fstype, "fat32");
    let mut bare = layout("unformatted");
    place_unassign(&mut bare, &partition);
    // The manual layout survives with no mount rows left. An empty manual
    // layout still means manual, and `None` would mean whole disk.
    assert!(bare.as_ref().is_some_and(|layout| layout.mounts.is_empty()));
}

/// The partition fixtures carry the real GPT type GUIDs, `c12a7328` for an
/// ESP and `0fc63daf` for a Linux filesystem. The table's type column reads
/// the GUID, so a made-up one would leave that cell empty.
#[test]
fn the_layout_table_reads_every_answer_back_on_its_row() {
    let partition = |device: &str, fstype: &str, parttype: &str| Partition {
        device: device.to_string(),
        size: "4G".to_string(),
        fstype: fstype.to_string(),
        label: String::new(),
        parttype: parttype.to_string(),
        uuid: String::new(),
    };
    let rows = vec![
        partition("/dev/vda1", "vfat", "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"),
        partition(
            "/dev/vda2",
            "crypto_LUKS",
            "0fc63daf-8483-4772-8e79-3d69d8477de4",
        ),
        partition("/dev/vda3", "xfs", "0fc63daf-8483-4772-8e79-3d69d8477de4"),
    ];
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![
            CustomMount {
                partition: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),

                passphrase: String::new(),
            },
            CustomMount {
                partition: "/dev/vda3".to_string(),
                target: "/var".to_string(),
                fstype: "ext4".to_string(),

                passphrase: String::new(),
            },
        ],
        opens: vec![LuksOpen {
            partition: "/dev/vda2".to_string(),
            target: "/".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    let disks = vec![("/dev/vda".to_string(), "64G".to_string())];
    let parts = vec![("/dev/vda".to_string(), rows)];
    let table = layout_table(
        &disks,
        "/dev/vda",
        Some(&layout),
        &parts,
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(table.rows.len(), 4);
    assert!(matches!(
        table.kinds.as_slice(),
        [
            RowKind::Disk(_),
            RowKind::Part { index: 0 },
            RowKind::Part { index: 1 },
            RowKind::Part { index: 2 }
        ]
    ));
    let said = |row: usize, column: usize| table.rows[row][column].text().to_string();
    assert_eq!(said(0, 0), "◉ /dev/vda");
    assert_eq!(said(0, 1), "64G");
    assert!(table.rows[0][0].answered(), "the chosen disk reads white");
    assert_eq!(said(1, 2), "fat32");
    assert!(table.rows[1][2].answered());
    assert_eq!(said(1, 3), copy::FORMAT_TICK);
    assert!(table.rows[1][3].answered());
    assert_eq!(said(1, 4), copy::EFI_CELL);
    assert!(table.rows[1][4].answered());
    assert_eq!(said(1, 5), "/boot/efi");
    // An open is not a format, so the format column stays unticked while
    // the container's mount point reads `/`.
    assert_eq!(said(2, 2), copy::LUKS_OPEN);
    assert!(table.rows[2][2].answered());
    assert!(!table.rows[2][3].answered());
    assert_eq!(said(2, 5), "/");
    assert_eq!(said(3, 2), "ext4");
    assert!(table.rows[3][2].answered());
    assert!(table.rows[3][3].answered());
    assert_eq!(said(3, 4), "linux");
    assert_eq!(said(3, 5), "/var");
    // A partition the user assigned without formatting keeps the filesystem
    // lsblk reported. That cell stays dim and the format column stays blank.
    let untouched = CustomLayout {
        mounts: vec![CustomMount {
            partition: "/dev/vda1".to_string(),
            target: "/boot/efi".to_string(),
            fstype: "unformatted".to_string(),

            passphrase: String::new(),
        }],
        ..layout.clone()
    };
    let kept = layout_table(
        &disks,
        "/dev/vda",
        Some(&untouched),
        &parts,
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(kept.rows[1][2].text(), "vfat");
    assert!(!kept.rows[1][2].answered());
    assert!(!kept.rows[1][3].answered());
    assert_eq!(kept.rows[1][5].text(), "/boot/efi");
    // A whole-disk layout draws preview rows the cursor cannot rest on.
    let automatic = layout_table(
        &disks,
        "/dev/vda",
        None,
        &[],
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(automatic.rows[0][0].text(), "◉ /dev/vda");
    assert!(automatic.rows[0][0].answered());
    assert_eq!(automatic.rows.len(), 4, "esp, /boot, root");
    assert!(automatic
        .kinds
        .iter()
        .skip(1)
        .all(|kind| matches!(kind, RowKind::Preview)));
    assert!(automatic.selectable.iter().skip(1).all(|ok| !ok));
    assert_eq!(automatic.rows[1][5].text(), "/boot/efi");
    assert_eq!(automatic.rows[2][5].text(), "/boot");
    assert_eq!(automatic.rows[3][5].text(), "/");
}

/// 68 is what `size_gb` reads from the `68.7 GB` disk string the suite's
/// fixtures carry. The payload overrides `bootloader` to `systemd`, because
/// any other bootloader adds a `/boot` row and moves every size below it.
#[test]
fn the_automatic_plan_computes_the_sizes_the_screen_shows() {
    assert_eq!(size_gb("68 GB"), Some(68));
    assert_eq!(size_gb("68.7 GB"), Some(68));
    assert_eq!(size_gb("64G"), Some(64));
    let payload = Payload {
        bootloader: "systemd".to_string(),
        ..a_payload()
    };
    let plan = automatic_rows(&payload, "", Some(68), false, true);
    let said: Vec<(&str, &str)> = plan
        .iter()
        .map(|(name, size, ..)| {
            (
                name.trim_start_matches(['\u{251c}', '\u{2514}', '\u{2500}', '\u{2502}', ' ']),
                size.as_str(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [("esp", "2.0 GB"), ("luks", "66.0 GB"), ("root", "66.0 GB")]
    );
}

/// The home size comes out of the root. The container still reads the whole
/// 66.0 GB it had, and the root drops by the 20 GB the home takes.
#[test]
fn a_separate_home_is_cut_out_of_the_container() {
    let payload = Payload {
        bootloader: "systemd".to_string(),
        ..a_payload()
    };
    let plan = automatic_rows(&payload, "20 GB", Some(68), true, true);
    let said: Vec<(&str, &str)> = plan
        .iter()
        .map(|(name, size, ..)| {
            (
                name.trim_start_matches(['\u{251c}', '\u{2514}', '\u{2500}', '\u{2502}', ' ']),
                size.as_str(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("esp", "2.0 GB"),
            ("luks", "66.0 GB"),
            ("root", "46.0 GB"),
            ("home", "20.0 GB"),
        ]
    );
}

/// The `separate home` answer gates the home row alone. A size left behind
/// by a taken-back answer still shrinks the root row, so a caller that shares
/// the root must blank the size it passes.
#[test]
fn a_size_without_a_separate_home_is_not_a_home_partition() {
    let payload = Payload {
        bootloader: "systemd".to_string(),
        ..a_payload()
    };
    let plan = automatic_rows(&payload, "20", Some(68), false, false);
    assert!(
        plan.iter().all(|(name, ..)| !name.contains("home")),
        "{plan:?}"
    );
    // The same size under a `separate home` answer draws the home row.
    let sized = automatic_rows(&payload, "20", Some(68), true, false);
    assert!(
        sized.iter().any(|(name, ..)| name.contains("home")),
        "{sized:?}"
    );
}

/// `RowKind::Part` numbers each partition within its own disk, so two disks
/// both hold an index 0. Only the chosen disk's rows are selectable, so an
/// index never resolves against the other disk's partition list.
#[test]
fn only_the_chosen_disks_partitions_can_be_answered() {
    let partition = |device: &str, fstype: &str| Partition {
        device: device.to_string(),
        size: "4G".to_string(),
        fstype: fstype.to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    let disks = vec![
        ("/dev/sda".to_string(), "16 GB".to_string()),
        ("/dev/vda".to_string(), "64 GB".to_string()),
    ];
    let parts = vec![
        ("/dev/sda".to_string(), vec![partition("/dev/sda1", "vfat")]),
        (
            "/dev/vda".to_string(),
            vec![
                partition("/dev/vda1", "vfat"),
                partition("/dev/vda2", "ext4"),
            ],
        ),
    ];
    let on_vda = layout_table(
        &disks,
        "/dev/vda",
        Some(&CustomLayout::empty("/dev/vda")),
        &parts,
        &a_payload(),
        "",
        false,
        false,
    );
    // The five rows run sda, sda1, vda, vda1, vda2.
    assert_eq!(on_vda.rows.len(), 5);
    assert_eq!(on_vda.selectable, [true, false, true, true, true]);
    assert!(matches!(on_vda.kinds[1], RowKind::Part { index: 0 }));
    assert!(matches!(on_vda.kinds[3], RowKind::Part { index: 0 }));
    assert!(matches!(on_vda.kinds[4], RowKind::Part { index: 1 }));
    let on_sda = layout_table(
        &disks,
        "/dev/sda",
        Some(&CustomLayout::empty("/dev/sda")),
        &parts,
        &a_payload(),
        "",
        false,
        false,
    );
    // The same five rows answer with `/dev/sda` chosen instead.
    assert_eq!(on_sda.rows.len(), 5);
    assert_eq!(on_sda.selectable, [true, true, true, false, false]);
    assert!(matches!(on_sda.kinds[1], RowKind::Part { index: 0 }));
    assert!(matches!(on_sda.kinds[3], RowKind::Part { index: 0 }));
}

/// A table holding no selectable row and an empty table both answer row 0.
#[test]
fn the_table_opens_on_a_row_a_person_can_rest_on() {
    assert_eq!(clamp_row(&[true, true, true], 1), 1);
    assert_eq!(clamp_row(&[true, false, true], 1), 2);
    assert_eq!(clamp_row(&[true, false, false], 1), 0);
    assert_eq!(clamp_row(&[false, false], 0), 0);
    assert_eq!(clamp_row(&[], 3), 0);
}

/// Clearing the mount point leaves the mount row in place and half
/// answered. `layout_short_of` then blocks `Install` on `CUSTOM_UNMOUNTED`.
#[test]
fn a_format_that_does_not_fit_the_mount_point_clears_it() {
    let partition = |fstype: &str| Partition {
        device: "/dev/vda1".to_string(),
        size: "512M".to_string(),
        fstype: fstype.to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    let held = |target: &str, fstype: &str| {
        Some(CustomLayout {
            disk: "/dev/vda".to_string(),
            mounts: vec![CustomMount {
                partition: "/dev/vda1".to_string(),
                target: target.to_string(),
                fstype: fstype.to_string(),

                passphrase: String::new(),
            }],
            opens: Vec::new(),
            ..Default::default()
        })
    };
    let mut esp = held("/boot/efi", "fat32");
    place_format(&mut esp, "/dev/vda", &partition("vfat"), "ext4");
    assert_eq!(esp.as_ref().unwrap().mounts[0].target, "");
    let mut keep = held("/", "ext4");
    place_format(&mut keep, "/dev/vda", &partition("ext4"), copy::NO_FORMAT);
    assert_eq!(keep.as_ref().unwrap().mounts[0].target, "/");
    assert_eq!(keep.as_ref().unwrap().mounts[0].fstype, "unformatted");
}

/// `Key::Data` answers the same rule as `Key::File`. Only a passphrase is
/// usable where no container encrypts the root.
#[test]
fn a_key_file_is_usable_only_where_a_container_encrypts_the_root() {
    assert!(usable_key(&Key::Passphrase("x".to_string()), false));
    assert!(!usable_key(&Key::File(PathBuf::from("/run/key")), false));
    assert!(!usable_key(&Key::Data(b"key".to_vec()), false));
    assert!(usable_key(&Key::File(PathBuf::from("/run/key")), true));
    assert!(usable_key(&Key::Data(b"key".to_vec()), true));
}

/// The argument is `plain_root`, so `true` is the layout with no encrypted
/// root and it draws one method. `false` draws both.
#[test]
fn the_key_file_method_is_hidden_where_the_root_is_not_encrypted() {
    let both = key_methods(false);
    assert_eq!(both.len(), 2);
    assert!(both.iter().all(|method| method.available));

    let refused = key_methods(true);
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].label, copy::KEY_PASSPHRASE);
}

#[test]
fn root_encryption_is_refused_without_an_initramfs_witness() {
    let rows = kinds(true, false, true);
    assert!(rows[0].available);
    for row in &rows[1..] {
        assert!(!row.available, "{}", row.label);
        assert!(row.hidden, "{}", row.label);
        assert_eq!(row.detail, copy::NO_LUKS_INITRAMFS);
    }
    assert_eq!(
        ask_encryption(
            Some("luks-passphrase".to_string()),
            Some("opensesame".to_string()),
            None,
            &Prompt::silent(),
            true,
            false,
        )
        .err()
        .as_deref(),
        Some(copy::NO_LUKS_INITRAMFS)
    );

    assert_eq!(open_points(), ["/", "/var", "/var/home"]);
}

/// A PIN kind owes fields 4 and 5 while it draws rows `[0, 1, 4, 5]`, so a
/// field number and a row position are not the same number. Every kind opens
/// on the third row it draws, which `common::ui::opens_at` finds from the
/// field.
#[test]
fn the_luks_window_reopens_on_the_secret_the_kind_owes() {
    for kind in ["luks-passphrase", "tpm2-luks-passphrase", "tpm2-luks-pin"] {
        let field = luks_window_start(kind).expect("the kind owes a secret");
        assert_eq!(
            field,
            match Encryption::wants_pin(kind) {
                true => 4,
                false => 2,
            },
            "{kind} reopens on its own secret",
        );
        let rows = luks_window_rows(kind, true);
        assert!(rows.contains(&field), "{kind} draws the field it opens on");
        assert_eq!(
            common::ui::opens_at(&rows, field),
            2,
            "{kind} opens on the third row it draws, whichever field that is",
        );
    }
    // A kind that owes no secret answers the window outright.
    assert_eq!(luks_window_start("tpm2-luks"), None);
    assert_eq!(luks_window_start(NONE), None);
}

/// The size window submits on the key its field answers with. A measure field
/// returns `Changed` on enter, because the main form redraws the row it sizes
/// from that key, and `ask_size` read only `Took`, so every manual create was
/// dropped after the user typed a size and the plan never drew it. Measured
/// 2026-09-23 on the real cut run: the window closed, no created row appeared,
/// and the install refused with `still needs a / partition`.
///
/// This case pins the mapping and the guard alone. The form's own measure key
/// is proven end to end by the manual cut in
/// `measurements/manual-cut-2026-09-23.md`, which is where a `common` pin bump
/// that changed what a measure's enter returns would show.
#[test]
fn the_size_window_submits_on_the_key_its_field_answers_with() {
    use common::ui::Filled;
    // The measure field's own answer is the submit. Reading only `Took` here
    // dropped every size the user typed.
    assert!(super::submitted(Filled::Changed(0)));
    // The action-button answer still submits, so an overlay that grows a
    // button later keeps working.
    assert!(super::submitted(Filled::Took(0)));
    // Cancel answers nothing.
    assert!(!super::submitted(Filled::Left));
    // The window's guard still refuses a zero, a number above the room, and
    // an empty box, which the loop re-asks and reopens with the reason.
    assert!(super::size_short_of("0", 100).is_some());
    assert_eq!(super::size_short_of("20", 100), None);
    assert!(super::size_short_of("", 100).is_some());
}

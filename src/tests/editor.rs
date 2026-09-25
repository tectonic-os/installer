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
        let (items, actions) = super::partition_menu(partition, Some(&layout), true, false);
        assert!(
            items.iter().any(|item| item.label == copy::DELETE_PART),
            "{:?}",
            items.iter().map(|item| &item.label).collect::<Vec<_>>()
        );
        assert!(matches!(actions.last(), Some(PartAction::Delete)));
    }
    layout.deletes = vec!["/dev/vda1".to_string()];
    let (items, actions) = super::partition_menu(&plain, Some(&layout), true, false);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, copy::RESET_CHANGES);
    assert!(matches!(actions[0], PartAction::Reset));
    // The partition beside the removed one keeps its own popup.
    assert!(
        super::partition_menu(&container, Some(&layout), true, false)
            .0
            .len()
            > 1
    );
}

/// A composefs target binds `/var` from the root before fstab units run, so
/// no Assign list may offer `/var` itself. The points under it stay, because
/// nothing binds them.
#[test]
fn a_composefs_target_offers_no_var_to_assign() {
    let points = |menu: &[common::ui::MenuItem]| -> Vec<String> {
        menu.iter()
            .find(|item| item.label == copy::ASSIGN)
            .map(|item| item.children.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|child| child != copy::UNASSIGN)
            .collect()
    };
    let opened = Partition {
        device: "/dev/vda2".to_string(),
        fstype: "crypto_LUKS".to_string(),
        ..Default::default()
    };
    let mut layout = CustomLayout::empty("/dev/vda");
    layout.opens.push(LuksOpen {
        partition: opened.device.clone(),
        target: "/var".to_string(),
        key: Key::Passphrase("opensesame".to_string()),
    });
    let sealed = points(&super::partition_menu(&opened, Some(&layout), true, true).0);
    assert!(!sealed.iter().any(|point| point == "/var"), "{sealed:?}");
    assert!(
        sealed.iter().any(|point| point == "/var/home"),
        "{sealed:?}"
    );
    let plain = points(&super::partition_menu(&opened, Some(&layout), true, false).0);
    assert!(plain.iter().any(|point| point == "/var"), "{plain:?}");

    let create = Created {
        gb: 20,
        ..Default::default()
    };
    let sealed = points(&super::created_menu(&create, true, true).0);
    assert!(!sealed.iter().any(|point| point == "/var"), "{sealed:?}");
    assert!(
        sealed.iter().any(|point| point == "/var/home"),
        "{sealed:?}"
    );
    let plain = points(&super::created_menu(&create, true, false).0);
    assert!(plain.iter().any(|point| point == "/var"), "{plain:?}");

    // The drawn table wires the payload's sealing into every create it draws.
    let mut sealed = a_payload();
    sealed.composefs = true;
    let held = CustomLayout {
        disk: "/dev/vda".to_string(),
        creates: vec![Created {
            gb: 20,
            ..Default::default()
        }],
        ..Default::default()
    };
    let scan = scan_of(
        &[("/dev/vda", "64G")],
        &[("/dev/vda", vec![part("/dev/vda1", "vfat")])],
    );
    let table = layout_table(&scan, "/dev/vda", Some(&held), &sealed, "", false, false);
    let at = table
        .kinds
        .iter()
        .position(|kind| matches!(kind, RowKind::Created { .. }))
        .expect("a created row");
    let drawn = points(&table.menus[at]);
    assert!(!drawn.iter().any(|point| point == "/var"), "{drawn:?}");
    assert!(drawn.iter().any(|point| point == "/var/home"), "{drawn:?}");
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
    assert_eq!(super::create_short_of("0", "50", 100), None);
    assert_eq!(super::create_short_of("0", "100", 100), None);
    // The offset is spent before the size, so together they must fit.
    assert_eq!(super::create_short_of("10", "90", 100), None);
    assert_eq!(
        super::create_short_of("10", "91", 100).as_deref(),
        Some(copy::custom_too_big(100).as_str())
    );
    assert_eq!(
        super::create_short_of("0", "101", 100).as_deref(),
        Some(copy::custom_too_big(100).as_str())
    );
    // An empty size returns a refusal holding no text. It blocks the window
    // and prints no line under the size row.
    assert_eq!(super::create_short_of("0", "", 100).as_deref(), Some(""));
    // A partition of zero GB holds nothing, so zero is refused as well.
    assert_eq!(
        super::create_short_of("0", "0", 100).as_deref(),
        Some(copy::custom_too_big(100).as_str())
    );
    // An empty offset is read as zero, which is where the region starts.
    assert_eq!(super::create_short_of("", "50", 100), None);
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
                ..Default::default()
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
                ..Default::default()
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
    let scan = scan_of(&[("/dev/vda", "64G")], &[("/dev/vda", rows)]);
    let table = layout_table(
        &scan,
        "/dev/vda",
        Some(&layout),
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
        &scan,
        "/dev/vda",
        Some(&untouched),
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
    let automatic = layout_table(&scan, "/dev/vda", None, &a_payload(), "", false, false);
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
        [
            ("EFI-SYSTEM", "2.0 GB"),
            ("root (LUKS)", "66.0 GB"),
            ("root", "66.0 GB"),
        ]
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
            ("EFI-SYSTEM", "2.0 GB"),
            ("root (LUKS)", "66.0 GB"),
            ("root", "46.0 GB"),
            ("var", "20.0 GB"),
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
        plan.iter().all(|(name, ..)| !name.contains("var")),
        "{plan:?}"
    );
    // The same size under a `separate home` answer draws the var row.
    let sized = automatic_rows(&payload, "20", Some(68), true, false);
    assert!(
        sized.iter().any(|(name, ..)| name.contains("var")),
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
    let disks = [("/dev/sda", "16 GB"), ("/dev/vda", "64 GB")];
    let scan = scan_of(
        &disks,
        &[
            ("/dev/sda", vec![partition("/dev/sda1", "vfat")]),
            (
                "/dev/vda",
                vec![
                    partition("/dev/vda1", "vfat"),
                    partition("/dev/vda2", "ext4"),
                ],
            ),
        ],
    );
    let on_vda = layout_table(
        &scan,
        "/dev/vda",
        Some(&CustomLayout::empty("/dev/vda")),
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
        &scan,
        "/dev/sda",
        Some(&CustomLayout::empty("/dev/sda")),
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

/// Every disk draws what it holds before it is chosen, so the screen answers
/// which disk to take before taking it. The chosen disk's plan replaces its
/// own partitions; the other disks keep theirs, grey, and the systems a
/// partition carries draw as child rows under it.
#[test]
fn every_disk_shows_what_it_holds_before_it_is_chosen() {
    let windows = Partition {
        device: "/dev/sda1".to_string(),
        size: "100G".to_string(),
        fstype: "ntfs".to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    let esp = Partition {
        device: "/dev/vda1".to_string(),
        fstype: "vfat".to_string(),
        ..windows.clone()
    };
    let scan = Scan {
        unread: Vec::new(),
        disks: vec![
            DiskScan {
                device: "/dev/sda".to_string(),
                detail: "256 GB".to_string(),
                partitions: vec![windows],
                carries: true,
                table: Ok(DiskTable::default()),
                labels: vec![("/dev/sda1".to_string(), vec![label("Windows")])],
                keys: Discovered::default(),
            },
            DiskScan {
                device: "/dev/vda".to_string(),
                detail: "64 GB".to_string(),
                partitions: vec![esp],
                carries: true,
                table: Ok(DiskTable::default()),
                labels: vec![("/dev/vda1".to_string(), vec![label("fedora")])],
                keys: Discovered::default(),
            },
        ],
    };
    // An automatic layout takes the whole chosen disk, so `vda`'s partition is
    // replaced by the plan and `sda` keeps what it holds.
    let table = layout_table(&scan, "/dev/vda", None, &a_payload(), "", false, false);
    let said = |row: usize, column: usize| table.rows[row][column].text().to_string();
    // The rows run sda, sda1, `Windows`, vda, then the plan's three rows.
    assert_eq!(table.rows.len(), 7);
    assert_eq!(
        table.selectable,
        [true, false, false, true, false, false, false]
    );
    assert_eq!(said(1, 0), "\u{2514}\u{2500} /dev/sda1");
    assert_eq!(said(2, 0), "   \u{2514}\u{2500} Windows");
    assert!(!table.rows[2][0].answered(), "a carried system reads grey");
    assert!(matches!(table.kinds[2], RowKind::Preview));
    assert_eq!(said(4, 4), "efi", "the chosen disk draws its plan");
    assert_eq!(said(4, 0), "\u{251c}\u{2500} EFI-SYSTEM");
    // A manual layout answers on the chosen disk alone, and every disk still
    // shows what it holds.
    let table = layout_table(
        &scan,
        "/dev/vda",
        Some(&CustomLayout::empty("/dev/vda")),
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(table.selectable, [true, false, false, true, true, false]);
    assert_eq!(table.rows.len(), 6);
    assert_eq!(table.rows[4][0].text(), "\u{2514}\u{2500} /dev/vda1");
    assert_eq!(table.rows[5][0].text(), "   \u{2514}\u{2500} fedora");
}

/// The plan's ESP carries the entries the image writes, and an entry whose
/// system the plan takes gives way to them. An entry the plan leaves alone
/// stays grey, and each entry the image writes draws white.
#[test]
fn the_plan_draws_the_entries_the_image_writes_on_its_esp() {
    let scan = Scan {
        unread: Vec::new(),
        disks: vec![DiskScan {
            device: "/dev/vda".to_string(),
            detail: "64 GB".to_string(),
            partitions: vec![part("/dev/vda1", "vfat"), part("/dev/vda2", "ext4")],
            carries: false,
            table: Ok(DiskTable::default()),
            labels: vec![(
                "/dev/vda1".to_string(),
                vec![
                    Label {
                        name: "fedora".to_string(),
                        link: "/dev/vda2".to_string(),
                    },
                    Label {
                        name: "Microsoft".to_string(),
                        link: "/dev/sda2".to_string(),
                    },
                ],
            )],
            keys: Discovered::default(),
        }],
    };
    let mut held = CustomLayout::empty("/dev/vda");
    held.mounts.push(CustomMount {
        partition: "/dev/vda1".to_string(),
        target: "/boot/efi".to_string(),
        fstype: "unformatted".to_string(),
        passphrase: String::new(),
    });
    held.mounts.push(CustomMount {
        partition: "/dev/vda2".to_string(),
        target: "/boot".to_string(),
        fstype: "ext4".to_string(),
        passphrase: String::new(),
    });
    let payload = a_payload_writing(&["fedora"]);
    let table = layout_table(&scan, "/dev/vda", Some(&held), &payload, "", false, false);
    let said: Vec<(String, bool)> = table
        .rows
        .iter()
        .map(|row| (row[0].text().to_string(), row[0].answered()))
        .collect();
    // The disk, its two partitions, then their systems. The old `fedora`
    // entry is replaced by the one the image writes, drawn white, and the
    // Windows entry the plan leaves alone stays grey.
    assert_eq!(
        said,
        [
            ("\u{25c9} /dev/vda".to_string(), true),
            ("\u{251c}\u{2500} /dev/vda1".to_string(), true),
            ("   \u{251c}\u{2500} Microsoft".to_string(), false),
            ("   \u{2514}\u{2500} fedora".to_string(), true),
            ("\u{2514}\u{2500} /dev/vda2".to_string(), true),
        ]
    );
    // A format erases every entry the walk found, so the plan draws only what
    // the image writes.
    held.mounts[0].fstype = "vfat".to_string();
    let table = layout_table(&scan, "/dev/vda", Some(&held), &payload, "", false, false);
    let systems: Vec<(&str, bool)> = table.rows[2..3]
        .iter()
        .map(|row| (row[0].text(), row[0].answered()))
        .collect();
    assert_eq!(systems, [("   \u{2514}\u{2500} fedora", true)]);
}

/// A partition the plan formats vfat takes the ESP role, so the entries the
/// image writes draw under it even though the walk read another filesystem
/// there.
#[test]
fn a_partition_the_plan_formats_vfat_takes_the_esp_role() {
    let scan = Scan {
        unread: Vec::new(),
        disks: vec![DiskScan {
            device: "/dev/vda".to_string(),
            detail: "64 GB".to_string(),
            partitions: vec![part("/dev/vda1", "ext4")],
            carries: false,
            table: Ok(DiskTable::default()),
            labels: Vec::new(),
            keys: Discovered::default(),
        }],
    };
    let mut held = CustomLayout::empty("/dev/vda");
    held.mounts.push(CustomMount {
        partition: "/dev/vda1".to_string(),
        target: "/boot/efi".to_string(),
        fstype: "vfat".to_string(),
        passphrase: String::new(),
    });
    let payload = a_payload_writing(&["fedora"]);
    let table = layout_table(&scan, "/dev/vda", Some(&held), &payload, "", false, false);
    let systems: Vec<(&str, bool)> = table.rows[2..]
        .iter()
        .map(|row| (row[0].text(), row[0].answered()))
        .collect();
    assert_eq!(systems, [("   \u{2514}\u{2500} fedora", true)]);
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

    assert_eq!(open_points(false), ["/", "/var", "/var/home"]);
    // A composefs target leaves `/var` itself out; the points under it stay.
    assert_eq!(open_points(true), ["/", "/var/home"]);
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

/// The create window submits on the key its fields answer with. A measure
/// field returns `Changed` on enter, because the main form redraws the row it
/// sizes from that key, so this window reads that same key as its submit.
/// Reading only `Took` here would drop every size the user types.
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
    // The window's guard still refuses a zero, an offset and size that
    // together overrun the room, and an empty size, which the loop re-asks
    // and reopens with the reason.
    assert!(super::create_short_of("0", "0", 100).is_some());
    assert_eq!(super::create_short_of("20", "80", 100), None);
    assert!(super::create_short_of("20", "81", 100).is_some());
    assert!(super::create_short_of("0", "", 100).is_some());
}

/// The automatic plan's type words land under the `type` heading and its format
/// column stays empty. A plan row states what will exist, not a format answer.
/// A swapped pair draws every word under the wrong heading while a text-only
/// assertion still passes.
#[test]
fn the_automatic_plan_cells_land_under_their_own_headings() {
    let payload = Payload {
        bootloader: "grub2".to_string(),
        ..a_payload()
    };
    let scan = scan_of(&[("/dev/vda", "64G")], &[]);
    let table = layout_table(&scan, "/dev/vda", None, &payload, "", false, false);
    // The chosen disk draws first, then one row per planned partition.
    assert_eq!(
        table.rows[1..]
            .iter()
            .map(|row| row[4].text())
            .collect::<Vec<_>>(),
        ["efi", "linux", "linux"]
    );
    for row in 1..table.rows.len() {
        assert_eq!(
            table.rows[row][3].text(),
            "",
            "the format column holds no plan answer"
        );
    }
}

/// A planned partition is named from its mount point, in the words the
/// automatic plan already writes. A mount point the installer does not name
/// opens blank, and a name the plan already carries wins.
#[test]
fn a_planned_partition_is_named_from_its_mount_point() {
    assert_eq!(rename_default("/boot/efi", ""), "EFI-SYSTEM");
    assert_eq!(rename_default("/boot", ""), "boot");
    assert_eq!(rename_default("/", ""), "root");
    assert_eq!(rename_default("/var", ""), "var");
    assert_eq!(rename_default("/var/home", ""), "var");
    assert_eq!(rename_default("/swap", ""), "");
    assert_eq!(rename_default("/", "mine"), "mine");
    assert_eq!(mount_name("/boot/efi"), "EFI-SYSTEM");
}

/// A rename the plan carries draws on the partition's own row in place of the
/// label the partition holds, and the row reads white because it is an answer.
#[test]
fn a_renamed_partition_draws_its_new_name() {
    let partition = |device: &str| Partition {
        device: device.to_string(),
        size: "4G".to_string(),
        fstype: "ext4".to_string(),
        label: "old".to_string(),
        ..Default::default()
    };
    let scan = scan_of(
        &[("/dev/vda", "64G")],
        &[(
            "/dev/vda",
            vec![partition("/dev/vda1"), partition("/dev/vda2")],
        )],
    );
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        renames: vec![Rename {
            partition: "/dev/vda2".to_string(),
            label: "mine".to_string(),
        }],
        ..Default::default()
    };
    let table = layout_table(
        &scan,
        "/dev/vda",
        Some(&layout),
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(table.rows[1][0].text(), "\u{251c}\u{2500} old (/dev/vda1)");
    assert!(!table.rows[1][0].answered(), "an unrenamed row stays dim");
    assert_eq!(table.rows[2][0].text(), "\u{2514}\u{2500} mine (/dev/vda2)");
    assert!(table.rows[2][0].answered(), "a renamed row reads white");
}

/// A planned partition offers the same name its existing neighbours do. A
/// planned partition keeps no `Reset`, so its name is taken back by dropping
/// the create, as every other answer on that row is.
#[test]
fn a_planned_partition_offers_rename() {
    let scan = scan_of(&[("/dev/vda", "64G")], &[]);
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        creates: vec![Created {
            gb: 10,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let table = layout_table(
        &scan,
        "/dev/vda",
        Some(&layout),
        &a_payload(),
        "",
        false,
        false,
    );
    assert!(matches!(table.kinds[1], RowKind::Created { index: 0 }));
    assert_eq!(
        table.actions[1]
            .iter()
            .filter(|action| matches!(action, PartAction::Rename))
            .count(),
        1
    );
    let named = CustomLayout {
        creates: vec![Created {
            gb: 10,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            label: "root".to_string(),
            ..Default::default()
        }],
        ..layout.clone()
    };
    let table = layout_table(
        &scan,
        "/dev/vda",
        Some(&named),
        &a_payload(),
        "",
        false,
        false,
    );
    assert_eq!(table.rows[1][0].text(), "\u{2514}\u{2500} root (/dev/vda1)");
}

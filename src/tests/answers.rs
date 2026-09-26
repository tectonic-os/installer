use super::*;

/// A planned partition is cut blank, so the user must name its format and
/// its mount point before the editor lets the layout be taken.
#[test]
fn a_created_partition_answers_the_same_rules() {
    let create = |gb: u64, target: &str, fstype: &str| Created {
        gb,
        target: target.to_string(),
        fstype: fstype.to_string(),
        ..Default::default()
    };
    let held = |creates: Vec<Created>| CustomLayout {
        disk: "/dev/vda".to_string(),
        creates,
        ..Default::default()
    };
    // The format check runs before the mount point check, so a blank planned
    // partition is refused for its filesystem.
    assert_eq!(
        layout_short_of(&held(vec![create(20, "/", "")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_UNFORMATTED)
    );
    // A planned partition with no mount point is a half answer.
    assert_eq!(
        layout_short_of(&held(vec![create(20, "", "ext4")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_UNMOUNTED)
    );
    // A layout built only from planned partitions is a whole answer.
    let whole = held(vec![
        create(2, "/boot/efi", "fat32"),
        create(60, "/", "btrfs"),
    ]);
    assert!(layout_short_of(&whole, false, None, 0).is_none());
    assert!(layout_short_of(&whole, true, None, 0).is_none());
    // A planned root answers the same fs-verity rule a reformatted root
    // answers. A sealed deployment cannot boot from xfs.
    let xfs = held(vec![
        create(2, "/boot/efi", "fat32"),
        create(60, "/", "xfs"),
    ]);
    assert!(layout_short_of(&xfs, false, None, 0).is_none());
    assert_eq!(
        layout_short_of(&xfs, true, None, 0).as_deref(),
        Some(copy::CUSTOM_VERITY)
    );
    // Planned and existing partitions share one mount point list.
    let mut clash = whole.clone();
    clash.mounts.push(CustomMount {
        partition: "/dev/vda9".to_string(),
        target: "/".to_string(),
        fstype: "ext4".to_string(),
        passphrase: String::new(),
    });
    assert_eq!(
        layout_short_of(&clash, false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
    // A layout of planned partitions alone still owes an ESP.
    assert_eq!(
        layout_short_of(&held(vec![create(60, "/", "btrfs")]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_ESP)
    );
}

/// A planned partition takes the `/swap` translation on its own branch in
/// `complete`, so this case covers that branch as well as the node.
#[test]
fn a_created_partition_reaches_the_recipe_as_the_node_it_got() {
    let mut layout = CustomLayout::empty("/dev/vda");
    layout.creates = vec![
        Created {
            gb: 2,
            target: "/boot/efi".to_string(),
            fstype: "fat32".to_string(),
            device: "/dev/vda1".to_string(),
            ..Default::default()
        },
        Created {
            gb: 8,
            target: "/swap".to_string(),
            fstype: "swap".to_string(),
            device: "/dev/vda2".to_string(),
            ..Default::default()
        },
    ];
    let root = scratch("created-recipe");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
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
        layout: Some(layout),
    };
    let done = complete(&recipe, &answers).expect("a created-partition recipe");
    let mounts = json::items(&done, "customMounts");
    let said: Vec<(String, String, String)> = mounts
        .iter()
        .map(|mount| {
            (
                json::text(mount, "partition").unwrap_or_default(),
                json::text(mount, "target").unwrap_or_default(),
                json::text(mount, "fstype").unwrap_or_default(),
            )
        })
        .collect();
    assert!(
        said.contains(&(
            "/dev/vda1".to_string(),
            "/boot/efi".to_string(),
            "fat32".to_string()
        )),
        "{said:?}"
    );
    assert!(
        said.contains(&(
            "/dev/vda2".to_string(),
            "swap".to_string(),
            "swap".to_string()
        )),
        "{said:?}"
    );
}

/// A layout that cleared the whole disk once summarised as an empty list,
/// under a sentence about formatting. This case guards that regression.
#[test]
fn the_confirmation_names_every_partition_it_will_remove() {
    let payload = a_payload();
    let answers = Answers {
        disk: "/dev/sda".to_string(),
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
        layout: Some(CustomLayout {
            disk: "/dev/sda".to_string(),
            // Clearing the disk is what leaves the mounts and the opens empty.
            deletes: vec![
                "/dev/sda1".to_string(),
                "/dev/sda2".to_string(),
                "/dev/sda3".to_string(),
            ],
            creates: vec![
                Created {
                    gb: 2,
                    target: "/boot/efi".to_string(),
                    fstype: "fat32".to_string(),
                    device: String::new(),
                    ..Default::default()
                },
                Created {
                    gb: 400,
                    target: "/".to_string(),
                    fstype: "btrfs".to_string(),
                    device: String::new(),
                    ..Default::default()
                },
            ],
            esp: vec![EspRemoval {
                partition: "/dev/sda1".to_string(),
                entries: vec!["fedora".to_string()],
            }],
            ..Default::default()
        }),
    };
    let said = answers.summary(&payload);
    let flat: String = said
        .iter()
        .map(|(label, value)| format!("{label} {value}\n"))
        .collect();
    for device in ["/dev/sda1", "/dev/sda2", "/dev/sda3"] {
        assert!(
            flat.contains(device),
            "{device} is not on the screen:\n{flat}"
        );
    }
    // The summary marks a removal DELETED, which is louder than format.
    assert_eq!(flat.matches("DELETED").count(), 3, "{flat}");
    // An ESP entry the plan replaces is named under the same `removed` row,
    // because its system goes with the partitions above it.
    assert!(
        flat.contains(&copy::summary_esp("fedora", "/dev/sda1")),
        "{flat}"
    );
    // A planned partition appears by its mount point.
    assert!(flat.contains("400 GB"), "{flat}");
    assert!(flat.contains("/boot/efi"), "{flat}");
    // The confirmation question counts the removals.
    let asked = copy::removing_partitions("/dev/sda", 3);
    assert!(asked.contains('3') && asked.contains("DELETE"), "{asked}");
    assert!(copy::removing_partitions("/dev/sda", 1).starts_with("DELETE 1 partition "));
}

/// The recipe carries the PIN in its own field. The passphrase stays empty,
/// because fisherman generates this kind's recovery key itself.
#[test]
fn the_completed_recipe_carries_the_pin() {
    let root = scratch("complete-pin");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "deb2".to_string(),
        user: "tect".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: "tpm2-luks-pin".to_string(),
            passphrase: String::new(),
            pin: "4321".to_string(),
        },
        data: Data::default(),
        layout: None,
    };
    let done = complete(&recipe, &answers).expect("the person's half goes in");
    let encryption = json::field(&done, "encryption").expect("an encryption");
    assert_eq!(
        json::text(encryption, "type").as_deref(),
        Some("tpm2-luks-pin")
    );
    assert_eq!(json::text(encryption, "pin").as_deref(), Some("4321"));
    assert_eq!(json::text(encryption, "passphrase").as_deref(), Some(""));
    let _ = std::fs::remove_dir_all(&root);
}

/// The editor lets the user leave a layout half answered, so the gate reads
/// the layout itself for a root and for a mount point claimed twice.
#[test]
fn custom_layouts_need_one_root_and_unique_mount_points() {
    let layout = |targets: &[&str]| CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: targets
            .iter()
            .enumerate()
            .map(|(at, target)| CustomMount {
                partition: format!("/dev/vda{}", at + 1),
                passphrase: String::new(),
                target: target.to_string(),
                fstype: "ext4".to_string(),
            })
            .collect(),
        opens: Vec::new(),
        ..Default::default()
    };
    assert_eq!(
        layout_short_of(&layout(&["/var"]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_ROOT)
    );
    assert_eq!(
        layout_short_of(&layout(&["/", "/"]), false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
    assert!(layout_short_of(&layout(&["/boot/efi", "/", "/var"]), false, None, 0).is_none());
    // An opened container's target joins the same mount point list, so a
    // second `/` is a duplicate whichever list holds it.
    let mut twice = layout(&["/boot/efi", "/var"]);
    twice.opens.push(LuksOpen {
        partition: "/dev/vda9".to_string(),
        target: "/".to_string(),
        key: Key::Passphrase("x".to_string()),
    });
    assert!(layout_short_of(&twice, false, None, 0).is_none());
    twice.mounts.push(CustomMount {
        partition: "/dev/vda8".to_string(),
        target: "/".to_string(),
        fstype: "ext4".to_string(),

        passphrase: String::new(),
    });
    assert_eq!(
        layout_short_of(&twice, false, None, 0).as_deref(),
        Some(copy::CUSTOM_DUPLICATE)
    );
}

/// A container the user opened reaches fisherman as its `/dev/mapper` name.
/// The layout numbers the mappers in the order it opens them.
#[test]
fn an_opened_container_reaches_fisherman_as_its_mapper() {
    let root = scratch("opened-layout");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
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
        layout: Some(CustomLayout {
            disk: "/dev/vda".to_string(),
            mounts: vec![CustomMount {
                partition: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                fstype: "unformatted".to_string(),

                passphrase: String::new(),
            }],
            opens: vec![
                LuksOpen {
                    partition: "/dev/vda2".to_string(),
                    target: "/".to_string(),
                    key: Key::Passphrase("opensesame".to_string()),
                },
                LuksOpen {
                    partition: "/dev/vda3".to_string(),
                    target: "/var".to_string(),
                    key: Key::File(PathBuf::from("/run/keyfile")),
                },
            ],
            ..Default::default()
        }),
    };
    let done = complete(&recipe, &answers).expect("a custom recipe");
    let mounts = json::items(&done, "customMounts");
    assert_eq!(mounts.len(), 3);
    assert_eq!(
        json::text(&mounts[1], "partition").as_deref(),
        Some("/dev/mapper/tect-1")
    );
    assert_eq!(json::text(&mounts[1], "target").as_deref(), Some("/"));
    assert_eq!(
        json::text(&mounts[1], "fstype").as_deref(),
        Some("unformatted")
    );
    assert_eq!(
        json::text(&mounts[2], "partition").as_deref(),
        Some("/dev/mapper/tect-2")
    );
    assert_eq!(json::text(&mounts[2], "target").as_deref(), Some("/var"));
    // The installer holds the container device and the key. Neither reaches
    // the recipe.
    let said = done.render();
    for secret in ["/dev/vda2", "/dev/vda3", "opensesame", "/run/keyfile"] {
        assert!(!said.contains(secret), "{secret} reached the recipe");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A `Debug` print of a key masks the secret, so a panic message or a test
/// failure never carries it. A key read out of an old system masks the same
/// way.
#[test]
fn a_passphrase_never_reads_back_out_of_a_key() {
    let said = format!("{:?}", Key::Passphrase("opensesame".to_string()));
    assert!(!said.contains("opensesame"), "{said}");
    let said = format!("{:?}", Key::Data(b"opensesame".to_vec()));
    assert!(!said.contains("opensesame"), "{said}");
}

#[test]
fn a_custom_layout_is_the_recipe_fisherman_takes() {
    let root = scratch("custom-layout");
    let recipe = root.join(RECIPE);
    let emitted = EMITTED.replace(
        "\"user\": { \"groups\": [\"sudo\"] },",
        "\"user\": { \"groups\": [\"sudo\"] },\n  \"varDisk\": { \"size\": \"20 GB\" },",
    );
    std::fs::write(&recipe, emitted).expect("a recipe");
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
        layout: Some(CustomLayout {
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
                CustomMount {
                    partition: "/dev/vda3".to_string(),
                    target: "/swap".to_string(),
                    fstype: "swap".to_string(),

                    passphrase: String::new(),
                },
            ],
            opens: Vec::new(),
            ..Default::default()
        }),
    };
    let done = complete(&recipe, &answers).expect("a custom recipe");
    let mounts = json::items(&done, "customMounts");
    assert_eq!(mounts.len(), 3);
    assert_eq!(
        json::text(&mounts[0], "partition").as_deref(),
        Some("/dev/vda1")
    );
    assert_eq!(
        json::text(&mounts[0], "fstype").as_deref(),
        Some("unformatted")
    );
    assert_eq!(json::text(&mounts[1], "target").as_deref(), Some("/"));
    // Fisherman knows a swap partition by the target `swap`.
    assert_eq!(json::text(&mounts[2], "target").as_deref(), Some("swap"));
    assert!(json::field(&done, "varDisk").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// Every rung of the key ladder is covered here, because a rung that answers
/// the wrong way leaves the user a machine that cannot unlock at boot.
#[test]
fn the_ladder_says_how_each_container_opens_at_boot() {
    let data = |key: Key| LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/var".to_string(),
        key,
    };
    let root = LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/".to_string(),
        key: Key::Data(b"opensesame".to_vec()),
    };
    assert_eq!(at_boot(&root, Opened::Keep), copy::OPENED_ROOT_KEYFILE);
    assert_eq!(at_boot(&root, Opened::Tpm2), copy::BOOT_TPM2);
    let root_passphrase = LuksOpen {
        partition: "/dev/vda3".to_string(),
        target: "/".to_string(),
        key: Key::Passphrase("opensesame".to_string()),
    };
    assert_eq!(
        at_boot(&root_passphrase, Opened::Keep),
        copy::BOOT_PASSPHRASE
    );
    assert_eq!(
        at_boot(&data(Key::Data(b"key".to_vec())), Opened::Keep),
        copy::BOOT_KEYFILE
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::Keep),
        copy::BOOT_PASSPHRASE
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::AddKey),
        copy::BOOT_ADDED_KEY
    );
    assert_eq!(
        at_boot(&data(Key::Passphrase("x".into())), Opened::Tpm2),
        copy::BOOT_TPM2
    );
}

/// The layout editor opened a second time reads the encryption row back, so
/// the user's answer survives the reopen.
#[test]
fn the_opened_rows_answer_reads_back() {
    assert_eq!(Opened::of(copy::OPENED_TPM2), Opened::Tpm2);
    assert_eq!(Opened::of(copy::OPENED_ADD_KEY), Opened::AddKey);
    assert_eq!(Opened::of(copy::OPENED_KEEP), Opened::Keep);
    assert_eq!(Opened::of("anything else"), Opened::Keep);
}

/// A headless run names its disk by flag and draws no picture, so a disk
/// whose partitions cannot be read is refused before the writer sees it.
#[test]
fn a_headless_run_refuses_a_disk_it_cannot_read() {
    let given = Given {
        disk: Some("/nonexistent/tect-none".to_string()),
        hostname: Some("deb2".to_string()),
        user: Some("tect".to_string()),
        password: Some("hunter2".to_string()),
        ..Default::default()
    };
    match Answers::seeded(&a_payload(), given, &Prompt::silent()) {
        Err(said) => assert!(said.contains("lsblk"), "{said}"),
        Ok(_) => panic!("a disk lsblk cannot read is refused"),
    }
}

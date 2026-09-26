use super::*;

/// Cuts a whole-disk plan into a sparse image and returns the `sfdisk --dump`
/// partition lines. The backing file carries an old `dos` table, so each case
/// also proves the cut replaces the old table. The helper returns `None` on a
/// host with no `sfdisk`.
fn cut_whole_disk(
    name: &str,
    payload: &Payload,
    kind: &str,
) -> Option<(CustomLayout, Vec<String>)> {
    let sfdisk = Command::new("sfdisk").arg("--version").output().ok()?;
    if !sfdisk.status.success() {
        return None;
    }
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(32 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let mut child = Command::new("sfdisk")
        .args(["-q", &disk])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"label: dos\nsize=20M\nsize=20M\n")
        .expect("the starting table");
    assert!(child.wait().expect("sfdisk").success());
    let encryption = Encryption {
        kind: kind.to_string(),
        passphrase: String::new(),
        pin: String::new(),
    };
    let layout = whole_disk(&disk, payload, &encryption).expect("a whole-disk plan");
    super::apply_cuts(&layout).expect("the cut");
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let lines = String::from_utf8_lossy(&dump.stdout)
        .lines()
        .filter(|line| line.starts_with(&disk))
        .map(|line| line.to_uppercase())
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    Some((layout, lines))
}

fn a_payload_for(bootloader: &str, composefs: bool) -> Payload {
    let mut payload = a_payload();
    payload.bootloader = bootloader.to_string();
    payload.composefs = composefs;
    payload.filesystem = "btrfs".to_string();
    payload
}

const ESP_TYPE: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const LINUX_TYPE: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

/// A GRUB target reads its kernel from an ext4 `/boot`, so the plan cuts one
/// between the ESP and the root. GRUB finds the root by `root=`, so the root
/// keeps the plain Linux type.
#[test]
fn a_grub_disk_is_cut_into_an_esp_a_boot_and_a_root() {
    for composefs in [false, true] {
        let payload = a_payload_for("grub2", composefs);
        let Some((layout, lines)) = cut_whole_disk("whole-grub", &payload, NONE) else {
            return;
        };
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains(ESP_TYPE) && lines[0].contains("EFI-SYSTEM"));
        assert!(lines[1].contains(LINUX_TYPE) && lines[1].contains("\"BOOT\""));
        assert!(lines[2].contains(LINUX_TYPE) && lines[2].contains("\"ROOT\""));
        let targets: Vec<(&str, &str)> = layout
            .creates
            .iter()
            .map(|create| (create.target.as_str(), create.fstype.as_str()))
            .collect();
        assert_eq!(
            targets,
            [("/boot/efi", "fat32"), ("/boot", "ext4"), ("/", "btrfs")]
        );
    }
}

/// A systemd-boot target reads its kernel from the ESP, so no `/boot` is cut.
/// On composefs the cut types the root for discovery, because no boot entry
/// carries a `root=` argument there. On ostree it keeps the Linux type.
#[test]
fn a_systemd_boot_disk_is_cut_into_an_esp_and_a_root() {
    for (composefs, root_type) in [(false, LINUX_TYPE), (true, ROOT_GUID)] {
        let payload = a_payload_for("systemd", composefs);
        let Some((_, lines)) = cut_whole_disk("whole-sdboot", &payload, NONE) else {
            return;
        };
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains(ESP_TYPE));
        assert!(
            lines[1].contains(&root_type.to_uppercase()),
            "composefs {composefs}: {}",
            lines[1]
        );
    }
}

/// The root receives the space after the ESP and `/boot`, less the part of a
/// GB the whole-GB placement leaves at the end.
#[test]
fn the_whole_disk_root_takes_the_rest_of_the_disk() {
    let payload = a_payload_for("grub2", false);
    let Some((layout, _)) = cut_whole_disk("whole-rest", &payload, NONE) else {
        return;
    };
    let root = layout.creates.last().expect("a root");
    // 32 GiB is 34.36 GB, and the ESP and `/boot` take 4 GB of it.
    assert_eq!(root.gb, 30);
}

/// An encrypted answer marks the root alone as a container, so the ESP and
/// `/boot` stay readable by the firmware and GRUB.
#[test]
fn an_encrypted_answer_seals_the_root_alone() {
    for kind in ["luks-passphrase", "tpm2-luks", "tpm2-luks-pin"] {
        let payload = a_payload_for("grub2", false);
        let Some((layout, _)) = cut_whole_disk("whole-sealed", &payload, kind) else {
            return;
        };
        let sealed: Vec<&str> = layout
            .creates
            .iter()
            .filter(|create| create.encrypt)
            .map(|create| create.target.as_str())
            .collect();
        assert_eq!(sealed, ["/"], "{kind}");
    }
    let payload = a_payload_for("grub2", false);
    let Some((layout, _)) = cut_whole_disk("whole-plain", &payload, NONE) else {
        return;
    };
    assert!(layout.creates.iter().all(|create| !create.encrypt));
}

/// Each refusal comes before the cut, so the disk keeps the table it had.
/// An absent bootloader must not read as GRUB, and an absent filesystem must
/// not reach `mkfs`.
#[test]
fn a_plan_the_payload_cannot_answer_refuses_before_the_cut() {
    let root = scratch("whole-refused");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(32 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let none = Encryption {
        kind: NONE.to_string(),
        passphrase: String::new(),
        pin: String::new(),
    };
    let mut unbooted = a_payload_for("grub2", false);
    unbooted.bootloader = String::new();
    assert!(whole_disk(&disk, &unbooted, &none).is_err());
    let mut unformatted = a_payload_for("grub2", false);
    unformatted.filesystem = String::new();
    assert!(whole_disk(&disk, &unformatted, &none).is_err());
    // The fixture's reserve is 10 GB, and a 12 GiB disk leaves a GRUB root of
    // 8 GB after the ESP and `/boot`.
    std::fs::File::options()
        .write(true)
        .open(&image)
        .and_then(|file| file.set_len(12 * 1024 * 1024 * 1024))
        .expect("a smaller disk");
    let small = whole_disk(&disk, &a_payload_for("grub2", false), &none)
        .expect_err("a root below the reserve");
    assert!(small.contains("below"), "{small}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The composefs backend enables fs-verity on the objects it stores on the
/// root, and an ext4 made without the feature refuses it. GRUB reads `/boot`
/// and refuses an ext4 feature it does not know.
#[test]
fn only_an_ext4_root_is_made_with_verity() {
    assert_eq!(
        mkfs_args("ext4", "/").expect("ext4"),
        ["mkfs.ext4", "-F", "-O", "verity"]
    );
    assert_eq!(
        mkfs_args("ext4", "/boot").expect("ext4"),
        ["mkfs.ext4", "-F"]
    );
    assert_eq!(
        mkfs_args("fat32", "/boot/efi").expect("fat32"),
        ["mkfs.fat", "-F", "32"]
    );
}

/// The owner decided the root carries no zfs. A filesystem this installer
/// cannot write is refused by name, and an empty one is refused too.
#[test]
fn a_filesystem_the_installer_cannot_write_is_refused() {
    for fstype in ["zfs", "", "luks", "ext3"] {
        assert!(mkfs_args(fstype, "/").is_err(), "{fstype:?}");
    }
}

/// A created container is formatted and mounted through its mapper. Its raw
/// partition holds a LUKS header, so writing a filesystem there would destroy
/// the container.
#[test]
fn a_sealed_root_is_formatted_through_its_mapper() {
    let layout = CustomLayout {
        disk: "/dev/vda".to_string(),
        creates: vec![
            Created {
                gb: 2,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: "/dev/vda1".to_string(),
                ..Default::default()
            },
            Created {
                gb: 30,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: "/dev/vda2".to_string(),
                encrypt: true,
                ..Default::default()
            },
        ],
        opens: vec![LuksOpen {
            partition: "/dev/vda2".to_string(),
            target: "/".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    // The create names its own filesystem, which wins over the image's.
    assert_eq!(
        volumes(&layout, "xfs"),
        [
            Volume {
                device: "/dev/vda1".to_string(),
                target: "/boot/efi".to_string(),
                format: Some("fat32".to_string()),
            },
            Volume {
                device: "/dev/mapper/tect-1".to_string(),
                target: "/".to_string(),
                format: Some("btrfs".to_string()),
            },
        ]
    );
}

/// bootc installs only onto an empty root, so an opened old container at `/`
/// takes the image's filesystem inside it. A kept ESP keeps its own.
#[test]
fn an_opened_root_is_formatted_inside_and_a_kept_partition_is_not() {
    let layout = CustomLayout {
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
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert_eq!(
        volumes(&layout, "ext4")
            .iter()
            .map(|volume| (volume.target.as_str(), volume.format.as_deref()))
            .collect::<Vec<_>>(),
        [("/boot/efi", None), ("/", Some("ext4"))]
    );
}

/// A child mount point mounted before its parent would be hidden under the
/// parent mount, and bootc would write the ESP into the root filesystem.
#[test]
fn a_mount_point_mounts_after_the_one_it_sits_under() {
    let volume = |target: &str| Volume {
        device: format!("/dev/{}", target.len()),
        target: target.to_string(),
        format: None,
    };
    let volumes = [
        volume("/boot/efi"),
        volume("/swap"),
        volume("/"),
        volume("/boot"),
    ];
    let order: Vec<&str> = mount_order(&volumes)
        .into_iter()
        .map(|volume| volume.target.as_str())
        .collect();
    assert_eq!(order, ["/", "/boot", "/boot/efi"]);
}

/// An EL live environment carries no `mkfs.btrfs`, so the lookup has to tell
/// a missing program from a present one before the cut.
#[test]
fn a_program_missing_from_path_is_not_found() {
    assert!(on_path("sh"));
    assert!(!on_path("tect-installer-no-such-mkfs"));
}

/// A `tect-` container on another disk can be the running system's root, so
/// `release_stale` closes only the target disk's containers.
#[test]
fn only_the_target_disks_tect_mappers_are_stale() {
    let root = scratch("stale-mappers");
    let blocks = root.join("block");
    let devices = root.join("devices");
    let mapper = |dm: &str, name: &str, disk: &str, part: &str| {
        let at = blocks.join(dm);
        std::fs::create_dir_all(at.join("dm")).expect("a dm directory");
        std::fs::create_dir_all(at.join("slaves")).expect("a slaves directory");
        std::fs::write(at.join("dm/name"), format!("{name}\n")).expect("a dm name");
        let partition = devices.join(disk).join(part);
        std::fs::create_dir_all(&partition).expect("a partition directory");
        std::os::unix::fs::symlink(&partition, at.join("slaves").join(part)).expect("a slave link");
    };
    mapper("dm-0", "tect-1", "vda", "vda3");
    mapper("dm-1", "tect-1x", "vdb", "vdb2");
    mapper("dm-2", "luks-old", "vda", "vda4");
    std::fs::create_dir_all(blocks.join("vda")).expect("a plain disk entry");
    assert_eq!(
        stale_mappers(&blocks, "/dev/vda").expect("a sysfs listing"),
        ["tect-1"]
    );
    let _ = std::fs::remove_dir_all(root);
}

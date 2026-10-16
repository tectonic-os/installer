use super::*;

#[test]
fn lsblk_partitions_keep_the_details_the_editor_shows() {
    let rows = partition_rows(
            r#"{
  "blockdevices": [{
    "name": "/dev/vda", "size": "64G", "fstype": null, "label": null, "type": "disk",
    "children": [
      {"name": "/dev/vda1", "size": "512M", "fstype": "vfat", "label": "EFI", "type": "part", "parttype": "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"},
      {"name": "/dev/vda2", "size": "63.5G", "fstype": "crypto_LUKS", "label": null, "type": "part", "parttype": null, "uuid": "a1b2c3d4-0000-0000-0000-000000000000"}
    ]
  }]
}"#,
        )
        .expect("lsblk JSON");
    assert_eq!(
        rows,
        [
            Partition {
                device: "/dev/vda1".to_string(),
                size: "512M".to_string(),
                fstype: "vfat".to_string(),
                label: "EFI".to_string(),
                parttype: "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".to_string(),
                uuid: String::new(),
            },
            Partition {
                device: "/dev/vda2".to_string(),
                size: "63.5G".to_string(),
                fstype: "crypto_LUKS".to_string(),
                label: String::new(),
                parttype: String::new(),
                uuid: "a1b2c3d4-0000-0000-0000-000000000000".to_string(),
            },
        ]
    );
}

/// A key an old system holds is reused without asking. The key file row is
/// offered only where the layout opens the root as a container, which is the
/// same rule `usable_key` applies to the found key.
#[test]
fn a_found_key_opens_the_container_without_asking() {
    let found = Discovered {
        found: vec![(
            "/dev/vda2".to_string(),
            OldKey {
                target: "/run/key".to_string(),
                key: Key::Passphrase("opensesame".to_string()),
            },
        )],
        why: Vec::new(),
    };
    let key = open_key("/dev/vda2", None, &found).expect("the found key");
    assert!(matches!(key, Key::Passphrase(said) if said == "opensesame"));
    assert!(key_methods(false)
        .iter()
        .any(|choice| choice.label == copy::KEY_FILE));
    assert!(!key_methods(true)
        .iter()
        .any(|choice| choice.label == copy::KEY_FILE));
}

/// Every shape a crypttab's third field takes is covered here. A line of two
/// fields asks for a passphrase at boot, so it names no key and takes the
/// systemd default.
#[test]
fn an_old_crypttab_says_where_each_key_is() {
    let entries = crypttabs(
        "\
# a comment
root UUID=aa11 none luks
var UUID=bb22 /etc/luks/var.key luks,discard
data UUID=cc33 /key:UUID=dd44 luks
stick UUID=ee55 UUID=ff66:/key:10 luks,keyscript=/lib/cryptsetup/scripts/passdev
other UUID=gg77 none luks,keyscript=/lib/cryptsetup/scripts/decrypt_derived
passphrase UUID=hh88
timed UUID=ii99 /media/stick/var.key luks,keyfile-timeout=30s
",
    );
    assert_eq!(entries.len(), 7);
    assert_eq!(entries[0].key, KeySource::Default);
    assert_eq!(
        entries[1].key,
        KeySource::Inside(PathBuf::from("/etc/luks/var.key"))
    );
    assert_eq!(
        entries[2].key,
        KeySource::OnDevice {
            device: "UUID=dd44".to_string(),
            path: PathBuf::from("/key"),
        }
    );
    assert_eq!(
        entries[3].key,
        KeySource::OnDevice {
            device: "UUID=ff66".to_string(),
            path: PathBuf::from("/key"),
        }
    );
    assert_eq!(entries[4].key, KeySource::Unreadable);
    assert_eq!(entries[5].key, KeySource::Default);
    // A waited key names removable media, so the `timed` check runs before
    // the fall through to `Inside`.
    assert_eq!(entries[6].key, KeySource::Waited);
}

/// A crypttab names its container by the LUKS uuid, by the by-uuid symlink
/// or by the device path. A spelling matching none of the three names
/// another disk.
#[test]
fn an_old_system_names_a_container_by_its_uuid_or_its_device() {
    let container = Partition {
        device: "/dev/vda3".to_string(),
        size: String::new(),
        fstype: "crypto_LUKS".to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: "A1B2".to_string(),
    };
    let entry = |device: &str| Crypttab {
        name: "var".to_string(),
        device: device.to_string(),
        key: KeySource::Default,
    };
    assert!(names(&entry("UUID=a1b2"), &container));
    assert!(names(&entry("/dev/disk/by-uuid/A1B2"), &container));
    assert!(names(&entry("/dev/vda3"), &container));
    assert!(!names(&entry("UUID=ffff"), &container));
    assert!(!names(&entry("/dev/vda2"), &container));
}

/// The walk proves each key against the container before it hands the key
/// on. A key that does not open the container becomes a reason in `why`.
#[test]
fn a_key_the_old_system_names_is_read_and_proved_against_the_container() {
    let root = scratch("old-system");
    let etc = root.join("etc");
    std::fs::create_dir_all(etc.join("cryptsetup-keys.d")).expect("a scratch /etc");
    std::fs::write(etc.join("crypttab"), "var UUID=aa11 none luks\n").expect("a crypttab");
    std::fs::write(
        etc.join("fstab"),
        "/dev/mapper/var /var ext4 defaults 0 0\n",
    )
    .expect("an fstab");
    std::fs::write(etc.join("cryptsetup-keys.d/var.key"), b"opensesame").expect("a key");
    let container = Partition {
        device: "/dev/vda3".to_string(),
        size: "60G".to_string(),
        fstype: "crypto_LUKS".to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: "AA11".to_string(),
    };
    let system = OldSystem {
        crypttab: std::fs::read_to_string(etc.join("crypttab")).unwrap(),
        fstab: std::fs::read_to_string(etc.join("fstab")).unwrap(),
        at: root.clone(),
    };
    let mut mounts = Mounts::default();
    let found = discover(
        std::slice::from_ref(&container),
        std::slice::from_ref(&system),
        &[],
        &mut mounts,
        &|_, key| Ok(key == b"opensesame"),
    );
    assert!(found.why.is_empty(), "{:?}", found.why);
    let (partition, old) = &found.found[0];
    assert_eq!(partition, "/dev/vda3");
    assert_eq!(old.target, "/var");
    assert!(matches!(&old.key, Key::Data(bytes) if bytes == b"opensesame"));

    let refused = discover(
        std::slice::from_ref(&container),
        std::slice::from_ref(&system),
        &[],
        &mut mounts,
        &|_, _| Ok(false),
    );
    assert!(refused.found.is_empty());
    assert_eq!(refused.why[0].1, copy::old_root_key_wrong("/dev/vda3"));

    // A read system that names no key for this container has its own reason.
    let unnamed = OldSystem {
        crypttab: "root UUID=ffff none luks\n".to_string(),
        fstab: String::new(),
        at: root.clone(),
    };
    let missed = discover(
        std::slice::from_ref(&container),
        std::slice::from_ref(&unnamed),
        &[],
        &mut mounts,
        &|_, _| Ok(true),
    );
    assert_eq!(missed.why[0].1, copy::old_root_unnamed("/dev/vda3"));
    // An unread device outranks a read system that named no key.
    let unread = ["mounting /dev/vda2 read-only: permission denied".to_string()];
    let partly = discover(
        std::slice::from_ref(&container),
        std::slice::from_ref(&unnamed),
        &unread,
        &mut mounts,
        &|_, _| Ok(true),
    );
    assert_eq!(partly.why[0].1, copy::old_root_partly(&unread[0]));
    let _ = std::fs::remove_dir_all(&root);
}

/// The old system's mount point does not exist in this case, which proves
/// the refusal lands before the walk reads anything.
#[test]
fn a_key_file_waited_for_at_boot_is_refused_rather_than_read() {
    let container = Partition {
        device: "/dev/vda3".to_string(),
        size: String::new(),
        fstype: "crypto_LUKS".to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: "AA11".to_string(),
    };
    let system = OldSystem {
        crypttab: "var UUID=aa11 /media/stick/var.key luks,keyfile-timeout=30s\n".to_string(),
        fstab: String::new(),
        at: PathBuf::from("/nonexistent-old-system"),
    };
    let mut mounts = Mounts::default();
    let found = discover(
        std::slice::from_ref(&container),
        std::slice::from_ref(&system),
        &[],
        &mut mounts,
        &|_, _| Ok(true),
    );
    assert!(found.found.is_empty());
    assert_eq!(found.why[0].1, copy::old_root_key_waited("var"));
}

/// A wrong answer here carries a key off media the user keeps apart from
/// the machine, so this case builds a `/sys/class/block` of its own.
#[test]
fn a_key_mediums_devices_are_told_apart_by_the_kernels_own_flag() {
    let root = scratch("removable");
    let class = root.join("class");
    let block = root.join("block");
    std::fs::create_dir_all(&class).expect("a class directory");
    for (disk, flag) in [("vdb", "1"), ("nvme0n1", "0")] {
        std::fs::create_dir_all(block.join(disk)).expect("a disk");
        std::fs::write(block.join(disk).join("removable"), flag).expect("a flag");
        std::os::unix::fs::symlink(block.join(disk), class.join(disk)).expect("a class entry");
    }
    std::fs::create_dir_all(block.join("vdb/vdb1")).expect("a partition");
    std::fs::write(block.join("vdb/vdb1/partition"), "1").expect("a partition number");
    std::os::unix::fs::symlink(block.join("vdb/vdb1"), class.join("vdb1"))
        .expect("a partition class entry");

    // A crypttab names a key's device by a udev path, so the lookup
    // canonicalizes to the kernel's node before it reads the flag.
    let by_uuid = root.join("by-uuid");
    std::fs::create_dir_all(&by_uuid).expect("a by-uuid directory");
    std::os::unix::fs::symlink(block.join("vdb/vdb1"), by_uuid.join("AAAA")).expect("a udev name");

    assert!(removable_at(&class, Path::new("/dev/vdb")));
    assert!(removable_at(&class, Path::new("/dev/vdb1")));
    assert!(removable_at(&class, &by_uuid.join("AAAA")));
    assert!(!removable_at(&class, Path::new("/dev/nvme0n1")));
    // An unknown device counts as fixed, so the walk reads the key it holds
    // instead of refusing it.
    assert!(!removable_at(&class, Path::new("/dev/mapper/vg-data")));
    assert!(!removable_at(&class, Path::new("/dev")));
    let _ = std::fs::remove_dir_all(&root);
}

/// A key path is read only under the mount that names it. The symlink out,
/// the `..` out and the FIFO all come back as the one `old_root_key_outside`
/// refusal.
#[test]
fn a_key_path_cannot_leave_the_system_that_names_it() {
    let root = scratch("key-confine");
    std::fs::create_dir_all(root.join("etc")).expect("a scratch /etc");
    std::fs::write(root.join("etc/real.key"), b"opensesame").expect("a key");
    std::os::unix::fs::symlink("/etc/hostname", root.join("etc/escape.key")).expect("a link");
    let fifo = root.join("etc/slow.key");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(key_file("var", &root, Path::new("/etc/real.key")).is_ok());
    assert_eq!(
        key_file("var", &root, Path::new("/etc/escape.key")).unwrap_err(),
        copy::old_root_key_outside("/etc/escape.key")
    );
    assert_eq!(
        key_file("var", &root, Path::new("/../../../etc/hostname")).unwrap_err(),
        copy::old_root_key_outside("/../../../etc/hostname")
    );
    assert_eq!(
        key_file("var", &root, Path::new("/etc/slow.key")).unwrap_err(),
        copy::old_root_key_outside("/etc/slow.key")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The systems a partition carries are read from what it holds: an ESP's
/// vendor directories, a filesystem's own `os-release`, or a loader entry's
/// title. `EFI/BOOT` alone names nothing, and so does a filesystem holding
/// none of the three.
#[test]
fn a_partition_names_the_systems_it_carries() {
    let names = |carried: &[Carried]| -> Vec<String> {
        carried.iter().map(|one| one.name.clone()).collect()
    };
    let root = scratch("labels");
    let esp = root.join("esp");
    std::fs::create_dir_all(esp.join("EFI/fedora")).unwrap();
    std::fs::create_dir_all(esp.join("EFI/Microsoft/Boot")).unwrap();
    std::fs::create_dir_all(esp.join("EFI/BOOT")).unwrap();
    // The directory names sort as they read, so `Microsoft` leads.
    assert_eq!(names(&labels_at(&esp, "vfat")), ["Microsoft", "fedora"]);
    let bare = root.join("bare");
    std::fs::create_dir_all(bare.join("EFI/BOOT")).unwrap();
    assert!(labels_at(&bare, "vfat").is_empty());

    let system = root.join("system");
    std::fs::create_dir_all(system.join("etc")).unwrap();
    std::fs::write(
        system.join("etc/os-release"),
        "NAME=Fedora\nPRETTY_NAME=\"Fedora Linux 44\"\n",
    )
    .unwrap();
    assert_eq!(names(&labels_at(&system, "ext4")), ["Fedora Linux 44"]);
    // A filesystem with no `os-release` falls back to a loader entry's title.
    // The entries sit under `boot/loader/` on a root and `loader/` on a
    // separate `/boot`.
    std::fs::remove_file(system.join("etc/os-release")).unwrap();
    std::fs::create_dir_all(system.join("boot/loader/entries")).unwrap();
    std::fs::write(
        system.join("boot/loader/entries/fedora.conf"),
        "title Fedora Linux 44 (Workstation)\n",
    )
    .unwrap();
    assert_eq!(
        names(&labels_at(&system, "ext4")),
        ["Fedora Linux 44 (Workstation)"]
    );
    let boot = root.join("boot");
    std::fs::create_dir_all(boot.join("loader/entries")).unwrap();
    std::fs::write(
        boot.join("loader/entries/fedora.conf"),
        "title Fedora Linux 44 (Server)\n",
    )
    .unwrap();
    assert_eq!(
        names(&labels_at(&boot, "ext4")),
        ["Fedora Linux 44 (Server)"]
    );
    assert!(labels_at(&root.join("nothing"), "xfs").is_empty());
    // A FIFO where the walk reads a name blocks the installer, so only a
    // regular file is read.
    let hostile = root.join("hostile");
    std::fs::create_dir_all(hostile.join("etc")).unwrap();
    let fifo = hostile.join("etc/os-release");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(labels_at(&hostile, "ext4").is_empty());
    // A symlink out of the mount names another filesystem's file, not this
    // one's.
    let linked = root.join("linked");
    std::fs::create_dir_all(linked.join("etc")).unwrap();
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join("os-release"),
        "PRETTY_NAME=\"Not This Disk\"\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(elsewhere.join("os-release"), linked.join("etc/os-release"))
        .unwrap();
    assert!(labels_at(&linked, "ext4").is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// The walk reads what each ESP entry boots from the loader's own files:
/// bootupd's `bootuuid.cfg`, the static `grub.cfg`'s search, or the ESP's own
/// loader entries. A loader entry is read only where the vendor directory
/// names nothing itself.
#[test]
fn an_esp_entry_names_the_filesystem_its_loader_boots() {
    let root = scratch("esp-links");
    let efi = root.join("EFI");
    std::fs::create_dir_all(efi.join("fedora")).unwrap();
    std::fs::write(
        efi.join("fedora/bootuuid.cfg"),
        "set BOOT_UUID=\"8e0615fb-7297-4b74-97c5-b9b84c0f36ba\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(efi.join("systemd")).unwrap();
    std::fs::create_dir_all(root.join("loader/entries")).unwrap();
    std::fs::write(
        root.join("loader/entries/ostree-1.conf"),
        "title Test OS\noptions root=UUID=6c38207b-7766-446b-a210-b438ae22aa2b \
         rd.luks.uuid=luks-37be8ca7-8834-4aca-aee0-dd280af10939 rw\n",
    )
    .unwrap();
    let carried = labels_at(&root, "vfat");
    let names: Vec<&str> = carried.iter().map(|one| one.name.as_str()).collect();
    assert_eq!(names, ["fedora", "systemd"]);
    assert_eq!(
        carried[0].links,
        [Link::Uuid(
            "8e0615fb-7297-4b74-97c5-b9b84c0f36ba".to_string()
        )]
    );
    // The loader entry names the root filesystem first and the container the
    // root lives in second, so a closed container falls back to it.
    assert_eq!(
        carried[1].links,
        [
            Link::Uuid("6c38207b-7766-446b-a210-b438ae22aa2b".to_string()),
            Link::Uuid("37be8ca7-8834-4aca-aee0-dd280af10939".to_string()),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A Windows install is named by the GPT types it carries, because it writes
/// no `os-release` and its ESP may sit on another disk. The reserved
/// partition tells an install from a data disk that merely uses NTFS, and the
/// basic data volume is the partition the user recognises.
#[test]
fn a_windows_install_is_named_by_the_types_it_carries() {
    let part = |device: &str, parttype: &str| Partition {
        device: device.to_string(),
        parttype: parttype.to_string(),
        ..Default::default()
    };
    let msr = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
    let data = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
    let recovery = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";
    let parts = [
        part("/dev/sda1", msr),
        part("/dev/sda2", data),
        part("/dev/sda3", recovery),
    ];
    assert_eq!(
        windows_label(&parts),
        Some(("/dev/sda2".to_string(), "Windows".to_string()))
    );
    // A reserved and recovery pair with no data volume left still names the
    // install, on the reserved partition.
    assert_eq!(
        windows_label(&[part("/dev/sda1", msr), part("/dev/sda3", recovery)]),
        Some(("/dev/sda1".to_string(), "Windows".to_string()))
    );
    // A data disk that merely uses NTFS, and a stray reserved partition, name
    // nothing.
    assert_eq!(windows_label(&[part("/dev/sda2", data)]), None);
    assert_eq!(windows_label(&[part("/dev/sda1", msr)]), None);
    assert_eq!(windows_label(&[]), None);
}

/// A Microsoft entry names a Windows install only when the scan can tell which
/// one, because a wrong link removes a boot entry for a system the plan keeps.
#[test]
fn a_microsoft_entry_links_only_to_the_install_it_can_be_sure_of() {
    let one = vec!["/dev/sda2".to_string()];
    let two = vec!["/dev/sda2".to_string(), "/dev/sdb2".to_string()];
    assert_eq!(
        windows_link(Some("/dev/sda2"), &two).as_deref(),
        Some("/dev/sda2")
    );
    assert_eq!(windows_link(None, &one).as_deref(), Some("/dev/sda2"));
    assert_eq!(windows_link(None, &two), None);
    assert_eq!(windows_link(None, &[]), None);
}

/// A lone disk answers the disk question only when it holds nothing, because
/// a disk with partitions is what `use this disk` exists to confirm.
#[test]
fn a_lone_disk_answers_only_when_it_is_empty() {
    let empty = scan_of(&[("/dev/vda", "64G")], &[]);
    assert_eq!(empty.only_empty_disk().as_deref(), Some("/dev/vda"));
    let held = scan_of(
        &[("/dev/vda", "64G")],
        &[("/dev/vda", vec![Partition::default()])],
    );
    assert_eq!(held.only_empty_disk(), None);
    let two = scan_of(&[("/dev/sda", "16 GB"), ("/dev/vda", "64G")], &[]);
    assert_eq!(two.only_empty_disk(), None);
    // A whole-disk filesystem and a whole-disk LVM physical volume leave no
    // partition children, and the disk still holds a system.
    let content = |raw: &str| carries_content(raw).expect("lsblk JSON");
    assert!(content(
        r#"{"blockdevices":[{"name":"/dev/vda","type":"disk","fstype":"btrfs"}]}"#
    ));
    assert!(content(
        r#"{"blockdevices":[{"name":"/dev/vda","type":"disk","fstype":null,"children":[{"name":"/dev/mapper/vg-root","type":"lvm"}]}]}"#
    ));
    assert!(!content(
        r#"{"blockdevices":[{"name":"/dev/vda","type":"disk","fstype":null,"children":[]}]}"#
    ));
    let whole = Scan {
        unread: Vec::new(),
        disks: vec![DiskScan {
            device: "/dev/vda".to_string(),
            detail: "64G".to_string(),
            partitions: Vec::new(),
            carries: true,
            table: Ok(DiskTable::default()),
            labels: Vec::new(),
            keys: Discovered::default(),
        }],
    };
    assert_eq!(whole.only_empty_disk(), None);
}

/// A disk the scan could not read is not offered, and it keeps the lone-disk
/// answer open, because the machine still holds the disk the form cannot see.
#[test]
fn a_disk_the_scan_could_not_read_is_not_offered() {
    let why = "lsblk /dev/sdb: I/O error";
    let mut survivor = scan_of(&[("/dev/vda", "64G")], &[]);
    survivor
        .unread
        .push(("/dev/sdb".to_string(), why.to_string()));
    assert_eq!(survivor.refusal("/dev/sdb").as_deref(), Some(why));
    assert_eq!(survivor.refusal("/dev/vda"), None);
    assert_eq!(survivor.only_empty_disk(), None);
    // A machine with no readable disk left stops on the first reason.
    let none = Scan {
        unread: vec![("/dev/sdb".to_string(), why.to_string())],
        ..Default::default()
    };
    assert_eq!(none.refusal("").as_deref(), Some(why));
    // A udev alias names the disk the scan read under its kernel name.
    let root = scratch("unread-alias");
    let real = root.join("sdb");
    std::fs::write(&real, b"").unwrap();
    let alias = root.join("sdb-by-id");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let aliased = Scan {
        unread: vec![(real.to_string_lossy().to_string(), why.to_string())],
        ..Default::default()
    };
    assert_eq!(
        aliased.refusal(&alias.to_string_lossy()).as_deref(),
        Some(why)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A `crypto_LUKS` node carries no filesystem of its own, so the walk takes
/// the mapper above it instead.
#[test]
fn the_walk_reads_every_mountable_node_and_no_container() {
    let rows = mountable_rows(
        r#"{
  "blockdevices": [
    {"name": "/dev/vda1", "fstype": "vfat", "type": "part"},
    {"name": "/dev/vda2", "fstype": "crypto_LUKS", "type": "part", "children": [
      {"name": "/dev/mapper/vda2", "fstype": "ext4", "type": "crypt", "children": [
        {"name": "/dev/mapper/vg-root", "fstype": "xfs", "type": "lvm"}
      ]}
    ]},
    {"name": "/dev/vda3", "fstype": null, "type": "part"}
  ]
}"#,
    )
    .expect("lsblk JSON");
    assert_eq!(
        rows,
        [
            ("/dev/vda1".to_string(), "vfat".to_string()),
            ("/dev/mapper/vda2".to_string(), "ext4".to_string()),
            ("/dev/mapper/vg-root".to_string(), "xfs".to_string()),
        ]
    );
    // The four journaling filesystems take `norecovery`, because a read-only
    // mount of one still writes a replayed journal.
    for fstype in ["ext3", "ext4", "xfs", "btrfs"] {
        assert_eq!(mount_options(fstype), "ro,norecovery", "{fstype}");
    }
    for fstype in ["vfat", "exfat", "iso9660", ""] {
        assert_eq!(mount_options(fstype), "ro", "{fstype}");
    }
}

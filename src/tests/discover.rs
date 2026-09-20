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

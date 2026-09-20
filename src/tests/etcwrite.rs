use super::*;

/// This install runs onto a disk whose crypttab already names `var` and
/// `home`, so the merge has to rewrite one line and leave the other.
#[test]
fn the_crypttab_keeps_other_entries_and_replaces_its_own() {
    let root = scratch("crypttab");
    let etc = root.join("etc");
    std::fs::create_dir_all(&etc).expect("an etc");
    std::fs::write(
        etc.join("crypttab"),
        "var UUID=old none luks\nhome UUID=bb none luks\n",
    )
    .expect("a crypttab");
    let lines = vec![(
        "var".to_string(),
        "var UUID=new /etc/cryptsetup-keys.d/var.key luks".to_string(),
    )];
    merge_crypttab(&etc, &lines).expect("a merged crypttab");
    let said = std::fs::read_to_string(etc.join("crypttab")).expect("a crypttab");
    assert!(said.contains("home UUID=bb"), "{said}");
    assert!(said.contains("UUID=new"), "{said}");
    assert!(!said.contains("UUID=old"), "{said}");
    // A layout with no data volume merges an empty list. The installed
    // `/etc` then gains no crypttab, which ostree would merge on upgrade.
    let other = scratch("crypttab-empty");
    let empty = other.join("etc");
    std::fs::create_dir_all(&empty).expect("an etc");
    merge_crypttab(&empty, &[]).expect("nothing to merge");
    assert!(!empty.join("crypttab").exists());
}

/// Writing the crypttab into the wrong deployment leaves a machine that
/// does not unlock. A root carrying two deployments is refused instead.
#[test]
fn the_deployment_etc_is_the_single_one_installed() {
    let root = scratch("deployments");
    let ostree = root.join("ostree/deploy/default/deploy/deadbeef.0/etc");
    std::fs::create_dir_all(&ostree).expect("an ostree deployment");
    assert_eq!(deployment_etc(&root).expect("one"), ostree);
    let composefs = root.join("state/deploy/deadbeef/etc");
    std::fs::create_dir_all(&composefs).expect("a composefs deployment");
    assert!(deployment_etc(&root).is_err());
    std::fs::remove_dir_all(&ostree).expect("one deployment");
    assert_eq!(deployment_etc(&root).expect("one"), composefs);
}

#[test]
fn the_root_device_is_where_the_layout_put_it() {
    let opened = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: Vec::new(),
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert_eq!(root_device(&opened).as_deref(), Some("/dev/mapper/tect-1"));
    let mounted = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![CustomMount {
            partition: "/dev/vda2".to_string(),
            target: "/".to_string(),
            fstype: "ext4".to_string(),

            passphrase: String::new(),
        }],
        opens: Vec::new(),
        ..Default::default()
    };
    assert_eq!(root_device(&mounted).as_deref(), Some("/dev/vda2"));
}

/// `retag_root` hands this number to `sfdisk --part-type`, so a device with
/// no trailing digits is refused instead of retagging another partition.
#[test]
fn a_partition_number_is_the_trailing_digits() {
    assert_eq!(partition_number("/dev/vda3").expect("a number"), "3");
    assert_eq!(partition_number("/dev/nvme0n1p12").expect("a number"), "12");
    assert_eq!(partition_number("/dev/mmcblk0p2").expect("a number"), "2");
    assert!(partition_number("/dev/vda").is_err());
}

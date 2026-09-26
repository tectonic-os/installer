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
    let ostree = root.join("ostree/deploy/default/deploy/deadbeef.0");
    std::fs::create_dir_all(ostree.join("etc")).expect("an ostree deployment");
    assert_eq!(deployment(&root).expect("one"), (ostree.clone(), false));
    let composefs = root.join("state/deploy/deadbeef");
    std::fs::create_dir_all(composefs.join("etc")).expect("a composefs deployment");
    assert!(deployment(&root).is_err());
    std::fs::remove_dir_all(&ostree).expect("one deployment");
    assert_eq!(deployment(&root).expect("one"), (composefs, true));
}

#[test]
fn swap_is_named_by_the_filesystem_device() {
    let plain = CustomLayout {
        mounts: vec![CustomMount {
            partition: "/dev/vda2".to_string(),
            target: "/swap".to_string(),
            fstype: "unformatted".to_string(),
            passphrase: String::new(),
        }],
        ..Default::default()
    };
    assert_eq!(swap_device(&plain).as_deref(), Some("/dev/vda2"));
    let opened = CustomLayout {
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/swap".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    assert_eq!(swap_device(&opened).as_deref(), Some("/dev/mapper/tect-1"));
}

#[test]
fn the_swap_fstab_line_replaces_only_its_own_uuid() {
    let root = scratch("swap-fstab");
    let etc = root.join("etc");
    std::fs::create_dir_all(&etc).expect("an etc");
    std::fs::write(
        etc.join("fstab"),
        "UUID=root / ext4 defaults 0 1\nUUID=swap none swap defaults 0 0\n",
    )
    .expect("an fstab");
    merge_fstab(&etc, "swap").expect("a swap line");
    let said = std::fs::read_to_string(etc.join("fstab")).expect("an fstab");
    assert_eq!(said.matches("UUID=swap").count(), 1, "{said}");
    assert!(said.contains("UUID=root / ext4"), "{said}");
}

#[test]
fn selinux_uses_the_target_policy_file() {
    let deployment = scratch("target-policy");
    let selinux = deployment.join("etc/selinux/strict/contexts/files");
    std::fs::create_dir_all(&selinux).expect("a policy directory");
    std::fs::write(
        deployment.join("etc/selinux/config"),
        "SELINUX=enforcing\nSELINUXTYPE='strict'\n",
    )
    .expect("a config");
    let contexts = selinux.join("file_contexts");
    std::fs::write(&contexts, "").expect("file contexts");
    assert_eq!(
        target_contexts(&deployment).expect("a policy"),
        Some(contexts)
    );
    assert_eq!(
        target_contexts(&scratch("no-target-policy")).expect("no policy"),
        None
    );
}

/// The authenticating key keeps its form: a passphrase travels on stdin and
/// never reaches the process list, and a key file is passed by path.
#[test]
fn the_authenticating_key_keeps_its_form() {
    let words = |key: &Key| -> Vec<String> {
        auth_args(key)
            .iter()
            .map(|word| word.to_string_lossy().to_string())
            .collect()
    };
    assert_eq!(
        words(&Key::File(PathBuf::from("/run/key"))),
        ["--key-file", "/run/key"]
    );
    assert_eq!(
        words(&Key::Passphrase("opensesame".to_string())),
        ["--key-file", "-"]
    );
    assert_eq!(words(&Key::Data(vec![1, 2, 3])), ["--key-file", "-"]);
}

/// `retag_root` hands this number to `sfdisk --part-type`, so a device with
/// no trailing digits is refused instead of retagging another partition.
#[test]
fn a_partition_number_is_the_trailing_digits() {
    assert_eq!(partition_number("/dev/vda3").expect("a number"), 3);
    assert_eq!(partition_number("/dev/nvme0n1p12").expect("a number"), 12);
    assert_eq!(partition_number("/dev/mmcblk0p2").expect("a number"), 2);
    assert!(partition_number("/dev/vda").is_err());
    // Digits that overflow `usize` are not a number. While this answered the
    // digits themselves, the readers dropped such an entry and `apply_cuts`
    // accepted it, so a delete the editor screen had ignored reached the disk
    // and `sfdisk` refused it once the deletes before it had run.
    assert!(partition_number("/dev/sda99999999999999999999").is_err());
}

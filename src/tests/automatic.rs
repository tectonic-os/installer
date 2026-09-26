use super::*;

/// The automatic layout's root is the partition `retag_root` types for GPT
/// auto-discovery. The credential goes to the partition systemd-stub boots
/// from. A second match leaves no single one, and a listing with no match
/// leaves none at all.
#[test]
fn a_partition_type_names_its_partitions() {
    let listed = "\
vda1 c12a7328-f81f-11d2-ba4b-00a0c93ec93b
vda2 4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709
vda3 0fc63daf-8483-4772-8e79-3d69d8477de4
";
    assert_eq!(partitions_of_type(listed, ROOT_GUID), ["/dev/vda2"]);
    assert_eq!(partitions_of_type(listed, copy::ESP_TYPE), ["/dev/vda1"]);
    assert!(
        partitions_of_type("vda1 0fc63daf-8483-4772-8e79-3d69d8477de4\n", ROOT_GUID).is_empty()
    );
    let two = "\
vda2 4f68bce3-e8cd-4db1-96e7-fbcaf984b709
vda3 4f68bce3-e8cd-4db1-96e7-fbcaf984b709
";
    assert_eq!(partitions_of_type(two, ROOT_GUID).len(), 2);
}

/// The automatic action chooses the slot it adds, so it can name it even if
/// the header cannot be read back afterwards. A full header stops the action
/// before anything changes.
#[test]
fn a_new_key_takes_the_first_free_slot() {
    assert_eq!(free_slot(&[]).expect("slot 0"), 0);
    assert_eq!(free_slot(&[0, 1]).expect("slot 2"), 2);
    assert_eq!(free_slot(&[1, 0, 3]).expect("slot 2"), 2);
    let full: Vec<u32> = (0..32).collect();
    assert!(free_slot(&full).is_err());
}

fn answers(kind: &str, passphrase: &str) -> Answers {
    Answers {
        disk: "/dev/vda".to_string(),
        hostname: "host".to_string(),
        user: "user".to_string(),
        password: "secret".to_string(),
        encryption: Encryption {
            kind: kind.to_string(),
            passphrase: passphrase.to_string(),
            pin: String::new(),
        },
        layout: None,
        opened: Opened::Keep,
    }
}

/// The kinds that stage a first-boot enrolment are the ones whose completion
/// screen offers to finalize. `layout::seal` generates the recovery key of a
/// `tpm2-luks` kind. The passphrase kind carries its own key.
#[test]
fn only_a_staged_enrolment_has_a_key_to_finalize_with() {
    let no_event = unlocking_key(&answers("tpm2-luks", ""), None);
    assert!(no_event.is_none(), "a lost recovery key offers nothing");
    let (key, pin) = unlocking_key(&answers("tpm2-luks", ""), Some("recovery")).expect("a key");
    assert_eq!(key.bytes(), Some(b"recovery".as_slice()));
    assert!(!pin);
    let (_, pin) = unlocking_key(&answers("tpm2-luks-pin", ""), Some("recovery")).expect("a key");
    assert!(pin, "a PIN kind states its window");
    let (key, _) = unlocking_key(&answers("tpm2-luks-passphrase", "chosen"), None).expect("a key");
    assert_eq!(key.bytes(), Some(b"chosen".as_slice()));
    assert!(unlocking_key(&answers("luks-passphrase", "chosen"), None).is_none());
    assert!(unlocking_key(&answers("none", ""), None).is_none());
}

/// A layout's automatic action is for the root alone. A kept container and a
/// layout whose root is not opened both stage no root enrolment.
#[test]
fn a_layout_finalizes_only_an_opened_root() {
    let opened = CustomLayout {
        disk: "/dev/vda".to_string(),
        opens: vec![LuksOpen {
            partition: "/dev/vda3".to_string(),
            target: "/".to_string(),
            key: Key::Passphrase("opensesame".to_string()),
        }],
        ..Default::default()
    };
    let mut held = answers("luks-passphrase", "chosen");
    held.opened = Opened::Tpm2;
    held.layout = Some(opened.clone());
    let (key, pin) = unlocking_key(&held, None).expect("a key");
    assert_eq!(key.bytes(), Some(b"opensesame".as_slice()));
    assert!(!pin);
    assert_eq!(encrypted_root(&held).expect("the opened root"), "/dev/vda3");

    let mut data_only = opened.clone();
    data_only.opens[0].target = "/var".to_string();
    held.layout = Some(data_only);
    assert!(unlocking_key(&held, None).is_none());

    held.layout = Some(opened);
    held.opened = Opened::Keep;
    assert!(unlocking_key(&held, None).is_none());
}

/// The credential has to land on the partition `systemd-stub` booted from.
/// A partition whose filesystem carries neither the loader entries nor a UKI
/// is one no initrd asks for the credential.
#[test]
fn only_a_partition_with_boot_files_carries_the_credential() {
    let bare = scratch("automatic-bare");
    assert!(!carries_boot_files(&bare));
    let entries = scratch("automatic-entries");
    std::fs::create_dir_all(entries.join("loader/entries")).unwrap();
    assert!(carries_boot_files(&entries));
    let uki = scratch("automatic-uki");
    std::fs::create_dir_all(uki.join("EFI/Linux")).unwrap();
    assert!(carries_boot_files(&uki));
    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&entries);
    let _ = std::fs::remove_dir_all(&uki);
}

/// A failed finalize names the one-time slot it could not remove. The reason
/// is printed where the user can read it and names the slot to wipe by hand.
#[test]
fn a_failed_finalize_names_the_slot_it_left_behind() {
    let why = "sealing the one-time key failed".to_string();
    assert_eq!(discarded_message(3, why.clone(), Ok(())), why);
    let left = discarded_message(3, why.clone(), Err("device is busy".to_string()));
    assert!(left.starts_with(&why), "{left}");
    assert!(left.contains("slot 3"), "{left}");
    assert!(left.contains("device is busy"), "{left}");
}

/// The slot removal authenticates with the key that opens the container.
/// `cryptsetup` reads the device before the slot number, so the two orders are
/// not interchangeable.
#[test]
fn the_slot_removal_names_the_device_then_the_slot() {
    let ready = Ready {
        image: "example.invalid/image:1".to_string(),
        disk: "/dev/vda".to_string(),
        partition: "/dev/vda3".to_string(),
        key: Key::Passphrase("recovery".to_string()),
        pin: false,
    };
    let command = kill_slot_command(&ready, 7);
    assert_eq!(command.get_program(), "cryptsetup");
    let words: Vec<String> = command
        .get_args()
        .map(|word| word.to_string_lossy().to_string())
        .collect();
    assert_eq!(
        words,
        ["-q", "luksKillSlot", "--key-file", "-", "/dev/vda3", "7"]
    );
}

/// The marker is written before the seal, so a seal that fails still leaves
/// the first boot the slot number that has to be wiped. Without the marker the
/// boot unit wipes no slot, removes no credential and shreds its own key.
#[test]
fn a_failed_seal_leaves_the_slot_marker_behind() {
    let esp = scratch("automatic-marker");
    let failed = credential_with(&esp, 7, |_| Err("the seal failed".to_string()));
    assert_eq!(failed.unwrap_err(), "the seal failed");
    let marker = esp.join(CREDENTIAL_DIR).join(SLOT_FILE);
    assert_eq!(std::fs::read_to_string(&marker).expect("the marker"), "7\n");
    let _ = std::fs::remove_dir_all(&esp);
}

/// The completion screen offers no automatic action until a first-boot
/// enrolment is staged, and it names the missing stub before it spends a
/// probe on the image.
#[test]
fn a_grub_image_is_refused_before_the_image_probe() {
    let payload = a_payload();
    assert!(payload.boot.is_empty());
    match offered(&payload, &answers("tpm2-luks", ""), Some("recovery")) {
        Offered::Unavailable(why) => assert_eq!(why, copy::AUTO_NO_STUB),
        _ => panic!("a chain without systemd-stub was offered the action"),
    }
    match offered(&payload, &answers("none", ""), None) {
        Offered::None => {}
        _ => panic!("an install with no enrolment was offered the action"),
    }
}

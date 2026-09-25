use super::*;

/// The automatic layout's root is the partition fisherman retags for GPT
/// auto-discovery, and the credential goes to the partition systemd-stub
/// boots from. A second match leaves no single one, and a listing with no
/// match leaves none at all.
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
        data: Data::default(),
        layout: None,
        opened: Opened::Keep,
    }
}

/// The kinds that stage a first-boot enrolment are the ones whose completion
/// screen offers to finalize. The recovery key of a generated kind arrives as
/// an event, and the passphrase kind carries its own.
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

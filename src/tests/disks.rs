use super::*;

/// One kind sits at index 3 in the full list and at index 2 in the
/// container list, because the container list leaves `none` out.
#[test]
fn the_luks_window_opens_on_the_kind_in_its_own_list() {
    assert_eq!(
        kind_at(&kinds(true, true, true), "tpm2-luks-passphrase"),
        Some(3)
    );
    let container = kinds(true, true, false);
    assert_eq!(kind_at(&container, "tpm2-luks-passphrase"), Some(2));
    assert_eq!(kind_at(&container, NONE), None);
}

/// Every kind carries two strings for one answer. This walks all five from
/// the drawn description to the name the recipe holds.
#[test]
fn every_shown_description_is_written_as_the_name_fisherman_takes() {
    let root = scratch("kinds");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
    for (name, label, _) in KINDS {
        let answers = Answers {
            disk: "/dev/vda".to_string(),
            hostname: "deb2".to_string(),
            user: "tect".to_string(),
            password: "hunter2".to_string(),
            opened: Opened::Keep,
            encryption: Encryption {
                kind: name.to_string(),
                passphrase: "opensesame".to_string(),
                pin: "4321".to_string(),
            },
            data: Data::default(),
            layout: None,
        };
        let fields = answers.fields(&a_payload(), &Scan::default(), "/dev/vda", None, "");
        assert_eq!(fields[ROW_ENCRYPTION].value(), label);
        let read = Answers::of(&fields, "/dev/vda".to_string(), None);
        assert_eq!(read.encryption.kind, name);
        let done = complete(&recipe, &read).expect("a completed recipe");
        let encryption = json::field(&done, "encryption").expect("an encryption");
        assert_eq!(json::text(encryption, "type").as_deref(), Some(name));
        // The summary says the description back, because that is what the
        // user picked.
        assert_eq!(shown(name), label);
        // The flag path takes fisherman's name for the same kind.
        let flagged = ask_encryption(
            Some(name.to_string()),
            Some("opensesame".to_string()),
            Some("4321".to_string()),
            &Prompt::silent(),
            true,
            true,
        )
        .expect("one of the five");
        assert_eq!(flagged.kind, name);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The home row offers two answers. Only the separate answer reads back a
/// size, which is what tells the two apart.
#[test]
fn the_home_row_offers_together_or_a_partition_of_its_own() {
    let rows = data_rows(false);
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, [copy::DATA_TOGETHER, copy::DATA_SEPARATE]);
    assert!(rows.iter().all(|row| row.available));
    assert!(chose(copy::DATA_TOGETHER, "20 GB").size.is_empty());
    assert_eq!(chose(copy::DATA_SEPARATE, "20 GB").size, "20 GB");
    assert_eq!(data_said(&Data::default()), copy::DATA_TOGETHER);
    assert_eq!(
        data_said(&Data {
            size: "20 GB".to_string()
        }),
        copy::DATA_SEPARATE
    );
    // A composefs target hides the separate answer rather than cutting a
    // `/var` the boot never mounts.
    let locked = data_rows(true);
    assert_eq!(locked[0].label, copy::DATA_TOGETHER);
    assert!(!locked[0].hidden);
    assert_eq!(locked[1].label, copy::DATA_SEPARATE);
    assert!(locked[1].hidden);
}

/// A separate home is written as fisherman's `varDisk` size. The size the
/// screen holds is written in whole GB.
#[test]
fn a_separate_home_writes_a_size_and_sharing_the_root_writes_nothing() {
    let root = scratch("var");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
    let written = |data: Data, kind: &str| {
        let answers = Answers {
            disk: "/dev/vda".to_string(),
            hostname: "deb2".to_string(),
            user: "tect".to_string(),
            password: "hunter2".to_string(),
            opened: Opened::Keep,
            encryption: Encryption {
                kind: kind.to_string(),
                passphrase: String::new(),
                pin: String::new(),
            },
            data,
            layout: None,
        };
        complete(&recipe, &answers).expect("a completed recipe")
    };
    let held = |doc: &Json, key: &str| {
        json::field(doc, "varDisk")
            .and_then(|var| json::field(var, key))
            .map(|value| value.render().trim().to_string())
    };

    let sized = written(chose(copy::DATA_SEPARATE, "200 GB"), NONE);
    assert_eq!(held(&sized, "size").as_deref(), Some("\"200GB\""));
    // An unencrypted root writes no `encrypt` key at all, which fisherman
    // reads as an unencrypted home.
    assert_eq!(held(&sized, "encrypt"), None);

    // Under an encrypted root the home partition carries `encrypt`, and
    // fisherman opens it with the root's passphrase.
    let encrypted = written(chose(copy::DATA_SEPARATE, "200 GB"), "luks-passphrase");
    assert_eq!(held(&encrypted, "encrypt").as_deref(), Some("true"));

    // A shared home writes nothing, so a `var-disk` the image declared
    // stands as `emit::recipe` wrote it.
    assert!(json::field(&written(Data::default(), NONE), "varDisk").is_none());
    assert!(json::field(&written(Data::default(), "tpm2-luks"), "varDisk").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// A machine with no TPM marks its three `tpm2-` rows unavailable and
/// hidden, so neither the drawn menu nor the numbered prompt offers one.
#[test]
fn the_tpm_forms_are_hidden_where_there_is_no_tpm() {
    let rows = kinds(false, true, true);
    let without: Vec<(&str, &str, bool, bool)> = rows
        .iter()
        .map(|choice| {
            (
                choice.label.as_str(),
                choice.detail.as_str(),
                choice.available,
                choice.hidden,
            )
        })
        .collect();
    // The label is the whole description. All three `tpm2-` rows carry
    // `NO_TPM` as their reason.
    assert_eq!(
        without,
        vec![
            (copy::ENC_NONE, "", true, false),
            (copy::ENC_TPM2, copy::NO_TPM, false, true),
            (copy::ENC_PASSPHRASE, "", true, false),
            (copy::ENC_BOTH, copy::NO_TPM, false, true),
            (copy::ENC_TPM2_PIN, copy::NO_TPM, false, true),
        ]
    );
    let with = kinds(true, true, true);
    assert!(with.iter().all(|choice| choice.available && !choice.hidden));
    assert!(with.iter().all(|choice| choice.detail.is_empty()));
    // The container window leaves `none` out, so it holds four rows.
    let container = kinds(true, true, false);
    assert_eq!(container.len(), 4);
    assert!(container
        .iter()
        .all(|choice| choice.label != copy::ENC_NONE));
    let numbered = visible_kinds(false, true);
    assert_eq!(
        numbered
            .iter()
            .map(|(at, choice)| (*at, choice.label.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, copy::ENC_NONE), (2, copy::ENC_PASSPHRASE)]
    );
}

/// A kind fisherman does not take is refused before the user is asked
/// anything. The refusal names the five kinds fisherman does take.
#[test]
fn an_encryption_no_backend_takes_is_refused_by_name() {
    let refused = ask_encryption(
        Some("luks".to_string()),
        None,
        None,
        &Prompt::silent(),
        true,
        true,
    )
    // `.err()` is used because `unwrap_err` would want a `Debug` on
    // `Encryption`, which holds a passphrase.
    .err()
    .expect("a refusal");
    assert!(refused.contains("tpm2-luks-passphrase"), "{refused}");
    let kept = ask_encryption(
        Some("tpm2-luks".to_string()),
        None,
        None,
        &Prompt::silent(),
        true,
        true,
    )
    .expect("one of the five");
    assert_eq!(kept.kind, "tpm2-luks");
    assert!(kept.passphrase.is_empty());
}

/// A headless run has no menu to draw a `tpm2-` kind out of, so the flag
/// itself has to refuse and stop.
#[test]
fn a_headless_tpm2_kind_is_refused_without_a_tpm() {
    assert_eq!(
        ask_encryption(
            Some("tpm2-luks".to_string()),
            None,
            None,
            &Prompt::silent(),
            false,
            true
        )
        .err()
        .as_deref(),
        Some(copy::NO_TPM)
    );
}

/// Builds a fake `/sys/block` holding one device per rule the disk list
/// applies. Two disks survive every rule and the other four are dropped.
#[test]
fn only_the_disks_a_person_could_install_onto_are_offered() {
    let sys = scratch("sys");
    let block = |name: &str, sectors: &str, model: Option<&str>, removable: &str, ro: &str| {
        let at = sys.join(name);
        std::fs::create_dir_all(at.join("device")).expect("a block device");
        std::fs::write(at.join("size"), sectors).expect("a size");
        std::fs::write(at.join("removable"), removable).expect("a removable");
        std::fs::write(at.join("ro"), ro).expect("a read-only flag");
        if let Some(model) = model {
            std::fs::write(at.join("device/model"), model).expect("a model");
        }
    };
    block(
        "sda",
        "937703088\n",
        Some("Samsung SSD 980\n"),
        "0\n",
        "0\n",
    );
    block("sdb", "60088320\n", None, "1\n", "0\n");
    block("loop0", "204800\n", None, "0\n", "0\n");
    // A card reader with no card in it reads zero sectors.
    block("sdc", "0\n", None, "1\n", "0\n");
    // A virtual machine attaches an iso as a read-only medium.
    block(
        "sdd",
        "6291456\n",
        Some("QEMU USB HARDDRIVE\n"),
        "1\n",
        "1\n",
    );
    // A stick written with `dd` stays writable, so the device itself gives
    // the rule nothing to go on. The live root mounted from `sde1` is the
    // only sign this stick is the installer medium.
    block("sde", "60088320\n", Some("Cruzer Blade\n"), "1\n", "0\n");
    let part = sys.join("sde/sde1");
    std::fs::create_dir_all(&part).expect("a partition");
    std::fs::write(part.join("partition"), "1\n").expect("a partition number");
    let mounts = "\
/dev/sde1 /run/initramfs/live iso9660 ro,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev 0 0
";

    assert_eq!(
        disks(&sys, mounts),
        vec![
            (
                "/dev/sda".to_string(),
                "480.1 GB  Samsung SSD 980".to_string()
            ),
            (
                "/dev/sdb".to_string(),
                format!("30.8 GB  {}", copy::REMOVABLE)
            ),
        ]
    );
    // With nothing mounted from it the same stick is offered again. The
    // rule hides the installer medium and never the user's spare drive.
    assert!(disks(&sys, "").iter().any(|(disk, _)| disk == "/dev/sde"));
    let _ = std::fs::remove_dir_all(&sys);
}

/// A header's slots are read by index, so a gap in the numbering stays a
/// gap. Slot 1 is absent here and slot 2 keeps its own number.
#[test]
fn a_headers_slots_are_read_by_index_and_the_tokens_beside_them() {
    let raw = r#"{"keyslots":{"0":{"type":"luks2"},"2":{"type":"luks2"}},
                      "tokens":{"0":{"type":"systemd-tpm2"}}}"#;
    let slots = slots_from(raw).expect("a luksDump document");
    assert_eq!(slots.keys, vec![0, 2]);
    assert_eq!(slots.tokens, vec!["systemd-tpm2".to_string()]);
    assert!(slots.has_tpm2());
    let said = copy::slots_said(&slots.keys, &slots.tokens);
    assert!(said.contains("slot 2"), "{said}");
    assert!(said.contains("systemd-tpm2"), "{said}");
}

/// Two answers put a key on the root. A key file opens it at boot, and a
/// first-boot TPM2 enrolment stages its key. Both are drawn unpickable
/// with the reason while the root is a plain mount.
#[test]
fn a_key_on_the_root_needs_an_encrypted_root() {
    let volume = |target: &str, partition: &str| LuksOpen {
        partition: partition.to_string(),
        target: target.to_string(),
        key: Key::Passphrase("opensesame".to_string()),
    };
    let layout = |encrypted_root: bool| {
        let mounts = match encrypted_root {
            true => Vec::new(),
            false => vec![CustomMount {
                partition: "/dev/vda2".to_string(),
                target: "/".to_string(),
                fstype: "ext4".to_string(),

                passphrase: String::new(),
            }],
        };
        let mut opens = vec![volume("/var", "/dev/tect-test-no-luks")];
        if encrypted_root {
            opens.insert(0, volume("/", "/dev/tect-test-no-luks-root"));
        }
        CustomLayout {
            disk: "/dev/vda".to_string(),
            mounts,
            opens,
            ..Default::default()
        }
    };
    fn row<'a>(rows: &'a [Choice], label: &str) -> &'a Choice {
        rows.iter().find(|row| row.label == label).expect("a row")
    }
    let plain = opened_rows(&layout(false), true, true);
    for label in [copy::OPENED_ADD_KEY, copy::OPENED_TPM2] {
        let row = row(&plain, label);
        assert!(!row.available, "{label}");
        assert!(row.hidden, "{label}");
        assert_eq!(row.detail, copy::OPENED_KEYFILE_PLAIN, "{label}");
    }
    // The fake device has no readable header, so `OPENED_KEEP` carries the
    // unreadable reason and the two additions show the offered state.
    let encrypted = opened_rows(&layout(true), true, true);
    assert!(row(&encrypted, copy::OPENED_ADD_KEY).available);
    assert_eq!(
        row(&encrypted, copy::OPENED_ADD_KEY).detail,
        copy::OPENED_ADD_KEY_COST
    );
    assert!(row(&encrypted, copy::OPENED_KEEP)
        .detail
        .starts_with("its slots could not be read"));
}

/// The summary's home row names the answer the user picked. The size sits
/// on the home partition's own row under it.
#[test]
fn the_home_answer_reads_as_together_or_separate() {
    assert_eq!(data_said(&Data::default()), copy::DATA_TOGETHER);
    let separate = chose(copy::DATA_SEPARATE, "200 GB");
    assert_eq!(data_said(&separate), copy::DATA_SEPARATE);
    // The separate answer keeps the size fisherman cuts the home to.
    assert_eq!(separate.size, "200 GB");
}

/// The installer hands the payload's sealing to the home row, so a composefs
/// target hides the answer that cuts a `/var` its boot never mounts. A
/// non-composefs payload still offers it.
#[test]
fn a_composefs_target_hides_the_separate_home_answer() {
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
        layout: None,
    };
    let separate_hidden = |composefs: bool| -> bool {
        let mut payload = a_payload();
        payload.composefs = composefs;
        let fields = answers.fields(&payload, &Scan::default(), "/dev/vda", None, "");
        let common::ui::Field::Pick { options, .. } = &fields[ROW_DATA] else {
            panic!("the home row is a pick");
        };
        options
            .iter()
            .find(|choice| choice.label == copy::DATA_SEPARATE)
            .expect("the separate answer keeps its index")
            .hidden
    };
    assert!(separate_hidden(true));
    assert!(!separate_hidden(false));
}

use super::*;

/// A root can hold both a payload and a repository, so precedence matters.
#[test]
fn a_payload_wins_over_the_repository_that_would_have_to_be_built() {
    let root = scratch("cases");
    assert!(matches!(classify(&root), Ok(Found::Nothing(_))));

    std::fs::write(root.join(crate::REPO_FILE), "repo {\n}\n").expect("a repo.kdl");
    assert!(matches!(classify(&root), Ok(Found::Repo(_))));

    // A payload wins over a repository, because its bytes are already built.
    std::fs::write(root.join(RECIPE), EMITTED).expect("a recipe");
    let Ok(Found::Image(payload)) = classify(&root) else {
        panic!("a payload beside a repository is still a payload");
    };
    assert_eq!(payload.image, "ghcr.io/tectonic-os/deb2:latest");
    assert_eq!(payload.hostname, "deb2");
    assert!(payload.boot.is_empty());
    assert!(!payload.luks_initramfs, "an old recipe proves nothing");
    // `EMITTED` carries `composeFsBackend`, so this row reads true.
    assert!(payload.composefs);

    let uki = EMITTED.replace(
        "\"bootloader\": \"grub2\",",
        "\"bootloader\": \"systemd\",\n  \"boot\": \"uki-db\",\n  \"luksInitramfs\": true,",
    );
    std::fs::write(root.join(RECIPE), uki).expect("a UKI recipe");
    let Ok(Found::Image(payload)) = classify(&root) else {
        panic!("a UKI recipe is a payload");
    };
    assert_eq!(payload.boot, "uki-db");
    assert!(payload.luks_initramfs);

    // A broken recipe refuses rather than falling through to `Nothing`.
    std::fs::write(root.join(RECIPE), "{").expect("a broken recipe");
    assert!(classify(&root).is_err());
    std::fs::write(root.join(RECIPE), "{}").expect("an empty recipe");
    assert!(classify(&root)
        .unwrap_err()
        .contains(&format!("{RECIPE}: no `image`")));
    let _ = std::fs::remove_dir_all(&root);
}

/// `classify` requires `image` and `hostname`, and a recipe of those two
/// alone still cannot install. `complete` never writes `filesystem`, and
/// fisherman refuses an auto-partitioning recipe whose `filesystem` is not
/// one of xfs, ext4, btrfs or zfs. The third field must survive `complete`
/// untouched, because nothing downstream supplies it.
#[test]
fn three_hand_written_fields_are_the_floor_for_a_generic_bootc_image() {
    let root = scratch("non-tectonic");
    let recipe = root.join(RECIPE);
    let hand_written = "{\n  \"image\": \"quay.io/fedora/fedora-bootc:42\",\n  \
             \"hostname\": \"workstation\",\n  \"filesystem\": \"ext4\"\n}";
    std::fs::write(&recipe, hand_written).expect("a hand-written recipe");
    let Ok(Found::Image(payload)) = classify(&root) else {
        panic!("three fields are a payload");
    };
    assert_eq!(payload.image, "quay.io/fedora/fedora-bootc:42");
    assert_eq!(payload.filesystem, "ext4");
    // An absent `bootloader` reads as `grub2`, so the summary still draws a
    // separate `/boot` row. An absent `boot` draws no boot chain row.
    assert!(payload.bootloader.is_empty());
    assert!(payload.boot.is_empty());
    assert!(!payload.composefs);
    // Without a LUKS initramfs every encrypted kind is refused, so a hand
    // written recipe installs the machine plain.
    assert!(!payload.luks_initramfs);
    // The `none` kind is left out, so every row here is an encrypted kind.
    assert!(kinds(true, payload.luks_initramfs, false)
        .iter()
        .all(|kind| !kind.available));

    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "workstation".to_string(),
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
    let done = complete(&recipe, &answers).expect("the person's half goes in");
    // The user answers `disk` and `hostname`. The hand written recipe carries
    // `image` and `filesystem` through `complete` untouched.
    for (key, value) in [
        ("disk", "/dev/vda"),
        ("hostname", "workstation"),
        ("image", "quay.io/fedora/fedora-bootc:42"),
        ("filesystem", "ext4"),
    ] {
        assert_eq!(json::text(&done, key).as_deref(), Some(value), "{key}");
    }
    // No base family derived an admin group here, so the account goes in
    // with no groups.
    let user = json::field(&done, "user").expect("an account");
    assert_eq!(json::text(user, "username").as_deref(), Some("tect"));
    assert!(json::strings(user, "groups").is_empty());

    // A two field recipe still classifies, and `complete` leaves it with no
    // `filesystem` for fisherman to take.
    std::fs::write(
        &recipe,
        "{\n  \"image\": \"quay.io/fedora/fedora-bootc:42\",\n  \
             \"hostname\": \"workstation\"\n}",
    )
    .expect("a two-field recipe");
    assert!(matches!(classify(&root), Ok(Found::Image(_))));
    let thin = complete(&recipe, &answers).expect("the person's half goes in");
    assert_eq!(json::text(&thin, "filesystem"), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// An image the installer cannot measure reserves 10 GB.
#[test]
fn the_root_reserve_is_twice_the_image_plus_slack() {
    assert_eq!(root_reserve(&Some(4)), 10);
    assert_eq!(root_reserve(&None), 10);
    assert_eq!(root_reserve(&Some(20)), 42);
}

/// `labelled` keeps every device the scan named, because `root` refuses a
/// second labelled partition rather than picking one.
#[test]
fn more_than_one_labelled_partition_is_named_rather_than_chosen() {
    assert!(labelled("\n").is_empty());
    assert_eq!(labelled("/dev/sdb2\n"), ["/dev/sdb2"]);
    assert_eq!(
        labelled("/dev/sdb2\n/dev/sdc1\n"),
        ["/dev/sdb2", "/dev/sdc1"]
    );
}

/// Each root that cannot install names what would make it installable.
#[test]
fn a_root_with_nothing_built_on_it_refuses_by_name() {
    let nothing = Found::Nothing("/mnt/tect".into()).payload().unwrap_err();
    assert!(nothing.contains(RECIPE) && nothing.contains(crate::REPO_FILE));
    assert!(nothing.contains("/mnt/tect"), "{nothing}");
    let repo = Found::Repo("/mnt/tect".into()).payload().unwrap_err();
    assert!(repo.contains("tect build"), "{repo}");
}

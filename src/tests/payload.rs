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

/// A hand-written recipe can use the image as its update reference, but it
/// must name the media store that carries the image.
#[test]
fn a_hand_written_recipe_names_the_media_store() {
    let root = scratch("non-tectonic");
    let recipe = root.join(RECIPE);
    let hand_written = "{\n  \"image\": \"quay.io/fedora/fedora-bootc:42\",\n  \
             \"hostname\": \"workstation\",\n  \"filesystem\": \"ext4\",\n  \
             \"bootloader\": \"grub2\",\n  \
             \"additionalImageStores\": [\"/var/lib/tectonic/store\"]\n}";
    std::fs::write(&recipe, hand_written).expect("a hand-written recipe");
    let Ok(Found::Image(payload)) = classify(&root) else {
        panic!("a hand-written recipe with a store is a payload");
    };
    assert_eq!(payload.image, "quay.io/fedora/fedora-bootc:42");
    assert_eq!(payload.filesystem, "ext4");
    // An absent `boot` draws no boot chain row.
    assert_eq!(payload.bootloader, "grub2");
    assert!(payload.boot.is_empty());
    assert!(!payload.composefs);
    // Without a LUKS initramfs every encrypted kind is refused, so a hand
    // written recipe installs the machine plain.
    assert!(!payload.luks_initramfs);
    assert_eq!(payload.install.target_imgref, payload.image);
    assert_eq!(payload.install.stores, ["/var/lib/tectonic/store"]);
    // The `none` kind is left out, so every row here is an encrypted kind.
    assert!(kinds(true, payload.luks_initramfs, false)
        .iter()
        .all(|kind| !kind.available));

    std::fs::write(
        &recipe,
        "{\"image\":\"image\",\"hostname\":\"host\",\"filesystem\":\"ext4\",\
         \"bootloader\":\"grub2\"}",
    )
    .expect("a recipe without a store");
    let refused = classify(&root).unwrap_err();
    assert!(refused.contains("additionalImageStores"), "{refused}");
    let _ = std::fs::remove_dir_all(&root);
}

/// An image the installer cannot measure reserves 10 GB.
#[test]
fn the_root_reserve_is_twice_the_image_plus_slack() {
    assert_eq!(root_reserve(&Some(4)), 10);
    assert_eq!(root_reserve(&None), 10);
    assert_eq!(root_reserve(&Some(20)), 42);
}

/// The probe script's status says the listing ran. A `[ -d ]` that is false
/// on the last pattern otherwise reports a successful listing as a failure,
/// and the picture loses the entries the image writes. Both roots are given
/// a scratch tree, so the unmatched glob is forced on every host.
#[test]
fn the_image_probe_reports_a_listing_that_ran() {
    let probe = |payload: &Path, boot: &Path| {
        std::process::Command::new("sh")
            .args([
                "-c",
                payload::ESP_LIST,
                "probe",
                &payload.to_string_lossy(),
                &boot.to_string_lossy(),
            ])
            .output()
            .expect("a shell to run the probe")
    };
    let root = scratch("probe-list");
    std::fs::create_dir_all(root.join("usr/lib/efi/grub2/1/EFI/fedora")).expect("an entry");
    let out = probe(&root.join("usr/lib/efi"), &root.join("boot/EFI"));
    assert!(out.status.success(), "the probe exits {}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        root.join("usr/lib/efi/grub2/1/EFI/fedora")
            .to_string_lossy()
    );
    // A root carrying no entry reports success and prints nothing. The old
    // `&&` form reported the last unmatched glob as a failed listing.
    let empty = scratch("probe-empty");
    let out = probe(&empty.join("usr/lib/efi"), &empty.join("boot/EFI"));
    assert!(out.status.success(), "the probe exits {}", out.status);
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&empty);
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

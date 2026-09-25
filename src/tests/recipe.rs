use super::*;

/// `complete` writes the user's half into the recipe `emit::recipe::build`
/// already wrote. Fisherman reads both halves from the one file.
#[test]
fn the_completed_recipe_keeps_every_derived_field_and_hashes_the_password() {
    let root = scratch("complete");
    let recipe = root.join(RECIPE);
    std::fs::write(&recipe, EMITTED).expect("a recipe");
    let answers = Answers {
        disk: "/dev/vda".to_string(),
        hostname: "deb2".to_string(),
        user: "tect".to_string(),
        password: "hunter2".to_string(),
        opened: Opened::Keep,
        encryption: Encryption {
            kind: "luks-passphrase".to_string(),
            passphrase: "opensesame".to_string(),
            pin: String::new(),
        },
        data: Data::default(),
        layout: None,
    };
    let done = complete(&recipe, &answers).expect("the person's half goes in");

    // The image derived these six fields. `complete` must not rewrite them.
    for (key, value) in [
        ("image", "\"ghcr.io/tectonic-os/deb2:latest\""),
        ("composeFsBackend", "true"),
        ("bootloader", "\"grub2\""),
        ("filesystem", "\"ext4\""),
        ("hostname", "\"deb2\""),
        ("disk", "\"/dev/vda\""),
    ] {
        let held = json::field(&done, key).map(|v| v.render().trim().to_string());
        assert_eq!(held.as_deref(), Some(value), "{key}");
    }

    // `emit::recipe::build` derived `sudo` for this Debian target, and the
    // merged account has to leave that group in place.
    let user = json::field(&done, "user").expect("an account");
    assert_eq!(json::strings(user, "groups"), ["sudo"]);
    assert_eq!(json::text(user, "username").as_deref(), Some("tect"));
    let hash = json::text(user, "password").expect("a password");
    assert!(hash.starts_with("$6$"), "{hash}");
    assert!(!hash.contains("hunter2"), "{hash}");

    // `complete` writes the kind, the passphrase and the pin as one
    // `encryption` object, so the recipe never names a kind with no key.
    let encryption = json::field(&done, "encryption").expect("an encryption");
    assert_eq!(
        json::text(encryption, "type").as_deref(),
        Some("luks-passphrase")
    );
    assert_eq!(
        json::text(encryption, "passphrase").as_deref(),
        Some("opensesame")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A pending Format answer decides the list before lsblk's filesystem does,
/// which is why a blank partition answered `fat32` is offered `/boot/efi`.
/// The Format overlay offers every `format_options` entry instead, because
/// a Format answer wins over the mount point the partition already holds.
#[test]
fn the_assign_list_follows_the_filesystem() {
    let partition = |fstype: &str| Partition {
        device: "/dev/vda1".to_string(),
        size: "512M".to_string(),
        fstype: fstype.to_string(),
        label: String::new(),
        parttype: String::new(),
        uuid: String::new(),
    };
    assert_eq!(
        assigns(&partition("vfat"), None, true, false),
        ["/boot/efi"]
    );
    assert_eq!(
        assigns(&partition("ext4"), None, true, false),
        ["/", "/boot", "/var", "/var/home"]
    );
    // A systemd-boot target reads its kernel from the ESP, so a separate
    // `/boot` is no answer it can use.
    assert_eq!(
        assigns(&partition("ext4"), None, false, false),
        ["/", "/var", "/var/home"]
    );
    assert_eq!(assigns(&partition("swap"), None, true, false), ["/swap"]);
    assert!(assigns(&partition(""), None, true, false).is_empty());
    // A composefs target binds `/var` from the root, so `/var` itself is no
    // answer there; a point under it still mounts.
    assert_eq!(
        assigns(&partition("ext4"), None, true, true),
        ["/", "/boot", "/var/home"]
    );
    let chosen = Mounted {
        target: String::new(),
        fstype: "fat32".to_string(),
    };
    assert_eq!(
        assigns(&partition(""), Some(&chosen), true, false),
        ["/boot/efi"]
    );
}

/// A `stage` that stops returning a guard leaves the staged recipe in
/// `TMPDIR`. The passphrase inside is then readable at the installer's
/// root console.
#[test]
fn the_staged_recipe_is_removed_when_its_guard_drops() {
    let recipe = Json::parse(r#"{"hostname":"host"}"#).expect("the test recipe parses");
    let path = {
        let staged = stage(&recipe).expect("the recipe stages");
        assert!(
            staged.0.exists(),
            "the recipe is readable while fisherman needs it"
        );
        staged.0.clone()
    };
    assert!(!path.exists(), "the guard removed the staged recipe");
}

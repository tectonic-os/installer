use super::*;

#[test]
fn the_typed_recipe_holds_every_field_the_install_uses() {
    let root = scratch("typed-recipe");
    let path = root.join(RECIPE);
    std::fs::write(&path, EMITTED).expect("a recipe");
    let recipe = InstallRecipe::read(&path).expect("the emitted recipe is supported");

    assert_eq!(recipe.image, "ghcr.io/tectonic-os/deb2:latest");
    assert_eq!(recipe.target_imgref, recipe.image);
    assert!(recipe.composefs && recipe.generic);
    assert_eq!(recipe.bootloader, "grub2");
    assert_eq!(recipe.groups, ["sudo"]);
    assert_eq!(recipe.stores, ["/var/lib/tectonic/store"]);
    let _ = std::fs::remove_dir_all(root);
}

/// A legacy field can name another disk, so the refusal must happen before the
/// installer changes the disk the user reviewed.
#[test]
fn a_recipe_field_the_installer_does_not_implement_is_refused_by_name() {
    let root = scratch("unsupported-recipe");
    let path = root.join(RECIPE);
    let raw = EMITTED.replace(
        "\"user\": { \"groups\": [\"sudo\"] },",
        "\"user\": { \"groups\": [\"sudo\"] },\n  \"varDisk\": { \"disk\": \"/dev/sdb\" },",
    );
    std::fs::write(&path, raw).expect("a recipe");
    let refused = InstallRecipe::read(&path)
        .err()
        .expect("varDisk is not an installer field");
    assert!(refused.contains("`varDisk`") && refused.contains("not implemented"));
    let _ = std::fs::remove_dir_all(root);
}

/// A default bootloader would cut a GRUB `/boot` for a systemd-boot image
/// before the installer noticed the image had no GRUB.
#[test]
fn a_recipe_without_a_bootloader_is_refused_by_name() {
    let root = scratch("no-bootloader-recipe");
    let path = root.join(RECIPE);
    let raw: String = EMITTED
        .lines()
        .filter(|line| !line.contains("\"bootloader\""))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, raw).expect("a recipe");
    let refused = InstallRecipe::read(&path)
        .err()
        .expect("a recipe without a bootloader is refused");
    assert!(refused.contains("`bootloader`"), "{refused}");
    let _ = std::fs::remove_dir_all(root);
}

/// The install formats an opened root with the recipe's `filesystem`, and no
/// editor rule weighs it, so a sealed recipe on xfs must stop at the read.
#[test]
fn a_sealed_recipe_on_a_filesystem_without_verity_is_refused() {
    let root = scratch("sealed-xfs-recipe");
    let path = root.join(RECIPE);
    let raw = EMITTED.replace("\"filesystem\": \"ext4\"", "\"filesystem\": \"xfs\"");
    assert_ne!(raw, EMITTED);
    std::fs::write(&path, raw).expect("a recipe");
    let refused = InstallRecipe::read(&path)
        .err()
        .expect("a sealed recipe on xfs is refused");
    assert!(refused.contains("`composeFsBackend`"), "{refused}");
    let _ = std::fs::remove_dir_all(root);
}

/// A pending Format answer decides the list before lsblk's filesystem does,
/// which is why a blank partition answered `fat32` is offered `/boot/efi`.
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
    assert_eq!(assigns(&partition("vfat"), None, true), ["/boot/efi"]);
    assert_eq!(assigns(&partition("ext4"), None, true), ["/", "/boot"]);
    assert_eq!(assigns(&partition("ext4"), None, false), ["/"]);
    assert_eq!(assigns(&partition("swap"), None, true), ["/swap"]);
    assert!(assigns(&partition(""), None, true).is_empty());
    let chosen = Mounted {
        target: String::new(),
        fstype: "fat32".to_string(),
    };
    assert_eq!(assigns(&partition(""), Some(&chosen), true), ["/boot/efi"]);
}

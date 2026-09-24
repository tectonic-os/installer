use super::*;

#[test]
fn a_grub_search_names_the_boot_filesystem_it_looks_for() {
    // The installed ESP carries bootupd's static file. Its UUID branch names
    // a shell variable this walk cannot resolve, and the label its fallback
    // names is the one the image labels /boot with.
    let static_cfg = "\
if [ -f ${config_directory}/bootuuid.cfg ]; then
  source ${config_directory}/bootuuid.cfg
fi
if [ -n \"${BOOT_UUID}\" ]; then
  search --fs-uuid \"${BOOT_UUID}\" --set prefix --no-floppy
else
  search --label boot --set prefix --no-floppy
fi
";
    assert_eq!(searched(static_cfg), Some(Link::Label("boot".to_string())));
    // A file whose only search names a variable links to nothing.
    assert_eq!(
        searched("search --fs-uuid \"${BOOT_UUID}\" --set prefix --no-floppy\n"),
        None
    );
    assert_eq!(
        searched("search --no-floppy --fs-uuid --set=dev 8E0615FB-7297-4B74-97C5-B9B84C0F36BA\n"),
        Some(Link::Uuid(
            "8E0615FB-7297-4B74-97C5-B9B84C0F36BA".to_string()
        ))
    );
    assert_eq!(
        searched("search --label boot --set prefix --no-floppy\n"),
        Some(Link::Label("boot".to_string()))
    );
    // A `--set` before the name takes its own variable, and a later flag
    // never becomes the name.
    assert_eq!(
        searched("search --fs-uuid --set=dev 8E0615FB --label boot\n"),
        Some(Link::Uuid("8E0615FB".to_string()))
    );
    assert_eq!(
        searched("search --fs-uuid --set root 8E0615FB\n"),
        Some(Link::Uuid("8E0615FB".to_string()))
    );
    assert_eq!(
        searched("search.fs_uuid 8e0615fb root\n"),
        Some(Link::Uuid("8e0615fb".to_string()))
    );
    assert_eq!(
        searched("set prefix=($dev)/grub2\nconfigfile $prefix/grub2.cfg\n"),
        None
    );
}

#[test]
fn a_bootuuid_file_names_the_boot_filesystem() {
    assert_eq!(
        bootuuid("set BOOT_UUID=\"8e0615fb-7297-4b74-97c5-b9b84c0f36ba\"\n"),
        Some("8e0615fb-7297-4b74-97c5-b9b84c0f36ba".to_string())
    );
    // An unset variable and an empty value name no filesystem.
    assert_eq!(bootuuid("set BOOT_UUID=\"${BOOT_UUID}\"\n"), None);
    assert_eq!(bootuuid("set BOOT_UUID=\"\"\n"), None);
    assert_eq!(bootuuid(""), None);
}

#[test]
fn a_bls_entry_names_its_root_and_the_container_beside_it() {
    let measured = "title Falcos 20260913 (ostree:0)\n\
        options ostree=/ostree/boot.0/default/71a22e00/0 \
        rd.luks.uuid=luks-37be8ca7-8834-4aca-aee0-dd280af10939 rhgb quiet \
        root=UUID=6c38207b-7766-446b-a210-b438ae22aa2b rootflags=subvol=root rw\n";
    // The root filesystem leads, so a partition it names wins over the
    // closed container that holds it.
    assert_eq!(
        bls_links(measured),
        [
            Link::Uuid("6c38207b-7766-446b-a210-b438ae22aa2b".to_string()),
            Link::Uuid("37be8ca7-8834-4aca-aee0-dd280af10939".to_string()),
        ]
    );
    assert_eq!(
        bls_links("title x\noptions root=UUID=aa11 rw\n"),
        [Link::Uuid("aa11".to_string())]
    );
    assert_eq!(bls_links("title x\nlinux /vmlinuz\n"), []);
}

#[test]
fn an_image_lists_the_entries_its_staged_payloads_carry() {
    let measured = "\
/usr/lib/efi/grub2/1:2.12-64.fc44/EFI/fedora
/usr/lib/efi/shim/16.1-5/EFI/BOOT
/usr/lib/efi/shim/16.1-5/EFI/fedora
";
    assert_eq!(entry_names(measured), ["fedora"]);
    // A UKI image carries its entry under `/boot`, and a name repeated under
    // two payloads lands once.
    assert_eq!(
        entry_names("/boot/EFI/Linux\n/usr/lib/efi/systemd-boot/1/EFI/systemd\n"),
        ["Linux", "systemd"]
    );
    assert_eq!(
        entry_names("/usr/lib/efi/shim/1/EFI/BOOT\n"),
        Vec::<String>::new()
    );
    assert_eq!(entry_names(""), Vec::<String>::new());
}

#[test]
fn a_microsoft_entry_never_takes_the_esps_loader_link() {
    // A systemd-boot ESP holds the loader entries, and Windows writes its
    // boot manager to the same ESP. Formatting the Linux root must not take
    // the Windows entry with it, so the entries never become Microsoft's
    // link.
    let root = scratch("microsoft");
    let efi = root.join("EFI");
    std::fs::create_dir_all(efi.join("Microsoft/Boot")).unwrap();
    std::fs::create_dir_all(efi.join("systemd")).unwrap();
    std::fs::create_dir_all(root.join("loader/entries")).unwrap();
    std::fs::write(
        root.join("loader/entries/linux.conf"),
        "title Linux\noptions root=UUID=6c38 rw\n",
    )
    .unwrap();
    let carried = entries(&root);
    let links = |name: &str| {
        carried
            .iter()
            .find(|one| one.name == name)
            .map(|one| one.links.clone())
    };
    assert_eq!(links("Microsoft"), Some(Vec::new()));
    assert_eq!(links("systemd"), Some(vec![Link::Uuid("6c38".to_string())]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_walked_link_resolves_to_the_partition_it_names() {
    let partitions = vec![
        part("/dev/vda1", "vfat"),
        Partition {
            uuid: "8E0615FB".to_string(),
            label: "boot".to_string(),
            ..part("/dev/vda2", "ext4")
        },
        Partition {
            uuid: "37BE8CA7".to_string(),
            ..part("/dev/vda3", "crypto_LUKS")
        },
    ];
    let walked = vec![(
        "/dev/vdb1".to_string(),
        vec![
            Carried {
                name: "fedora".to_string(),
                links: vec![Link::Uuid("8e0615fb".to_string())],
            },
            Carried {
                name: "grublabel".to_string(),
                links: vec![Link::Label("BOOT".to_string())],
            },
            Carried {
                name: "systemd".to_string(),
                links: vec![
                    Link::Uuid("6c38207b".to_string()),
                    Link::Uuid("37be8ca7".to_string()),
                ],
            },
            Carried {
                name: "Microsoft".to_string(),
                links: Vec::new(),
            },
            Carried {
                name: "stale".to_string(),
                links: vec![Link::Uuid("ffff".to_string())],
            },
        ],
    )];
    let linked = esp::linked(walked, &partitions, Some("/dev/sda2"));
    let links: Vec<&str> = linked[0].1.iter().map(|row| row.link.as_str()).collect();
    assert_eq!(
        links,
        [
            "/dev/vda2",
            "/dev/vda2",
            // The root names no partition while its container is closed, and
            // the container the entry names beside it is the one read.
            "/dev/vda3",
            "/dev/sda2",
            "",
        ]
    );
}

#[test]
fn an_entry_gives_way_when_the_plan_takes_the_partition_it_boots() {
    let mount = |partition: &str, fstype: &str| CustomMount {
        partition: partition.to_string(),
        target: "/boot".to_string(),
        fstype: fstype.to_string(),
        passphrase: String::new(),
    };
    let mut layout = CustomLayout::empty("/dev/vda");
    assert!(!layout.entry_replaced("/dev/vda2"));
    layout.mounts.push(mount("/dev/vda2", "unformatted"));
    // A kept filesystem replaces the system the entry boots, and erases
    // nothing.
    assert!(layout.entry_replaced("/dev/vda2"));
    assert!(!layout.formatted("/dev/vda2"));
    assert!(!layout.entry_gone("/dev/vda2"));
    layout.mounts[0].fstype = "ext4".to_string();
    assert!(layout.formatted("/dev/vda2"));
    assert!(layout.entry_gone("/dev/vda2"));
    layout.deletes.push("/dev/vda3".to_string());
    assert!(layout.entry_gone("/dev/vda3"));
    assert!(layout.entry_replaced("/dev/vda3"));
    // An empty link names no system, so no answer touches it.
    assert!(!layout.entry_replaced(""));
    assert!(!layout.entry_gone(""));
}

#[test]
fn the_plan_removes_the_esp_entries_whose_systems_it_rewrites() {
    let scan = Scan {
        unread: Vec::new(),
        disks: vec![DiskScan {
            device: "/dev/vda".to_string(),
            detail: "64 GB".to_string(),
            partitions: vec![part("/dev/vda1", "vfat"), part("/dev/vda2", "ext4")],
            carries: false,
            table: Ok(DiskTable::default()),
            labels: vec![(
                "/dev/vda1".to_string(),
                vec![
                    Label {
                        name: "fedora".to_string(),
                        link: "/dev/vda2".to_string(),
                    },
                    Label {
                        name: "Microsoft".to_string(),
                        link: "/dev/sda2".to_string(),
                    },
                ],
            )],
            keys: Discovered::default(),
        }],
    };
    let mut layout = CustomLayout::empty("/dev/vda");
    layout.mounts.push(CustomMount {
        partition: "/dev/vda1".to_string(),
        target: "/boot/efi".to_string(),
        fstype: "unformatted".to_string(),
        passphrase: String::new(),
    });
    // An assigned ESP keeps its entries until the systems they boot go.
    assert!(esp::removals(&scan, &layout).is_empty());
    layout.mounts.push(CustomMount {
        partition: "/dev/vda2".to_string(),
        target: "/boot".to_string(),
        fstype: "ext4".to_string(),
        passphrase: String::new(),
    });
    assert_eq!(
        esp::removals(&scan, &layout),
        [EspRemoval {
            partition: "/dev/vda1".to_string(),
            entries: vec!["fedora".to_string()],
        }]
    );
    // A disk the plan never takes as its ESP keeps every entry.
    layout.mounts[0].target = String::new();
    assert!(esp::removals(&scan, &layout).is_empty());
}

#[test]
fn removing_an_entry_reaches_one_directory_under_the_esp() {
    let root = scratch("esp-remove");
    let efi = root.join("EFI");
    std::fs::create_dir_all(efi.join("fedora")).unwrap();
    std::fs::write(efi.join("fedora/grub.cfg"), b"search --label boot").unwrap();
    std::fs::create_dir_all(efi.join("keep")).unwrap();
    // A link out of the ESP names the live environment's file, and only the
    // link itself goes.
    std::os::unix::fs::symlink("/etc/hostname", efi.join("escape")).unwrap();
    remove_vendor(&efi, "fedora").expect("the vendor directory");
    assert!(!efi.join("fedora").exists());
    assert!(efi.join("keep").is_dir());
    remove_vendor(&efi, "escape").expect("the link itself");
    assert!(std::fs::symlink_metadata(efi.join("escape")).is_err());
    assert!(Path::new("/etc/hostname").exists());
    // A name that is not one path element never reaches a join.
    assert_eq!(
        remove_vendor(&efi, "../keep").unwrap_err(),
        copy::esp_entry_name("../keep")
    );
    assert!(remove_vendor(&efi, "keep/fedora")
        .unwrap_err()
        .contains("is not one entry name"));
    // An entry gone from the ESP is refused rather than reported as removed.
    assert!(remove_vendor(&efi, "gone")
        .unwrap_err()
        .contains("is not on the ESP"));
    let _ = std::fs::remove_dir_all(&root);
}

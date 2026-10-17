pub(crate) use super::*;
use std::io::Write as _;

mod account;
mod answers;
mod automatic;
mod cli;
mod discover;
mod disks;
mod editor;
mod esp;
mod etcwrite;
mod fdisk;
mod form;
mod layout;
mod lock;
mod panel;
mod payload;
mod recipe;
mod run;
mod screens;
mod table;
mod volumes;

/// This dump was measured on 2026-09-19 with util-linux 2.41.5, and the keys
/// the dump reader skips are trimmed out. The disk is a 200 MiB GPT whose
/// slot 2 was deleted and re-taken by a 10M append. That append landed after
/// the last partition and never in the 20M hole the delete left. A dump
/// reader that drifts from `sfdisk` fails here and never on a disk.
const MEASURED_DUMP: &str = "\
label: gpt
device: /dev/vda
unit: sectors
first-lba: 2048
last-lba: 409566
sector-size: 512

/dev/vda1 : start=        2048, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"one\"
/dev/vda2 : start=      124928, size=       20480, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"small\"
/dev/vda3 : start=       83968, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"three\"
";

/// Root reads a file whatever its mode, so a read failure forced by the mode
/// cannot happen as root.
fn running_as_root() -> bool {
    // SAFETY: geteuid takes no argument and always succeeds.
    unsafe { libc::geteuid() == 0 }
}

/// A file the user can write and not read, which fails a read and not a write.
fn write_only(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o200)).unwrap();
}

/// Makes a `write_only` file readable again and reads it back.
fn readable(path: &Path) -> Vec<u8> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::read(path).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tect-install-{name}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch tree");
    root
}

fn require_sfdisk() {
    let output = Command::new("sfdisk")
        .arg("--version")
        .output()
        .expect("sfdisk is required by the disk tests");
    assert!(output.status.success(), "sfdisk --version failed");
}

/// Holds the image ID `a_payload` records.
const AN_IMAGE_ID: &str = "30ecbe1509090b38ef3260f3609d64d8261230464eaa4d235901db496e9647fb";

/// Holds the payload the form tests ask their questions against. The image,
/// hostname, filesystem and bootloader match `EMITTED`, which describes the
/// same Debian target.
fn a_payload() -> Payload {
    let payload = Payload {
        recipe: "/mnt/tect/install-recipe.json".into(),
        install: an_install_recipe(),
        image: "ghcr.io/tectonic-os/deb2:latest".to_string(),
        image_id: ImageId::parse(AN_IMAGE_ID),
        hostname: "deb2".to_string(),
        filesystem: "ext4".to_string(),
        bootloader: "grub2".to_string(),
        boot: String::new(),
        composefs: false,
        reserve: std::sync::OnceLock::new(),
        luks_initramfs: true,
        esp_entries: std::sync::OnceLock::new(),
    };
    // `image_gb` runs podman, which no test does, so the fixture presets
    // the 10 GB `root_reserve` falls back to for an unmeasurable image.
    payload
        .reserve
        .set(10)
        .expect("a reserve nothing has read yet");
    // The probe into the image is a podman run as well, so the fixture holds
    // the entries a payload writes and a case that needs them sets its own.
    payload
        .esp_entries
        .set(Vec::new())
        .expect("entries nothing has read yet");
    payload
}

fn an_install_recipe() -> InstallRecipe {
    InstallRecipe {
        image: "ghcr.io/tectonic-os/deb2:latest".to_string(),
        target_imgref: "ghcr.io/tectonic-os/deb2:latest".to_string(),
        composefs: false,
        generic: false,
        bootloader: "grub2".to_string(),
        filesystem: "ext4".to_string(),
        hostname: "deb2".to_string(),
        groups: vec!["sudo".to_string()],
        stores: vec!["/var/lib/tectonic/store".to_string()],
        boot: String::new(),
        luks_initramfs: true,
    }
}

/// Holds the payload `a_payload` describes with the ESP entries one image
/// writes, for the table cases that draw what replaces an entry.
fn a_payload_writing(entries: &[&str]) -> Payload {
    let mut payload = a_payload();
    payload.esp_entries = std::sync::OnceLock::new();
    payload
        .esp_entries
        .set(entries.iter().map(|entry| entry.to_string()).collect())
        .expect("entries nothing has read yet");
    payload
}

/// Names one partition the walk read, for the label cases that place a
/// system on a device.
fn part(device: &str, fstype: &str) -> Partition {
    Partition {
        device: device.to_string(),
        fstype: fstype.to_string(),
        ..Default::default()
    }
}

/// Names one system row a partition carries, for the cases that build a scan
/// by hand. The link stays empty, because these cases place no system.
fn label(name: &str) -> Label {
    Label {
        name: name.to_string(),
        link: String::new(),
    }
}

/// Builds the scan a table case draws from the disk rows and partitions it
/// holds. The table and the walk come back empty, because these cases draw
/// the picture alone.
fn scan_of(disks: &[(&str, &str)], parts: &[(&str, Vec<Partition>)]) -> Scan {
    Scan {
        unread: Vec::new(),
        disks: disks
            .iter()
            .map(|(device, detail)| DiskScan {
                device: device.to_string(),
                detail: detail.to_string(),
                partitions: parts
                    .iter()
                    .find(|(at, _)| at == device)
                    .map(|(_, parts)| parts.to_vec())
                    .unwrap_or_default(),
                carries: false,
                table: Ok(DiskTable::default()),
                labels: Vec::new(),
                keys: Discovered::default(),
            })
            .collect(),
    }
}

/// Holds what `emit::recipe::build` emits for a Debian target, which is what
/// the payload root carries at install time.
const EMITTED: &str = r#"{
  "image": "ghcr.io/tectonic-os/deb2:latest",
  "targetImgref": "ghcr.io/tectonic-os/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "hostname": "deb2",
  "user": { "groups": ["sudo"] },
  "additionalImageStores": ["/var/lib/tectonic/store"]
}"#;

/// Builds a one-layer image in a store of its own, with no network. The tar
/// is owned by root, so a rootless store maps its file to the user running
/// the test, who can then alter it.
fn imported_store(name: &str) -> (PathBuf, PathBuf) {
    let root = scratch(name);
    let tree = root.join("tree/bin");
    std::fs::create_dir_all(&tree).expect("a layer tree");
    std::fs::write(tree.join("payload"), "the bytes the image was built with\n")
        .expect("a layer file");
    let tar = root.join("rootfs.tar");
    let packed = Command::new("tar")
        .args(["--owner=0", "--group=0", "-C"])
        .arg(root.join("tree"))
        .arg("-cf")
        .arg(&tar)
        .arg(".")
        .status()
        .expect("tar runs");
    assert!(packed.success(), "tar packs the layer");
    let conf = root.join("storage.conf");
    // Some rootless podman releases ignore `graphroot` and read
    // `rootless_storage_path`, so both name the scratch store.
    std::fs::write(
        &conf,
        format!(
            "[storage]\ndriver = \"vfs\"\ngraphroot = \"{0}/graph\"\n\
             rootless_storage_path = \"{0}/graph\"\nrunroot = \"{0}/run\"\n",
            root.display()
        ),
    )
    .expect("a storage.conf");
    let imported = Command::new("podman")
        .env("CONTAINERS_STORAGE_CONF", &conf)
        .args(["import", "--quiet"])
        .arg(&tar)
        .arg(STORED)
        .output()
        .expect("podman is required by the image check tests");
    assert!(
        imported.status.success(),
        "podman import: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    (root, conf)
}

const STORED: &str = "example.invalid/payload:1";

/// Runs `command` against the store `conf` names.
fn in_store(mut command: Command, conf: &Path) -> std::process::Output {
    command
        .env("CONTAINERS_STORAGE_CONF", conf)
        .output()
        .expect("podman runs")
}

/// Flips one bit of the layer file `imported_store` wrote.
fn alter_layer(root: &Path) {
    let layer_file = find_file(&root.join("graph"), "payload").expect("the stored layer file");
    let mut bytes = std::fs::read(&layer_file).expect("the stored layer file reads");
    bytes[4] ^= 1;
    std::fs::write(&layer_file, bytes).expect("the stored layer file is altered");
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}

/// The store holds files only the user namespace podman runs in can remove.
fn remove_store(root: &Path) {
    let removed = Command::new("podman")
        .args(["unshare", "rm", "-rf"])
        .arg(root)
        .status()
        .expect("podman runs");
    assert!(removed.success(), "the scratch store is removed");
}

/// Holds the answers the install cases run with.
fn answered(kind: &str, passphrase: &str) -> Answers {
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

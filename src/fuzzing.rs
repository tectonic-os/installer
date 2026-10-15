use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn root(name: &str) -> tempfile::TempDir {
    let prefix = format!("installer-fuzz-{name}-");
    let mut builder = tempfile::Builder::new();
    builder
        .prefix(&prefix)
        .permissions(Permissions::from_mode(0o700));
    builder
        .tempdir()
        .unwrap_or_else(|error| panic!("cannot create fuzz input directory: {error}"))
}

fn write(path: &Path, text: &str) {
    let parent = path.parent().expect("fuzz input path has a parent");
    std::fs::create_dir_all(parent).unwrap_or_else(|error| {
        panic!(
            "cannot create fuzz input directory {}: {error}",
            parent.display()
        )
    });
    std::fs::write(path, text)
        .unwrap_or_else(|error| panic!("cannot write fuzz input {}: {error}", path.display()));
}

pub fn recipe(text: &str) {
    let root = root("recipe");
    let path = root.path().join(crate::RECIPE);
    write(&path, text);
    let _ = crate::InstallRecipe::read(&path);
}

pub fn discovery(text: &str) {
    crate::discover::fuzz_readers(text);
}

pub fn disk_headers(text: &str) {
    let _ = crate::slots_from(text);
    let root = root("disks");
    let disk = root.path().join("fuzz");
    write(&disk.join("size"), text);
    write(&disk.join("ro"), text);
    write(&disk.join("removable"), text);
    write(&disk.join("device/model"), text);
    let _ = crate::disks(root.path(), text);
}

pub fn automatic_partitions(text: &str) {
    let _ = crate::partitions_of_type(text, "fuzz");
}

pub fn boot_entries(text: &str) {
    let _ = crate::bootuuid(text);
    let _ = crate::searched(text);
    let _ = crate::bls_links(text);
    let _ = crate::entry_names(text);
}

pub fn payload_labels(text: &str) {
    crate::payload::fuzz_readers(text);
}

pub fn system_config(text: &str) {
    let root = root("config");
    let etc = root.path().join("etc");
    write(&etc.join("crypttab"), text);
    let _ = crate::merge_crypttab(&etc, &[("fuzz".to_string(), "fuzz UUID=fuzz".to_string())]);
    write(&etc.join("fstab"), text);
    let _ = crate::merge_fstab(&etc, "fuzz");
    write(&etc.join("selinux/config"), text);
    let _ = crate::target_contexts(root.path());
}

pub fn accounts(text: &str) {
    let root = root("accounts");
    write(&root.path().join("group"), text);
    let requested = ["wheel".to_string(), "video".to_string(), text.to_string()];
    let _ = crate::present_groups(root.path(), &requested);
    let _ = crate::portable_name(text);
}

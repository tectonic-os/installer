use super::*;

/// Names the document a payload root carries. `emit::recipe::build` writes it
/// and `tect vm build iso` bakes it onto installer media.
pub const RECIPE: &str = "install-recipe.json";

/// Returns the root `--from` defaults to, which is the directory holding the
/// build's own documents. Installer media carries its recipe there, beside the
/// manifest.
pub fn media() -> PathBuf {
    Path::new(crate::BUILD_MANIFEST)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf()
}

/// Holds a recipe and the fields the installer shows the user before they
/// agree to erase a disk.
#[derive(Debug)]
pub struct Payload {
    pub recipe: PathBuf,
    pub(crate) install: InstallRecipe,
    /// Names the reference bootc installs. On media this is the published
    /// name the local bytes are embedded under.
    pub image: String,
    /// Names the installed machine. This is the one derived value the
    /// installer expects the user to replace.
    pub hostname: String,
    /// Names the root filesystem. The base family settles it and no question
    /// offers it, because a composefs-sealed deployment needs fs-verity and
    /// xfs has none. The screen shows it so the user about to erase a disk
    /// can read it.
    pub filesystem: String,
    /// Names the bootloader, `grub2` or `systemd`. It decides whether the
    /// layout carries a separate `/boot`. An empty value reads as `grub2`.
    pub bootloader: String,
    /// Names the declared UKI trust chain. An empty value takes the existing
    /// boot path.
    pub boot: String,
    /// States whether the image installs through the composefs backend. That
    /// backend seals the deployment and needs fs-verity on the root, so a
    /// manual root formatted xfs or ext3 cannot carry it.
    pub composefs: bool,
    /// The installer probes the image the first time a caller asks. The
    /// figure is twice the
    /// image bytes, which covers the copy that installs and the deployment
    /// staged beside it, plus 2 GB of slack. An image it cannot measure
    /// reserves 10 GB.
    pub(crate) reserve: std::sync::OnceLock<u64>,
    /// States that the built image proved its initramfs carries a LUKS
    /// userspace driver. An old recipe and an image that made no such claim
    /// both read false.
    pub luks_initramfs: bool,
    /// Holds the entry directory names the image's staged EFI payloads carry.
    /// The form reads them once; the first draw that needs them probes the
    /// image, because no recipe field names them.
    pub(crate) esp_entries: std::sync::OnceLock<Vec<String>>,
}

/// Lists the EFI directories the staged boot payloads carry, under bootupd's
/// update source and under `/boot` where a signed UKI lives. A pattern that
/// matches nothing stays literal, and the directory test drops it. The `if`
/// keeps a glob that matched nothing from failing the whole listing. The two
/// optional arguments let a test give the roots it made itself.
pub(crate) const ESP_LIST: &str = "for d in \"${1:-/usr/lib/efi}\"/*/*/EFI/*/ \"${2:-/boot/EFI}\"/*/; do if [ -d \"$d\" ]; then printf '%s\\n' \"${d%/}\"; fi; done";

impl Payload {
    /// Lists the entry directories the image's staged EFI payloads carry. The
    /// install writes them into the ESP, so the picture shows what replaces
    /// the entries it removes. An image the probe cannot read gives an empty
    /// list, and the picture keeps the entries the walk found.
    pub(crate) fn esp_entries(&self) -> &[String] {
        self.esp_entries.get_or_init(|| {
            let out = Command::new("podman")
                .args([
                    "run",
                    "--rm",
                    "--pull=never",
                    "--net=none",
                    "--security-opt",
                    "label=disable",
                    "--entrypoint",
                    "",
                ])
                .arg(&self.image)
                .args(["/bin/sh", "-c", ESP_LIST])
                .output();
            match out {
                Ok(out) if out.status.success() => {
                    esp::entry_names(&String::from_utf8_lossy(&out.stdout))
                }
                _ => Vec::new(),
            }
        })
    }

    /// The first call probes the image. Every draw of the form after that is
    /// free.
    pub fn reserve_gb(&self) -> u64 {
        *self
            .reserve
            .get_or_init(|| root_reserve(&image_gb(&self.image)))
    }
}

/// Reserves twice the image plus 2 GB. A bootc install needs room for the
/// installed copy and the staged deployment beside it.
pub(crate) fn root_reserve(image_gb: &Option<u64>) -> u64 {
    image_gb.map_or(10, |image| 2 * image + 2)
}

/// Reads the image's size from the local store, in whole decimal GB the way
/// the screens say it. An image that is absent, or a podman that cannot
/// answer, returns `None`.
fn image_gb(image: &str) -> Option<u64> {
    let out = Command::new("podman")
        .args(["image", "inspect", "--format", "{{.Size}}"])
        .arg(image)
        .output()
        .ok()?;
    match out.status.success() {
        true => String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok()
            .map(|bytes| bytes.div_ceil(1_000_000_000)),
        false => None,
    }
}

/// Names which of the three cases a root is.
#[derive(Debug)]
pub enum Found {
    /// Carries built bytes and the recipe for them. This installs, and needs
    /// no network.
    Image(Payload),
    /// Carries a repository, which is the source. The user must build it
    /// before the installer can write an image from it.
    Repo(PathBuf),
    /// Carries neither. It keeps the root so the refusal can name where the
    /// installer looked, which matters most on the default root the user
    /// never typed.
    Nothing(PathBuf),
}

impl Found {
    /// Returns the payload, or states why this root has none. The installer
    /// asks before it asks the user for a disk to erase, because a
    /// precondition that can be checked early and is checked late is a bug in
    /// an installer.
    pub fn payload(&self) -> Result<&Payload, String> {
        match self {
            Self::Image(payload) => Ok(payload),
            // The installer never builds here. The scratch would have to go
            // on the target disk, which means partitioning before the build
            // and an erased disk when the build fails.
            Self::Repo(root) => Err(format!(
                "{} is a repository, so there is nothing here to install yet\n\nhelp: \
                 `tect build` in it, then `{PROGRAM} --from <the built payload>`",
                root.display()
            )),
            Self::Nothing(root) => Err(format!(
                "{} holds no {RECIPE} and no {}, so it is neither a payload nor a repository",
                root.display(),
                crate::REPO_FILE
            )),
        }
    }
}

/// Classifies a root. A malformed recipe refuses, naming the file. Falling
/// through to the next case would read a stick that carries a payload it
/// cannot install as a stick that carries nothing.
pub fn classify(root: &Path) -> Result<Found, String> {
    let recipe = root.join(RECIPE);
    if recipe.is_file() {
        let install = InstallRecipe::read(&recipe)?;
        return Ok(Found::Image(Payload {
            image: install.image.clone(),
            hostname: install.hostname.clone(),
            filesystem: install.filesystem.clone(),
            bootloader: install.bootloader.clone(),
            boot: install.boot.clone(),
            composefs: install.composefs,
            reserve: std::sync::OnceLock::new(),
            luks_initramfs: install.luks_initramfs,
            esp_entries: std::sync::OnceLock::new(),
            recipe,
            install,
        }));
    }
    Ok(match root.join(crate::REPO_FILE).is_file() {
        true => Found::Repo(root.to_path_buf()),
        false => Found::Nothing(root.to_path_buf()),
    })
}

/// Labels a payload partition. This label is the whole rule for finding a
/// root the user never named.
pub const LABEL: &str = "TECT";

/// Mounts a labelled partition under `/run`. A live environment holds `/run`
/// in RAM, so reading a payload writes to no disk.
pub(crate) const MOUNTPOINT: &str = "/run/tect-payload";

/// Finds the root to classify when no `--from` named one. A `TECT` partition
/// overrides the media's own payload. Two of them refuse, naming both,
/// because picking wrong erases a disk from the wrong image.
pub fn root() -> Result<PathBuf, String> {
    let listed = Command::new("blkid")
        .args(["-t", &format!("LABEL={LABEL}"), "-o", "device"])
        .output();
    let devices = match &listed {
        Ok(out) => labelled(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    };
    match devices.as_slice() {
        [] => Ok(media()),
        [device] => mounted(device),
        many => Err(format!(
            "{} partitions are labelled {LABEL}, so which one to install from is not \
             clear: {}\n\nhelp: `{PROGRAM} --from <root>` names one outright",
            many.len(),
            many.join(", ")
        )),
    }
}

pub(crate) fn labelled(listed: &str) -> Vec<String> {
    listed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Mounts the payload partition read-only and leaves it mounted. `open_log`
/// later remounts the partition to write the install log beside the recipe.
/// The mount lasts until this environment reboots.
pub(crate) fn mounted(device: &str) -> Result<PathBuf, String> {
    let at = PathBuf::from(MOUNTPOINT);
    // A second run finds the mount the first run left and carries on over it.
    if at.join(RECIPE).is_file() {
        return Ok(at);
    }
    std::fs::create_dir_all(&at).map_err(|err| format!("{MOUNTPOINT}: {err}"))?;
    let out = Command::new("mount")
        .args(["-o", "ro", device, MOUNTPOINT])
        .output()
        .map_err(|err| format!("mount: {err}, and it is what reads a {LABEL} partition"))?;
    match out.status.success() {
        true => Ok(at),
        false => Err(format!(
            "mounting {device} at {MOUNTPOINT}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

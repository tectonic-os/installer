use super::*;
use std::io::Write as _;

/// Names every recipe field this installer accepts; the installer refuses a
/// field outside this list before it changes the disk.
#[derive(Debug)]
pub(crate) struct InstallRecipe {
    pub(crate) image: String,
    pub(crate) target_imgref: String,
    pub(crate) composefs: bool,
    pub(crate) generic: bool,
    pub(crate) bootloader: String,
    pub(crate) filesystem: String,
    pub(crate) hostname: String,
    pub(crate) groups: Vec<String>,
    pub(crate) stores: Vec<String>,
    pub(crate) boot: String,
    pub(crate) luks_initramfs: bool,
}

impl InstallRecipe {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let raw =
            std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let doc = Json::parse(&raw).map_err(|err| format!("{}: {err}", path.display()))?;
        let Json::Object(fields) = &doc else {
            return Err(format!("{}: not an object", path.display()));
        };
        const KNOWN: &[&str] = &[
            "image",
            "targetImgref",
            "composeFsBackend",
            "genericImage",
            "bootloader",
            "filesystem",
            "hostname",
            "user",
            "additionalImageStores",
            "boot",
            "luksInitramfs",
        ];
        for (at, (name, _)) in fields.iter().enumerate() {
            if !KNOWN.contains(&name.as_str()) {
                return Err(format!(
                    "{}: field `{name}` is not implemented",
                    path.display()
                ));
            }
            if fields[..at].iter().any(|(held, _)| held == name) {
                return Err(format!("{}: field `{name}` occurs twice", path.display()));
            }
        }

        let image = required_text(&doc, path, "image")?;
        let target_imgref = match optional_text(&doc, path, "targetImgref")? {
            Some(value) if value.is_empty() => {
                return Err(format!("{}: no `targetImgref`", path.display()))
            }
            Some(value) => value,
            None => image.clone(),
        };
        // A default here would cut a `/boot` for a systemd-boot image and
        // leave its root untyped, so the recipe names the bootloader.
        let bootloader = required_text(&doc, path, "bootloader")?;
        if !matches!(bootloader.as_str(), "grub2" | "systemd") {
            return Err(format!(
                "{}: {}",
                path.display(),
                copy::unknown_bootloader(&bootloader)
            ));
        }
        let groups = match json::field(&doc, "user") {
            None => Vec::new(),
            Some(Json::Object(user)) => {
                for (at, (name, _)) in user.iter().enumerate() {
                    if name != "groups" {
                        return Err(format!(
                            "{}: field `user.{name}` is not implemented",
                            path.display()
                        ));
                    }
                    if user[..at].iter().any(|(held, _)| held == name) {
                        return Err(format!(
                            "{}: field `user.{name}` occurs twice",
                            path.display()
                        ));
                    }
                }
                strings(&doc, path, "user", "groups")?
            }
            Some(_) => return Err(format!("{}: field `user` is not an object", path.display())),
        };
        let stores = strings(&doc, path, "", "additionalImageStores")?;
        if stores.is_empty() {
            return Err(format!(
                "{}: no `additionalImageStores`, and every install reads the media store",
                path.display()
            ));
        }
        let composefs = optional_bool(&doc, path, "composeFsBackend")?.unwrap_or(false);
        let filesystem = required_text(&doc, path, "filesystem")?;
        // The installer formats every root this recipe installs with its
        // `filesystem`. An opened root escapes the editor's rule, so the pair
        // is refused here.
        if composefs && !["ext4", "btrfs"].contains(&filesystem.as_str()) {
            return Err(format!(
                "{}: `composeFsBackend` needs fs-verity, which `filesystem` {filesystem:?} has not got",
                path.display()
            ));
        }
        Ok(Self {
            image,
            target_imgref,
            composefs,
            generic: optional_bool(&doc, path, "genericImage")?.unwrap_or(false),
            bootloader,
            filesystem,
            hostname: required_text(&doc, path, "hostname")?,
            groups,
            stores,
            boot: optional_text(&doc, path, "boot")?.unwrap_or_default(),
            luks_initramfs: optional_bool(&doc, path, "luksInitramfs")?.unwrap_or(false),
        })
    }
}

/// Returns a crypt string without placing the password in the process list.
/// The installer writes the same string into the installed account's shadow
/// file.
pub(crate) fn hashed(password: &str) -> Result<String, String> {
    let mut child = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("openssl: {err}, and it is what hashes the password"))?;
    child
        .stdin
        .take()
        .ok_or("openssl: no stdin")?
        .write_all(format!("{password}\n").as_bytes())
        .map_err(|err| format!("openssl: {err}"))?;
    let out = child
        .wait_with_output()
        .map_err(|err| format!("openssl: {err}"))?;
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.success() && hash.starts_with('$') {
        true => Ok(hash),
        false => Err(format!(
            "openssl passwd wrote no crypt string: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

fn required_text(doc: &Json, path: &Path, key: &str) -> Result<String, String> {
    optional_text(doc, path, key)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{}: no `{key}`", path.display()))
}

fn optional_text(doc: &Json, path: &Path, key: &str) -> Result<Option<String>, String> {
    match json::field(doc, key) {
        None => Ok(None),
        Some(Json::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{}: field `{key}` is not a string", path.display())),
    }
}

fn optional_bool(doc: &Json, path: &Path, key: &str) -> Result<Option<bool>, String> {
    match json::field(doc, key) {
        None => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!(
            "{}: field `{key}` is not true or false",
            path.display()
        )),
    }
}

fn strings(doc: &Json, path: &Path, parent: &str, key: &str) -> Result<Vec<String>, String> {
    let value = match parent {
        "" => json::field(doc, key),
        _ => json::field(doc, parent).and_then(|value| json::field(value, key)),
    };
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Json::Array(items) = value else {
        return Err(format!("{}: field `{key}` is not a list", path.display()));
    };
    items
        .iter()
        .map(|item| match item {
            Json::String(value) => Ok(value.clone()),
            _ => Err(format!(
                "{}: field `{key}` contains a value that is not a string",
                path.display()
            )),
        })
        .collect()
}

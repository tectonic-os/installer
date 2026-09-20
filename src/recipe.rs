use super::*;
use std::io::Write as _;

/// Replaces the value under `key`, or appends it. A value that is not an
/// object is left alone, so a recipe whose `user` is a string fails in
/// fisherman's own reader.
pub(crate) fn set(value: &mut Json, key: &str, field: Json) {
    let Json::Object(fields) = value else { return };
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some((_, held)) => *held = field,
        None => fields.push((key.to_string(), field)),
    }
}

pub(crate) fn unset(value: &mut Json, key: &str) {
    let Json::Object(fields) = value else { return };
    fields.retain(|(name, _)| name != key);
}

/// Returns the recipe with the user's half in it. The installer merges `user`
/// rather than replace it. The groups already there name the target's admin
/// group, and `useradd` refuses the whole call when it names a group the
/// target has not got.
pub fn complete(recipe: &Path, answers: &Answers) -> Result<Json, String> {
    let raw =
        std::fs::read_to_string(recipe).map_err(|err| format!("{}: {err}", recipe.display()))?;
    let mut doc = Json::parse(&raw).map_err(|err| format!("{}: {err}", recipe.display()))?;
    set(&mut doc, "disk", Json::string(&answers.disk));
    set(&mut doc, "hostname", Json::string(&answers.hostname));
    set(
        &mut doc,
        "encryption",
        Json::object([
            ("type", Json::string(&answers.encryption.kind)),
            ("passphrase", Json::string(&answers.encryption.passphrase)),
            ("pin", Json::string(&answers.encryption.pin)),
        ]),
    );
    let mut user = match doc {
        Json::Object(ref mut fields) => match fields.iter().position(|(name, _)| name == "user") {
            Some(at) => fields.remove(at).1,
            None => Json::object([]),
        },
        _ => return Err(format!("{}: not an object", recipe.display())),
    };
    set(&mut user, "username", Json::string(&answers.user));
    set(
        &mut user,
        "password",
        Json::string(hashed(&answers.password)?),
    );
    set(&mut doc, "user", user);
    if let Some(layout) = &answers.layout {
        unset(&mut doc, "varDisk");
        let mut mounts: Vec<Json> = layout
            .mounts
            .iter()
            .map(|mount| {
                Json::object([
                    ("partition", Json::string(&mount.partition)),
                    // Fisherman knows swap by the target `swap` rather than
                    // by a mount point. It mounts every other target as a
                    // path.
                    (
                        "target",
                        Json::string(match mount.target.as_str() {
                            "/swap" => "swap",
                            target => target,
                        }),
                    ),
                    ("fstype", Json::string(&mount.fstype)),
                ])
            })
            .collect();
        // A created partition is a plain mount by the time fisherman sees
        // it. `cut_partitions` cut the partition and wrote the node it got, so
        // the recipe carries a device that exists and a filesystem for it.
        for create in &layout.creates {
            mounts.push(Json::object([
                ("partition", Json::string(&create.device)),
                (
                    "target",
                    Json::string(match create.target.as_str() {
                        "/swap" => "swap",
                        target => target,
                    }),
                ),
                ("fstype", Json::string(&create.fstype)),
            ]));
        }
        // An opened container is a mapper by the time fisherman sees it. The
        // container device and the key stay in this process, because fisherman
        // decodes neither.
        for (name, open) in layout.mappers() {
            mounts.push(Json::object([
                ("partition", Json::string(&mapper_path(&name))),
                ("target", Json::string(&open.target)),
                ("fstype", Json::string("unformatted")),
            ]));
        }
        set(&mut doc, "customMounts", Json::array(mounts));
        return Ok(doc);
    }
    // An answer of none writes nothing, so a `var-disk` the image declared is
    // left exactly as `emit::recipe` wrote it.
    let encrypted = answers.encryption.kind != NONE;
    if !answers.data.size.is_empty() {
        // A separate home is a `/var` cut out of the install disk, which is
        // what `size` without a `disk` means to fisherman. The screen holds
        // the number and draws the unit, and sfdisk is handed both.
        let size = match size_gb(&answers.data.size) {
            Some(size) => format!("{size}GB"),
            None => answers.data.size.clone(),
        };
        let mut var = Json::object([("size", Json::string(&size))]);
        if encrypted {
            set(&mut var, "encrypt", Json::Bool(true));
        }
        set(&mut doc, "varDisk", var);
    }
    Ok(doc)
}

/// Returns a `$`-prefixed crypt string. Fisherman hands the field to
/// `chpasswd`, and only a `$` takes its `-e` branch. A plaintext password goes
/// through PAM and dies `pam_chauthtok() failed, error: Module is unknown`
/// after the OS is already on the disk. This calls `openssl passwd` because
/// crypt(3) lives in libcrypt, which this binary does not link. `-stdin` keeps
/// the password out of `ps`.
pub(crate) fn hashed(password: &str) -> Result<String, String> {
    let mut child = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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
            "openssl passwd wrote no crypt string, and a plaintext password loses the install at \
             its last step: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Holds the staged recipe file and removes it when this guard drops. The
/// recipe carries the password hash and the plaintext LUKS passphrase, so
/// every way out of `run` removes it. The three backend spawn failures return
/// before fisherman starts, and the next edit forgets an explicit removal
/// there.
pub(crate) struct Staged(pub(crate) PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes the completed recipe to a staged file. The recipe carries a
/// password hash, so the file is created 0600 and no later call widens it. On
/// installer media `TMPDIR` is `/tmp`, which is RAM and never reaches the
/// disk.
pub(crate) fn stage(recipe: &Json) -> Result<Staged, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = std::env::temp_dir().join(format!("tect-install.{}.json", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    // The guard takes the path before the first write, because a write that
    // fails part-way still leaves the passphrase in the file it opened. A full
    // tmpfs is how that happens on installer media.
    let staged = Staged(path);
    file.write_all(recipe.render().as_bytes())
        .map_err(|err| format!("{}: {err}", staged.0.display()))?;
    Ok(staged)
}

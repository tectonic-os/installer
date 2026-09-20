use super::*;
use std::io::Write as _;

/// Names the file fisherman's whole output is written to.
const LOG: &str = "tect-install.log";

/// Holds the log where no partition can. An iso-only boot has RAM and
/// nothing else.
const IN_RAM: &str = "/run";

/// Opens the install log beside the payload, where that partition can be
/// remounted writable. `root()` mounts the partition read-only, and an image's
/// own `/usr/share/tectonic` never remounts. Every iso-only boot falls back to
/// RAM. The installer prints which of the two it used.
pub(crate) fn open_log(payload: &Payload) -> (Option<std::fs::File>, Option<PathBuf>) {
    if payload.recipe.parent() == Some(Path::new(MOUNTPOINT)) && remounted_rw(MOUNTPOINT) {
        let path = Path::new(MOUNTPOINT).join(LOG);
        if let Ok(file) = std::fs::File::create(&path) {
            return (Some(file), Some(path));
        }
    }
    let path = Path::new(IN_RAM).join(LOG);
    match std::fs::File::create(&path) {
        Ok(file) => (Some(file), Some(path)),
        // A live environment holds nothing writable at all. The screen is
        // then the only copy of the transcript, and the installer says so.
        Err(_) => (None, None),
    }
}

/// Remounts the payload partition writable. The installer mounts that
/// partition read-only, so a log beside the payload needs a deliberate
/// remount.
fn remounted_rw(at: &str) -> bool {
    Command::new("mount")
        .args(["-o", "remount,rw", at])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Names the device an opened container appears at, which is what fisherman
/// mounts.
pub(crate) fn mapper_path(name: &str) -> String {
    format!("/dev/mapper/{name}")
}

/// Holds the mapper names an install has open. Dropping this closes every one
/// of them, on every way out of `run`. A failed fisherman, a key that does not
/// fit and a finished install all reach the same drop.
#[derive(Default)]
pub(crate) struct Mappers(Vec<String>);

impl Drop for Mappers {
    fn drop(&mut self) {
        for name in &self.0 {
            if let Err(why) = close_volume(name) {
                eprintln!("{PROGRAM}: {why}");
            }
        }
    }
}

/// Opens every container the layout names, before anything is written. A
/// container that does not open stops the install here, with the partitions
/// untouched.
pub(crate) fn open_volumes(layout: Option<&CustomLayout>) -> Result<Mappers, String> {
    let mut opened = Mappers::default();
    let Some(layout) = layout else {
        return Ok(opened);
    };
    for (name, open) in layout.mappers() {
        open_volume(open, &name)?;
        opened.0.push(name);
    }
    Ok(opened)
}

/// Builds `cryptsetup`'s call for one container. A passphrase and a data
/// volume's key both arrive on stdin, so neither reaches the process list. A
/// key file is named by its path, which `cryptsetup` reads itself.
pub(crate) fn open_command(open: &LuksOpen, name: &str) -> Command {
    let mut command = Command::new("cryptsetup");
    command.args(["-q", "luksOpen"]);
    match &open.key {
        Key::Passphrase(_) | Key::Data(_) => {
            command.args(["--key-file", "-"]);
        }
        Key::File(path) => {
            command.args(["--key-file"]).arg(path);
        }
    }
    command.arg(&open.partition).arg(name);
    command
}

pub(crate) fn open_volume(open: &LuksOpen, name: &str) -> Result<(), String> {
    // A mapper left behind by a killed install fails `luksOpen` on a retry in
    // the same boot. Fisherman clears its own mappers the same way.
    if Path::new(&mapper_path(name)).exists() {
        let _ = close_volume(name);
    }
    let mut command = open_command(open, name);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    if open.key.bytes().is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("cryptsetup: {err}, and it is what opens {}", open.partition))?;
    if let Some(bytes) = open.key.bytes() {
        child
            .stdin
            .take()
            .ok_or("cryptsetup: no stdin")?
            .write_all(bytes)
            .map_err(|err| format!("cryptsetup: {err}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|err| format!("cryptsetup: {err}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "cryptsetup could not open {}: {}",
            open.partition,
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub(crate) fn close_volume(name: &str) -> Result<(), String> {
    let out = Command::new("cryptsetup")
        .args(["-q", "close", name])
        .output()
        .map_err(|err| format!("cryptsetup: {err}, and it is what closes a container"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{} is still open: {}",
            mapper_path(name),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

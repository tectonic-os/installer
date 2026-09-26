use super::*;
use std::io::{BufRead as _, Write as _};

const TICK: std::time::Duration = std::time::Duration::from_millis(120);
const STORAGE_CONF: &str = "/etc/containers/storage.conf";

/// Prepares the disk and runs the payload's own bootc over it, drawing bootc's
/// output into a bounded region and writing all of it to a file.
///
/// A failed draw never fails the install, so no call here uses `?` on the
/// region.
pub fn run(payload: &Payload, answers: &mut Answers, prompt: &Prompt) -> Result<(), String> {
    require_signed_boot_chain(&payload.image, &payload.boot)?;
    // A TPM2 answer stages an enrolment the image performs on its own first
    // boot. This is the last moment before a write that can refuse an image
    // unable to perform it.
    if answers.opened == Opened::Tpm2 || answers.encryption.kind.starts_with("tpm2-") {
        require_tpm2_enrolment(&payload.image)?;
    }
    validate_recipe(payload)?;
    if !portable_name(&answers.user) {
        return Err(copy::account_name(&answers.user));
    }
    let password_hash = hashed(&answers.password)?;
    // The disk stays cut, open and mounted until `prepared` drops, on every
    // way out of this function, the panic path included.
    let mut prepared = prepare(payload, answers)?;
    let changes_table = prepared.layout.changes_table();
    installed(payload, answers, prompt, &password_hash, &mut prepared).map_err(|why| {
        match changes_table {
            true => format!("{why}\n\n{}", copy::table_already_changed(&answers.disk)),
            false => why,
        }
    })
}

/// Runs bootc over the prepared disk and writes the installed system. Every
/// failure here comes after the cut, which `run` tells the user.
fn installed(
    payload: &Payload,
    answers: &mut Answers,
    prompt: &Prompt,
    password_hash: &str,
    prepared: &mut Prepared,
) -> Result<(), String> {
    let karg = root_luks_arg(&prepared.layout, &payload.boot)?;
    let mut command = bootc_command(&payload.install, karg.as_deref());
    let (mut log, at) = open_log(payload);
    // Printed only where no widget draws. On the installer's own screen this
    // line would land above the box and stay there, because a bounded region
    // redraws itself and never the row over it.
    if !prompt.draws() {
        eprintln!(
            "{PROGRAM}: installing {} as {} onto {}, {}",
            payload.image,
            answers.hostname,
            answers.disk,
            copy::logging(at.as_deref())
        );
    }
    // One pipe carries both streams, so bootc's stderr arrives with stdout.
    // An inherited stderr would print straight onto the drawn region.
    let (events, writer) = std::io::pipe().map_err(|err| format!("podman: {err}"))?;
    let errors = writer.try_clone().map_err(|err| format!("podman: {err}"))?;
    let mut child = command
        .stdout(Stdio::from(writer))
        .stderr(Stdio::from(errors))
        .spawn()
        .map_err(|err| format!("podman: {err}, and it is what runs bootc"))?;
    // `Command` keeps its configured descriptors after `spawn`, so retaining
    // it would keep the read loop open after podman exits.
    drop(command);
    let mut region = match prompt.draws() {
        true => common::ui::Progress::open(&copy::writing(at.as_deref())).ok(),
        false => None,
    };
    if let Some(region) = &mut region {
        let _ = region.step(0, 100, "Installing the operating system");
    }
    {
        // A thread reads the pipe and this takes lines with a timeout, so the
        // region keeps drawing while bootc is silent. A step can hold the
        // machine for minutes between two messages. Both write ends moved into
        // the child, so the read ends when the child ends, and this loop with
        // it.
        let (lines, arriving) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(events)
                .lines()
                .map_while(Result::ok)
            {
                if lines.send(line).is_err() {
                    return;
                }
            }
        });
        loop {
            let line = match arriving.recv_timeout(TICK) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(region) = &mut region {
                        let _ = region.tick();
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if let Some(log) = &mut log {
                let _ = writeln!(log, "{line}");
            }
            match &mut region {
                None => println!("{line}"),
                Some(region) => {
                    let _ = region.note(&line);
                }
            }
        }
    }
    let finished = child.wait().map_err(|err| format!("podman: {err}"));
    if let Some(mut region) = region {
        if finished.as_ref().is_ok_and(|status| status.success()) {
            let _ = region.step(100, 0, "Operating system installed");
        }
        region.close();
    }
    let status = finished?;
    if !status.success() {
        return Err(format!(
            "bootc did not finish: {status}, and {}",
            copy::logging(at.as_deref())
        ));
    }
    // The containers the layout opened must open again on the installed
    // machine, and no other step arranges that on the path the user chose.
    let notes = arrange(payload, answers, &prepared.layout, password_hash)?;
    configure_boot_chain(&payload.image, &answers.disk, &payload.boot)?;
    render_menu(&payload.image, &answers.disk)?;
    // The steps the last screen asks for are read from the firmware now
    // rather than from the form's panel. A key enrolled while the install ran
    // changes them.
    let steps = next_steps(&payload.boot, &firmware(payload));
    let recovery = prepared.recovery.take();
    let volumes = match recovery.is_some() {
        true => recovery_volumes(answers),
        false => Vec::new(),
    };
    let offered = match prompt.draws() {
        true => automatic::offered(payload, answers, recovery.as_deref()),
        // A run that draws nothing cannot offer the action, and the probe
        // into the image would cost an answer no one reads.
        false => Offered::None,
    };
    finish(
        recovery.as_deref(),
        at.as_deref(),
        steps,
        &notes,
        &volumes,
        offered,
        prompt,
    )
}

/// Refuses missing host paths, and an image the stores lack, before the
/// installer changes the partition table. The install runs with `--pull=never`,
/// so a missing image would otherwise stop it after the cut.
fn validate_recipe(payload: &Payload) -> Result<(), String> {
    if !Path::new(STORAGE_CONF).is_file() {
        return Err(format!(
            "{STORAGE_CONF}: missing, and bootc needs it to read the media store"
        ));
    }
    for store in &payload.install.stores {
        if !store.starts_with('/') || store.contains(':') || !Path::new(store).is_dir() {
            return Err(format!(
                "{}: `additionalImageStores` names unusable path {store:?}",
                payload.recipe.display()
            ));
        }
    }
    // The output is captured, because an inherited stderr would print onto
    // the drawn region.
    let held = Command::new("podman")
        .args(["image", "exists", &payload.install.image])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what reads the media store"))?;
    match held.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(format!(
            "no store {STORAGE_CONF} names holds {}",
            payload.install.image
        )),
        _ => Err(format!(
            "podman could not read the stores {STORAGE_CONF} names: {}",
            String::from_utf8_lossy(&held.stderr).trim()
        )),
    }
}

pub(crate) fn bootc_command(recipe: &InstallRecipe, karg: Option<&str>) -> Command {
    let mut command = Command::new("podman");
    command.args([
        "run",
        "--rm",
        "--pull=never",
        "--privileged",
        "--pid=host",
        "--security-opt",
        "label=disable",
        "-v",
        "/dev:/dev",
        "-v",
        "/var/lib/containers:/var/lib/containers",
    ]);
    for store in &recipe.stores {
        command.args(["-v", &format!("{store}:{store}:ro")]);
    }
    command.args([
        "-v",
        &format!("{STORAGE_CONF}:{STORAGE_CONF}:ro"),
        "-v",
        &format!("{SYSROOT}:/target"),
        &recipe.image,
        "bootc",
        "install",
        "to-filesystem",
        "--skip-finalize",
        "--target-imgref",
        &recipe.target_imgref,
    ]);
    if recipe.composefs {
        command.arg("--composefs-backend");
    }
    if !recipe.bootloader.is_empty() && recipe.bootloader != "grub2" {
        command.args(["--bootloader", &recipe.bootloader]);
    }
    if recipe.generic {
        command.arg("--generic-image");
    }
    if let Some(karg) = karg {
        command.args(["--karg", karg]);
    }
    command.arg("/target");
    command
}

/// Gives bootc the root container before it writes the BLS entry. A signed UKI
/// finds its root by partition type and carries no mutable kernel command line.
fn root_luks_arg(layout: &CustomLayout, boot: &str) -> Result<Option<String>, String> {
    if !boot.is_empty() {
        return Ok(None);
    }
    let Some((name, open)) = layout
        .mappers()
        .into_iter()
        .find(|(_, open)| open.target == "/")
    else {
        return Ok(None);
    };
    Ok(Some(format!(
        "rd.luks.name={}={name}",
        luks_uuid(&open.partition)?
    )))
}

/// Lists what a recovery key opens, so a photograph of the screen names the
/// disk. The list carries the install disk and every partition the layout
/// chose to open. `Prepared` holds an automatic layout, so on that path the
/// answers name only the disk.
fn recovery_volumes(answers: &Answers) -> Vec<String> {
    let mut volumes = vec![format!("{}: {}", copy::ENCRYPTED_DISK, answers.disk)];
    if let Some(layout) = &answers.layout {
        for open in &layout.opens {
            volumes.push(format!("{}: {}", copy::ENCRYPTED_PARTITION, open.partition));
        }
    }
    volumes
}

/// Draws the last screen. It carries the recovery key, which is on screen
/// because it is deliberately in no file. It carries the steps the firmware
/// still asks for. It offers the restart, because the stick is still in the
/// machine and no other screen says what to do next. Where a first-boot
/// enrolment is staged, it offers the automatic finalize beside the manual
/// restart, and the automatic action writes the credential before restarting.
/// `notes` states what arranging the opened containers owes about slots,
/// which this says rather than does. `volumes` names what the key opens.
pub(crate) fn finish(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
    offered: Offered,
    prompt: &Prompt,
) -> Result<(), String> {
    // No widget draws, so the streams are the only channel left.
    if !prompt.draws() {
        if let Some(key) = recovery {
            println!("\n{}", copy::recovery(key));
            for volume in volumes {
                println!("{volume}");
            }
        }
        if let Some(steps) = steps {
            println!("{}", copy::NEXT_STEPS_HEADING);
            println!("{steps}\n");
        }
        for note in notes {
            println!("{note}");
        }
        eprintln!("{PROGRAM}: {}", copy::logging(log));
        return Ok(());
    }
    // The whole key goes inside the box. A key held in no file and shown on
    // no screen leaves a disk the user cannot open.
    let mut offered = offered;
    choosing(
        &mut offered,
        |offered| {
            let rows = done_rows(recovery, log, steps, notes, volumes, offered);
            let mut actions = Vec::new();
            match offered {
                Offered::Ready(_) => actions.push(Choice::new(copy::FINALIZE_AUTO, "")),
                Offered::Unavailable(why) => {
                    actions.push(Choice::new(copy::FINALIZE_AUTO, why.clone()).unavailable())
                }
                Offered::None => actions.push(Choice::new(copy::RESTART, "")),
            }
            if !matches!(offered, Offered::None) {
                actions.push(Choice::new(copy::FINALIZE_MANUAL, ""));
            }
            common::ui::offer_over(copy::INSTALL_DONE, rows, &actions, copy::DONE_KEYS)
        },
        automatic::finalize,
        restart,
    )
}

/// Runs the completion screen until an action ends it. A failed automatic
/// finalize draws the screen again with the reason, because the recovery key
/// the screen shows is in no file and a restart would leave the disk
/// unopenable.
pub(crate) fn choosing(
    offered: &mut Offered,
    mut draw: impl FnMut(&Offered) -> Result<Option<usize>, String>,
    mut finalize: impl FnMut(&Ready) -> Result<(), String>,
    mut restart: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let Some(at) = draw(offered)? else {
            return Ok(());
        };
        if let (0, Offered::Ready(ready)) = (at, &*offered) {
            if let Err(why) = finalize(ready) {
                eprintln!("{PROGRAM}: {why}");
                *offered = Offered::Unavailable(copy::AUTO_FAILED.to_string());
                continue;
            }
        }
        return restart();
    }
}

/// Builds the last screen's rows. They carry the key as text under its
/// heading, the steps the firmware still asks for under their own, and where
/// the log went. The key is the one row the user must copy by eye. Where a
/// first-boot enrolment is staged, the rows end with what each action does,
/// and the automatic action's window draws in the warning colour.
pub(crate) fn done_rows(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
    offered: &Offered,
) -> Vec<Choice> {
    let mut rows = Vec::new();
    if let Some(key) = recovery {
        rows.push(Choice::new(copy::RECOVERY_HEADING, "").heading());
        rows.push(Choice::new(copy::write_down(), "").content());
        // A blank sits above and below the volumes, so a photograph of the
        // key names the disk it opens even when the key is cropped out.
        rows.push(Choice::new("", ""));
        for volume in volumes {
            rows.push(Choice::new(volume.clone(), "").content());
        }
        rows.push(Choice::new("", ""));
        rows.push(Choice::new(key, "").content().tinted());
    }
    if let Some(steps) = steps {
        if !rows.is_empty() {
            rows.push(Choice::new("", ""));
        }
        rows.push(Choice::new(copy::NEXT_STEPS_HEADING, "").heading());
        // A row of this list carries the cursor's two-column marker beside
        // it, so prose wraps two columns short of the panel's room and the
        // last word stays whole.
        for line in common::ui::table::wrap(steps, common::ui::ROW_ROOM) {
            rows.push(Choice::new(line, "").content());
        }
    }
    for note in notes {
        rows.push(Choice::new(note.clone(), "").content());
    }
    rows.push(Choice::new(copy::logging(log), "").content());
    if !matches!(offered, Offered::None) {
        rows.push(Choice::new("", ""));
        for (at, paragraph) in copy::automatic_explanation().into_iter().enumerate() {
            if at > 0 {
                rows.push(Choice::new("", ""));
            }
            for line in common::ui::table::wrap(paragraph, common::ui::ROW_ROOM) {
                rows.push(Choice::new(line, "").content());
            }
        }
        if let Offered::Ready(ready) = offered {
            let window = match ready.pin {
                true => copy::AUTO_WINDOW_PIN,
                false => copy::AUTO_WINDOW,
            };
            rows.push(Choice::new(window, "").warning().content());
        }
    }
    rows
}

/// The live environment runs systemd, which is what puts the installer on its
/// console.
pub(crate) fn restart() -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("reboot")
        .output()
        .map_err(|err| format!("systemctl: {err}, and it is what restarts a machine"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "systemctl reboot: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Takes the console. Every widget after this draws full screen under one
/// title bar. A login banner and a discovery line sit above, and neither is
/// worth the room.
pub fn own_screen(payload: &Payload, prompt: &Prompt) {
    if !prompt.draws() {
        return;
    }
    // This never calls a terminal's own `clear`, because no terminal is open
    // yet and this must work as the first bytes the process writes.
    print!("\u{1b}[2J\u{1b}[H");
    let _ = std::io::stdout().flush();
    common::ui::own_screen(copy::installing(&image_name(&payload.image)));
}

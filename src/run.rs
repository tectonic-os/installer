use super::*;
use std::io::{BufRead as _, Write as _};

/// Completes the recipe and runs fisherman over it, drawing its event stream
/// into a bounded region and writing all of it to a file.
///
/// A failed draw never fails the install, so no call here uses `?` on the
/// region.
pub fn run(payload: &Payload, answers: &mut Answers, prompt: &Prompt) -> Result<(), String> {
    require_signed_boot_chain(&payload.image, &payload.boot)?;
    // A TPM2 answer stages an enrolment the image performs on its own first
    // boot. This is the last moment before a write that can refuse an image
    // unable to perform it.
    if answers.opened == Opened::Tpm2 {
        require_tpm2_enrolment(&payload.image)?;
    }
    // Every step that can fail for a reason the cut has nothing to do with
    // runs first. `complete` parses the recipe and shells out to `openssl` for
    // the password hash, and neither depends on the cut. A missing `openssl`
    // or an unreadable recipe found after the cut would abort with the disk
    // already repartitioned. This call throws its document away, because the
    // created partitions carry no nodes yet and fisherman gets a later one.
    // The call runs only to find those failures while the disk is untouched.
    complete(&payload.recipe, answers)?;
    // The cut is the first step that touches the disk. Every later step names
    // devices the cut creates, and a container cannot open on a partition that
    // is not there yet.
    if let Some(layout) = answers.layout.as_mut() {
        cut_partitions(layout)?;
    }
    // Every container the layout opens stays open for the whole of fisherman.
    // Every way out of this function closes them, including the panic path.
    let _opened = open_volumes(answers.layout.as_ref())?;
    let staged = stage(&complete(&payload.recipe, answers)?)?;
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
    // One pipe carries both streams, so fisherman's stderr arrives as an
    // event like the rest. An inherited stderr would print straight onto the
    // drawn region.
    let (events, writer) = std::io::pipe().map_err(|err| format!("{BACKEND}: {err}"))?;
    let errors = writer
        .try_clone()
        .map_err(|err| format!("{BACKEND}: {err}"))?;
    let mut child = Command::new(BACKEND)
        .arg(&staged.0)
        .stdout(Stdio::from(writer))
        .stderr(Stdio::from(errors))
        .spawn()
        .map_err(|err| format!("{BACKEND}: {err}"))?;
    let mut region = match prompt.draws() {
        true => common::ui::Progress::open(&copy::writing(at.as_deref())).ok(),
        false => None,
    };
    let mut recovery = None;
    {
        // A thread reads the pipe and this takes lines with a timeout, so the
        // region keeps drawing while fisherman is silent. A step can hold the
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
            let event = Event::of(&line);
            if let Event::Recovery(key) = &event {
                recovery = Some(key.clone());
            }
            if let (Some(said), Some(log)) = (event.logged(), &mut log) {
                let _ = writeln!(log, "{said}");
            }
            match &mut region {
                None => println!("{}", event.say()),
                Some(region) => {
                    let _ = match &event {
                        Event::Step(pct, flight, what) => region.step(*pct, *flight, what),
                        Event::Done(message) => region.step(100, 0, message),
                        // Held back until the region closes. The recovery key
                        // is the one line on this screen worth reading
                        // twice.
                        Event::Recovery(_) => Ok(()),
                        Event::Note(message) => region.note(message),
                        Event::Other(line) => region.note(line),
                    };
                }
            }
        }
    }
    let finished = child.wait().map_err(|err| format!("{BACKEND}: {err}"));
    if let Some(region) = region {
        region.close();
    }
    // The staged recipe carries the password hash and the passphrase, and the
    // install is over. Dropping it here rather than at the end of `run` keeps
    // the recipe from outliving fisherman while the menu is rendered.
    drop(staged);
    let status = finished?;
    if !status.success() {
        return Err(format!(
            "{BACKEND} did not finish: {status}, and {}",
            copy::logging(at.as_deref())
        ));
    }
    // The containers the layout opened must open again on the installed
    // machine, and no other step arranges that on the path the user chose.
    // This runs before the menu, because the menu bakes the BLS options in.
    let notes = arrange(payload, answers)?;
    configure_boot_chain(&payload.image, &answers.disk, &payload.boot)?;
    render_menu(&payload.image, &answers.disk)?;
    // The steps the last screen asks for are read from the firmware now
    // rather than from the form's panel. A key enrolled while the install ran
    // changes them.
    let steps = next_steps(&payload.boot, &firmware(payload));
    let volumes = match recovery.is_some() {
        true => recovery_volumes(answers),
        false => Vec::new(),
    };
    finish(
        recovery.as_deref(),
        at.as_deref(),
        steps,
        &notes,
        &volumes,
        prompt,
    )
}

/// Lists what a recovery key opens, so a photograph of the screen names the
/// disk. The list carries the install disk and every partition the layout
/// chose to open. Fisherman makes the automatic layout's root, so on that path
/// the disk is the only device this side knows.
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
/// machine and no other screen says what to do next. `notes` states what
/// arranging the opened containers owes about slots, which this says rather
/// than does. `volumes` names what the key opens.
pub(crate) fn finish(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
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
    match common::ui::offer_over(
        copy::INSTALL_DONE,
        done_rows(recovery, log, steps, notes, volumes),
        copy::RESTART,
        copy::DONE_KEYS,
    )? {
        false => Ok(()),
        true => restart(),
    }
}

/// Builds the last screen's rows. They carry the key as text under its
/// heading, the steps the firmware still asks for under their own, and where
/// the log went. The key is the one row the user must copy by eye.
pub(crate) fn done_rows(
    recovery: Option<&str>,
    log: Option<&Path>,
    steps: Option<&str>,
    notes: &[String],
    volumes: &[String],
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

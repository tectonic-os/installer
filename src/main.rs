//! Reads the arguments, runs the install and prints what it did.
//!
//! `cli::Args` owns the words. A flag the words leave out becomes a question
//! on the screen and never a default, because this binary erases disks.

use clap::Parser;
use common::prompt::Prompt;
use installer::cli::Args;
use installer::PROGRAM;
use std::process::ExitCode;

/// Reports every run that did not finish. A flag this binary does not take, a
/// root with nothing installable on it, and a backend that failed after the
/// disk was written all end here. The code is the one `tect` answers with, and
/// the screen and the install log name what the user does next.
const USAGE_ERROR: u8 = 1;

fn main() -> ExitCode {
    // Rust ignores SIGPIPE, so a run piped into `head` panics on the write.
    // The default handler ends it quietly.
    // SAFETY: the two arguments are the constants `SIGPIPE` and `SIG_DFL`, and
    // the call holds no pointer and returns a value this code ignores.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };

    match run() {
        Ok(code) => code,
        Err(message) => {
            // A message that is already a sentence keeps its own
            // punctuation. A block of sentences keeps it too.
            let stop = match message.ends_with(['.', '!', '?']) || message.contains('\n') {
                true => "",
                false => ".",
            };
            eprintln!("Error: {message}{stop}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// Help prints clap's text and exits 0; a refusal prints one `Error:` line and
/// exits 1.
fn refused(error: clap::Error) -> ExitCode {
    use clap::error::ErrorKind;
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{error}");
            ExitCode::SUCCESS
        }
        _ => {
            // clap's own text opens with `error:`, and the run's form is one
            // `Error:` and then its message, so the parser's prefix is dropped.
            let message = error.to_string();
            let message = message.strip_prefix("error: ").unwrap_or(&message);
            eprintln!("Error: {message}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let words: Vec<String> = std::env::args().skip(1).collect();
    // clap carries no version flag, because the release tag spells the `v`
    // this line prints. Installer media runs the flag once as a probe that
    // the binary starts.
    if words == ["--version"] {
        println!("{PROGRAM} v{}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    let args = match Args::try_parse_from(std::iter::once(PROGRAM.to_string()).chain(words)) {
        Ok(args) => args,
        Err(error) => return Ok(refused(error)),
    };
    let prompt = Prompt::new(args.no_tui);

    // Bound to a name so the lock lives until the install ends. `let _` would
    // release it before the disk is touched. Installer media starts this on
    // tty1 and autologins root elsewhere, so the lock stops two installers at
    // once.
    let _lock = installer::hold()?;
    let root = match args.from {
        Some(from) => from,
        None => installer::root()?,
    };
    let found = installer::classify(&root)?;
    let payload = found.payload()?;
    eprintln!("{PROGRAM}: {}, from {}", payload.image, root.display());
    // From here the installer owns the console. The login banner and the
    // discovery line above are not worth the room.
    installer::own_screen(payload, &prompt);
    let given = installer::Given {
        disk: args.disk,
        hostname: args.hostname,
        user: args.user,
        password: args.password,
        encryption: args.encryption,
        passphrase: args.passphrase,
        pin: args.pin,
    };
    // Leaving the review ends the run. No disk has been touched yet.
    let Some(mut answers) = installer::Answers::collect(payload, given, &prompt)? else {
        return Ok(ExitCode::SUCCESS);
    };
    installer::run(payload, &mut answers, &prompt)?;
    Ok(ExitCode::SUCCESS)
}

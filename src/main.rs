//! Reads the arguments, runs the install and prints what it did.
//!
//! This binary installs, so it carries one command and no command table. The
//! words decide where the payload sits and which of the user's answers were
//! given outright. A guess here erases a disk, so a missing flag becomes a
//! question on the screen and never a default.

use common::prompt::Prompt;
use installer::PROGRAM;
use std::path::PathBuf;
use std::process::ExitCode;

/// Reports every run that did not finish. A flag this binary does not take, a
/// root with nothing installable on it, and a backend that failed after the
/// disk was written all end here. The code is the one `tect` answers with, and
/// the screen and the install log name what the user does next.
const USAGE_ERROR: u8 = 1;

/// Lists the user's half of the recipe, as the words gave it. Every flag here
/// is also a row on the screen, so the installer asks for a flag left out
/// rather than assume it.
const FLAGS: &[&str] = &[
    "from",
    "disk",
    "hostname",
    "user",
    "password",
    "encryption",
    "passphrase",
    "pin",
];

/// Holds the words with every `--<flag> <value>` and `--<flag>=<value>` taken
/// out. A word left over is a mistyped flag or an argument this takes none of.
/// The installer refuses both before it reads a payload.
struct Args {
    words: Vec<String>,
    given: Vec<(String, String)>,
}

impl Args {
    fn parse(words: Vec<String>) -> Result<Self, String> {
        let mut args = Args {
            words,
            given: Vec::new(),
        };
        for flag in FLAGS {
            let mut i = 0;
            while i < args.words.len() {
                let taken = if let Some(value) = args.words[i].strip_prefix(&format!("--{flag}=")) {
                    args.given.push(((*flag).to_string(), value.to_string()));
                    1
                } else if args.words[i] == format!("--{flag}") {
                    let value = args
                        .words
                        .get(i + 1)
                        .ok_or_else(|| format!("`--{flag}` takes a value"))?;
                    args.given.push(((*flag).to_string(), value.clone()));
                    2
                } else {
                    i += 1;
                    continue;
                };
                args.words.drain(i..i + taken);
            }
        }
        Ok(args)
    }

    /// Removes `--<flag>`. A switch takes no value.
    fn switch(&mut self, flag: &str) -> bool {
        let before = self.words.len();
        self.words.retain(|word| word != &format!("--{flag}"));
        self.words.len() != before
    }

    /// Returns the last value given, which is what a flag repeated on one
    /// line means.
    fn flag(&self, flag: &str) -> Option<String> {
        self.given
            .iter()
            .rev()
            .find(|(name, _)| name == flag)
            .map(|(_, value)| value.clone())
    }
}

fn main() -> ExitCode {
    // Rust ignores SIGPIPE, so a run piped into `head` panics on the write.
    // The default handler ends it quietly.
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

fn usage() -> String {
    format!(
        "{PROGRAM} — install the image this media carries onto a disk\n\n\
         usage: {PROGRAM} [--from <root>] [--disk <device>] [--hostname <name>]\n\
         \x20                     [--user <name>] [--password <password>]\n\
         \x20                     [--encryption <kind>] [--passphrase <passphrase>]\n\
         \x20                     [--pin <pin>]\n\n\
         Every flag is also a question on the screen. `--from` names the payload\n\
         root, which defaults to the media this binary was booted from.\n"
    )
}

fn run() -> Result<ExitCode, String> {
    let mut args = Args::parse(std::env::args().skip(1).collect())?;
    if args.words == ["--version"] {
        println!("{PROGRAM} v{}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    // Taken before the help check, the way `tect`'s parser takes it. A switch
    // belongs to every invocation. Leaving it in front of `--help` would make
    // `--no-tui --help` a refusal.
    let prompt = Prompt::new(args.switch("no-tui"));
    if matches!(
        args.words.first().map(String::as_str),
        Some("-h" | "--help")
    ) {
        print!("{}", usage());
        return Ok(ExitCode::SUCCESS);
    }
    if let [word, ..] = args.words.as_slice() {
        return Err(format!("`{PROGRAM}` does not take {word}"));
    }

    // Bound to a name so the lock lives until the install ends. `let _` would
    // release it before the disk is touched. Installer media starts this on
    // tty1 and autologins root elsewhere, so the lock stops two installers at
    // once.
    let _lock = installer::hold()?;
    let root = match args.flag("from") {
        Some(from) => PathBuf::from(from),
        None => installer::root()?,
    };
    let found = installer::classify(&root)?;
    let payload = found.payload()?;
    eprintln!("{PROGRAM}: {}, from {}", payload.image, root.display());
    // From here the installer owns the console. The login banner and the
    // discovery line above are not worth the room.
    installer::own_screen(payload, &prompt);
    let given = installer::Given {
        disk: args.flag("disk"),
        hostname: args.flag("hostname"),
        user: args.flag("user"),
        password: args.flag("password"),
        encryption: args.flag("encryption"),
        passphrase: args.flag("passphrase"),
        pin: args.flag("pin"),
    };
    // Leaving the review ends the run. No disk has been touched yet.
    let Some(mut answers) = installer::Answers::collect(payload, given, &prompt)? else {
        return Ok(ExitCode::SUCCESS);
    };
    installer::run(payload, &mut answers, &prompt)?;
    Ok(ExitCode::SUCCESS)
}

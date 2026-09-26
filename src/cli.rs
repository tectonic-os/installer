//! The installer's command line. The parser, `--help` and `docs/commands.md`
//! read this one tree, so a flag and its documentation cannot disagree.

use clap::{ColorChoice, Parser};
use std::path::PathBuf;

mod help;

// Every flag is optional, because the screen asks for what the words leave
// out. A default here would answer a question the user was never asked, and
// this binary erases disks.
#[derive(Parser)]
#[command(
    name = "tect-installer",
    about = help::ABOUT,
    long_about = help::LONG_ABOUT,
    after_long_help = help::NOTES,
    color = ColorChoice::Never
)]
pub struct Args {
    /// the payload root, else the one a TECT partition or this media carries
    #[arg(long, value_name = "dir")]
    pub from: Option<PathBuf>,
    /// the block device to erase and install onto
    #[arg(long, value_name = "dev")]
    pub disk: Option<String>,
    /// what the installed machine is called, else the published name the recipe
    /// carries
    #[arg(long, value_name = "name")]
    pub hostname: Option<String>,
    /// the account to create, in the target's admin group
    #[arg(long, value_name = "name")]
    pub user: Option<String>,
    /// its password, hashed before it is written anywhere
    #[arg(long, value_name = "secret")]
    pub password: Option<String>,
    /// none, tpm2-luks, luks-passphrase, tpm2-luks-passphrase or tpm2-luks-pin;
    /// none by default
    #[arg(long, value_name = "type")]
    pub encryption: Option<String>,
    /// what unlocks the disk, for the two forms that are named for it
    #[arg(long, value_name = "secret")]
    pub passphrase: Option<String>,
    /// typed at every unlock alongside the TPM policy, for tpm2-luks-pin alone
    #[arg(long, value_name = "secret")]
    pub pin: Option<String>,
    /// ask nothing, and fail naming the flag a missing answer needs
    #[arg(long = "no-tui")]
    pub no_tui: bool,
}

//! Installs a bootc image onto a disk, from a payload the boot media carries.
//!
//! The crate finds a payload root, reads the recipe in that root, adds the
//! user's half of the answers, and hands the completed recipe to fisherman.
//! Fisherman owns partitioning, LUKS, TPM2 enrolment and `bootc install`.
//!
//! A payload root is any directory holding `install-recipe.json`. `--from`
//! names one outright, and `payload::root` finds one by the partition label
//! `TECT` when no flag names it. `docs/image-contract.md` states what the
//! recipe and the image must hold.

pub mod copy;
pub mod firmware;

pub(crate) use common::json::{self, Json};
pub(crate) use common::prompt::Prompt;
pub(crate) use common::ui::Choice;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::{Command, Stdio};

mod answers;
mod backend;
mod boot;
mod collect;
mod discover;
mod disks;
mod editor;
mod env;
mod etcwrite;
mod form;
mod lock;
mod panel;
mod payload;
mod recipe;
mod run;
mod table;
mod volumes;

pub use answers::*;
pub(crate) use backend::*;
pub(crate) use boot::*;
pub(crate) use discover::*;
pub use disks::*;
pub(crate) use editor::*;
pub(crate) use env::*;
pub(crate) use etcwrite::*;
pub(crate) use form::*;
pub use lock::*;
pub(crate) use panel::*;
pub use payload::*;
pub use recipe::*;
pub use run::*;
pub(crate) use table::*;
pub(crate) use volumes::*;

/// Names this installer wherever it names itself. It prefixes the lines a run
/// prints, and it is the command a refusal tells the user to type again. The
/// name is written once here, because a `[[bin]]` renamed in `Cargo.toml`
/// alone would leave the user instructions that do not run.
pub const PROGRAM: &str = "tect-installer";

/// Recognises a root holding a module repository instead of a payload, so the
/// refusal names what it found. `classify` only tests that the file exists.
/// This installer never reads it and never writes it.
pub const REPO_FILE: &str = "repo.kdl";

/// Names the build record the media carries beside its recipe. `media` uses
/// only its directory, which is the root `--from` defaults to.
pub const BUILD_MANIFEST: &str = "/usr/share/tectonic/manifest.json";

#[cfg(test)]
mod tests;

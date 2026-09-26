use super::*;
use crate::cli::Args;
use clap::error::ErrorKind;
use clap::Parser;

/// Every flag is optional, because the screen asks for each answer the words
/// leave out. A required one would refuse a run that has nothing to answer yet.
#[test]
fn no_flag_is_required() {
    let args = Args::try_parse_from(["tect-installer"]).expect("the screen asks for every answer");
    assert_eq!(args.from, None);
    assert!(!args.no_tui);
}

#[test]
fn an_unknown_flag_is_refused() {
    let error = Args::try_parse_from(["tect-installer", "--bogus"])
        .err()
        .expect("no such flag");
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn a_flag_missing_its_value_is_refused() {
    let error = Args::try_parse_from(["tect-installer", "--disk"])
        .err()
        .expect("no value");
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
}

/// Overriding a flag would make the last value win, and a disk named twice is
/// the case where guessing wrong erases the wrong one.
#[test]
fn a_repeated_flag_is_refused() {
    let error = Args::try_parse_from(["tect-installer", "--disk", "a", "--disk", "b"])
        .err()
        .expect("a repeated flag");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn every_flag_reaches_its_field() {
    let args = Args::try_parse_from([
        "tect-installer",
        "--from",
        "/run/tect-payload",
        "--disk=/dev/sda",
        "--hostname",
        "workstation",
        "--user",
        "me",
        "--password",
        "hunter2",
        "--encryption",
        "tpm2-luks",
        "--passphrase",
        "opensesame",
        "--pin",
        "123456",
        "--no-tui",
    ])
    .expect("every flag parses");
    assert_eq!(args.from.as_deref(), Some(Path::new("/run/tect-payload")));
    assert_eq!(args.disk.as_deref(), Some("/dev/sda"));
    assert_eq!(args.hostname.as_deref(), Some("workstation"));
    assert_eq!(args.user.as_deref(), Some("me"));
    assert_eq!(args.password.as_deref(), Some("hunter2"));
    assert_eq!(args.encryption.as_deref(), Some("tpm2-luks"));
    assert_eq!(args.passphrase.as_deref(), Some("opensesame"));
    assert_eq!(args.pin.as_deref(), Some("123456"));
    assert!(args.no_tui);
}

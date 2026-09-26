use super::*;
use std::ffi::OsString;

#[test]
fn the_account_password_is_hashed_before_it_reaches_useradd() {
    let hash = hashed("hunter2").expect("a password hash");
    assert!(hash.starts_with("$6$"), "{hash}");
    assert!(!hash.contains("hunter2"), "{hash}");
}

/// Each deployment shape needs a different account command because composefs
/// has no complete rootfs to chroot into during installation.
#[test]
fn the_useradd_argv_follows_the_deployment_backend() {
    let words = |argv: Vec<OsString>| -> Vec<String> {
        argv.iter()
            .map(|word| word.to_string_lossy().to_string())
            .collect()
    };
    let ostree = Path::new("/run/tect-sysroot/ostree/deploy/workstation/deploy/abc.0");
    assert_eq!(
        words(useradd_args(
            ostree,
            false,
            "tester",
            &["wheel".to_string()],
            "$6$salt$hash"
        )),
        [
            "chroot",
            "/run/tect-sysroot/ostree/deploy/workstation/deploy/abc.0",
            "useradd",
            "--no-create-home",
            "--shell",
            "/bin/bash",
            "--password",
            "$6$salt$hash",
            "--groups",
            "wheel",
            "tester",
        ]
    );
    let composefs = Path::new("/run/tect-sysroot/state/deploy/abc");
    assert_eq!(
        words(useradd_args(
            composefs,
            true,
            "tester",
            &["sudo".to_string()],
            "$6$salt$hash"
        )),
        [
            "useradd",
            "--root",
            "/run/tect-sysroot/state/deploy/abc",
            "--no-create-home",
            "--shell",
            "/bin/bash",
            "--password",
            "$6$salt$hash",
            "--groups",
            "sudo",
            "tester",
        ]
    );
    assert_eq!(
        words(useradd_args(composefs, true, "tester", &[], "$6$salt$hash")),
        [
            "useradd",
            "--root",
            "/run/tect-sysroot/state/deploy/abc",
            "--no-create-home",
            "--shell",
            "/bin/bash",
            "--password",
            "$6$salt$hash",
            "tester",
        ]
    );
}

/// A missing target group must be dropped before `useradd`, which otherwise
/// refuses the account and leaves the install unfinished.
#[test]
fn only_the_requested_groups_the_deployment_holds_survive() {
    let root = scratch("account-groups");
    let etc = root.join("etc");
    std::fs::create_dir_all(&etc).expect("an etc");
    std::fs::write(
        etc.join("group"),
        "root:x:0:\nwheel:x:10:tester\nsudo:x:27:\n",
    )
    .expect("a group file");
    let asked = ["docker", "wheel", "sudo"].map(str::to_string);
    assert_eq!(
        present_groups(&etc, &asked).expect("a retained list"),
        ["wheel", "sudo"]
    );
}

#[test]
fn a_group_file_that_cannot_be_read_is_an_error() {
    let root = scratch("account-no-group");
    assert!(present_groups(&root, &["wheel".to_string()]).is_err());
}

#[test]
fn the_tmpfiles_snippet_copies_the_skeleton_once() {
    assert_eq!(
        tmpfiles("tester"),
        "C /var/home/tester 0700 tester tester - /etc/skel\n"
    );
}

#[test]
fn only_existing_account_files_are_returned_for_labels() {
    let root = scratch("account-paths");
    let etc = root.join("etc");
    std::fs::create_dir_all(&etc).expect("an etc");
    for name in ["hostname", "passwd", "shadow", "group", "gshadow"] {
        std::fs::write(etc.join(name), "").expect("a file");
    }
    std::fs::write(etc.join("subuid"), "").expect("a file");
    let snippet = etc.join("tmpfiles.d/tect-home-tester.conf");
    std::fs::create_dir_all(snippet.parent().expect("a directory")).expect("a tmpfiles dir");
    std::fs::write(&snippet, "").expect("a snippet");
    let at = paths_to_label(&etc, &snippet);
    assert!(at.contains(&etc.join("hostname")));
    assert!(at.contains(&etc.join("subuid")));
    assert!(at.contains(&snippet));
    assert!(!at.contains(&etc.join("subgid")));
}

/// The target's `useradd` would refuse these only after bootc had written the
/// disk.
#[test]
fn only_a_portable_account_name_is_taken() {
    for name in ["tester", "_svc", "a-b_9"] {
        assert!(portable_name(name), "{name}");
    }
    for name in [
        "",
        "John",
        "9lives",
        "has space",
        "dollar$",
        &"a".repeat(33),
    ] {
        assert!(!portable_name(name), "{name}");
    }
}

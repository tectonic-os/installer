use super::*;

/// A retried install runs `add_boot_arg` over an entry it already patched,
/// so the second run leaves the options line alone. Two copies of one
/// `rd.luks.name` argument would leave the user guessing which run wrote it.
#[test]
fn a_boot_argument_lands_on_the_options_line_once() {
    let raw = "title Fedora\nlinux /vmlinuz\noptions root=UUID=aa quiet\n";
    let (text, changed) = add_boot_arg(raw, "rd.luks.name=bb=root");
    assert!(changed);
    assert!(text.contains("options root=UUID=aa quiet rd.luks.name=bb=root"));
    assert!(text.ends_with('\n'));
    let (again, changed) = add_boot_arg(&text, "rd.luks.name=bb=root");
    assert!(!changed);
    assert_eq!(again, text);
    // `boot_arg_named` separates the two `false` answers `add_boot_arg`
    // gives. Only the entry that already carries the argument is a retry.
    assert!(boot_arg_named(&text, "rd.luks.name=bb=root"));
    assert!(!boot_arg_named(raw, "rd.luks.name=bb=root"));
    assert!(!boot_arg_named("title nothing\n", "rd.luks.name=bb=root"));
}

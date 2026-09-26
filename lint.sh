#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if [ "${1:-}" = "--fix" ]; then
    cargo fmt
    echo "lint: formatted the source"
    exit
fi

# Every .rs in the crate, so a `build.rs` added at the root later is covered
# without editing this, the way `tect`'s own lint covers a new workspace member.
mapfile -t crate_src < <(find . -path ./target -prune -o -name '*.rs' -type f -print)

# This binary installs a bootc image and knows nothing about the tool that
# built one. A `tect` dependency here would put the repository model back in
# the installer and undo the split, and `common` is the only way to ratatui.
# clap owns the command line, so parsing, help, refusals and `docs/commands.md`
# read one tree.
#
# `normal,dev,build` and not `normal` alone: a `[dev-dependencies]` entry is
# exactly how the coupling would come back, since the split's own history is
# that what looked like 83 production references were 72 test fixtures. That is
# also why `clap-markdown` is named here: it renders the reference in a test
# alone, so no renderer reaches the dist binary.
deps=$(cargo tree -e normal,dev,build --depth 1 --prefix none | awk 'NR > 1 {print $1}' | sort -u | tr '\n' ' ')
if [ "$deps" != "clap clap-markdown common libc " ]; then
    echo "lint: the dependency floor moved, it is now: $deps" >&2
    exit 1
fi

# Belt and braces over the floor above, in every form a reach can take: an
# import, a renaming import, an `extern crate`, or a path written at the call
# site whose import sits in another file.
if grep -nE '\buse +(::)?tect\b|\bextern +crate +tect\b|(^|[^A-Za-z0-9_:])tect::' "${crate_src[@]}"; then
    echo "lint: the installer naming the tect crate, which it does not depend on" >&2
    exit 1
fi

# And a reach that is not a dependency at all: spawning the other binary. Only
# the two spawn forms, because the bare word `tect` is all over this crate as a
# username fixture, a mount point and a unit name on the *installed* machine,
# and none of those is this crate calling that one.
if grep -nE 'Command::new\("[^"]*tect"|"/usr/bin/tect"' "${crate_src[@]}"; then
    echo "lint: the installer running the tect binary, which media may not carry" >&2
    exit 1
fi

# `src` only: the goldens' comments name ratatui where they say why a drawn
# frame arrives in fragments, and prose is not a dependency.
if grep -rn 'ratatui' src; then
    echo "lint: ratatui reached directly, and it belongs to the common crate" >&2
    exit 1
fi

cargo fmt --check || {
    echo "lint: unformatted, run ./lint.sh --fix" >&2
    exit 1
}
echo "lint: the source is clean"

cargo test --quiet
echo "lint: the installer does what it did"

# `PROGRAM` is a constant interpolated into `format!`, so a message built as a
# plain `&str` keeps the braces and tells a person to run `{PROGRAM}`. A
# literal in the built binary is the one reliable tell: `format!` stores the
# pieces either side of the hole, a plain string stores the hole. This caught
# `lock.rs`'s second refusal, which two tests passed straight through.
cargo build --quiet
if strings target/debug/tect-installer | grep -n '{PROGRAM}'; then
    echo "lint: a message carries {PROGRAM} uninterpolated, so it is not in a format!" >&2
    exit 1
fi
echo "lint: every message names the program rather than the placeholder"

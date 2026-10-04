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

# This check covers every source form that the dependency graph cannot see: an
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

cargo deny --locked check bans licenses sources
echo "lint: the dependency policy accepts the locked graph"

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

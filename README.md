# tect-installer

Installs a bootc image onto a machine's disk, from a payload the boot media
carries rather than from a registry.

It is its own binary and its own repository. The tool that builds an image and
the binary that writes one to a disk are released apart, so the installer can
link the libraries that only exist on installer media while the build tool stays
a static portable binary.

## Using it

On installer media it runs as `tect-installer.service` on `tty1`. Run by hand it
takes a lock, so a second one refuses and names the terminal holding the first.

    tect-installer --disk /dev/sda --hostname workstation --user me --password …

With no flags it asks, on one screen, and every question is answered where it
stands. `docs/commands.md` is the whole reference: the flags, the screens and
what each one refuses.

## What it installs from

Two inputs, and neither is this project's build tool. A recipe document,
`install-recipe.json`, in a payload root the media carries; and the bootc image
that recipe names.

`docs/image-contract.md` states what each must hold. The short version: a
hand-written recipe needs `image`, `hostname`, `filesystem`, `bootloader` and
`additionalImageStores`, and an image needs no other field unless it declares a
signed boot chain or an encrypted root.
Any bootc workflow can write that input.

The installer performs every step itself. It finds the payload, cuts and
formats the disk,
creates and opens the LUKS containers, runs `bootc install to-filesystem` from
the payload's image through `podman`, and writes the hostname, the account and
the unlock configuration into the installed deployment.

## Working on it

    ./lint.sh          the dependency floor, the boundary, formatting and the tests
    ./lint.sh --fix    format

`clap`, `common` and `libc` are the runtime dependency floor, and
`clap-markdown` renders the command reference in a test alone. `lint.sh` fails
if either moves. It also fails if a file under `src/` or `tests/` names the
`tect` crate
or spawns the `tect` binary. A file under `src/` fails if it reaches ratatui at
all, because `common` is the only way to draw; the goldens under `tests/` may
name ratatui in a comment. Those checks are the split, so they run before cargo
is asked.

    common = { git = "https://github.com/tectonic-os/common", rev = "<sha>" }

pinned by commit, never by version. A change there is a commit and a pin bump
here.

## Licence

Apache 2.0. See [LICENSE](LICENSE).

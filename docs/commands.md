# Commands

This document contains the help content for the `tect-installer` command-line program.

**Command Overview:**

* [`tect-installer`↴](#tect-installer)

## `tect-installer`

`tect-installer` installs a built bootc image onto this machine's disk, from a
payload the media carries rather than from a registry. The tool that builds an
image and the binary that writes it to a disk are separate repositories,
released apart.

It finds the payload, cuts and formats the disk, creates and opens the LUKS
containers, and runs `bootc install to-filesystem` from the payload's image
through `podman`. It then writes the hostname, the account and the unlock
configuration into the installed deployment.

On installer media this runs as `tect-installer.service` on `tty1`, under
kmscon where the base has it and on the kernel console where it does not. The
media is single purpose: nothing logs in on `tty1`, and leaving the installer
starts it again rather than reaching a shell. A serial console, where one is
configured, autologins root to a shell for watching a run and does not start an
installer.

Run by hand it takes a lock on `/run/tect-installer.lock` for the length of the
run, so a second one refuses and names the terminal holding the first. The lock
is the process's: nothing has to be cleaned up after an installer that was
killed. `$TECT_INSTALLER_LOCK` names the file elsewhere, which is how the
screens are run without root.

**Usage:** `tect-installer [OPTIONS]`

Notes:

- **A payload root is a directory carrying `install-recipe.json`.** That is the
  whole discovery rule. The document is what `tect recipe` emits and what
  `tect vm build iso` bakes onto installer media, and it names the image and the
  store beside it, so nothing here opens a container image or re-derives a boot
  chain.
- **Where the root comes from, when no `--from` names one, is the filesystem
  label `TECT`.** One partition carrying it is mounted read-only at
  `/run/tect-payload` and classified there; more than one is refused naming
  every one of them, because picking wrong erases a disk from the wrong image.
  With none, the root is `/usr/share/tectonic`, which is where installer media
  carries its own recipe beside the manifest, so a labelled partition
  overrides the media, and someone who attached one did that deliberately.
- **On a terminal one form holds the answers, and enter edits a row where it
  stands.** The flags seed it (disk, hostname, username, password, confirm,
  encryption); text is typed into the row, and the disk list opens *under* its
  own row. The passphrase and the PIN belong to the encryption answer, which
  asks them in a window of its own.
- **`Install` is dim and unpickable until nothing is missing**, with what it is
  waiting for written under it: a disk, a username, a password, and a passphrase
  for the two encryption forms named for one. It is also where the two
  halves of the password are compared, since both are on screen at once and can
  be checked against each other instead of asked for twice.
- **`Switch to shell`, `Restart now` and `Shut down` sit beside it** and are
  never blocked, so a run that cannot finish can still be left. `Switch to
  shell` asks first, because it stops the installer and hands the console a
  root shell; the note it shows names the installer's tty and the key that
  returns to it.
- **Taking `Install` asks once more**, over a summary of what it would do: a box
  headed `Installation Summary`, `Ready to install?` over the rows, the cost in
  one line (*Erase everything on `/dev/sda`? This cannot be undone.*), and
  `Start Installation` and `Go back` as buttons under them. The summary rows
  cannot be landed on, up and down or left and right move between the buttons,
  and esc is `Go back`.
- The disk is offered as the whole disks `/sys/block` holds, with their size,
  model and whether they are removable. `$TECT_SYS_BLOCK` names that directory
  instead where it is set, which is how the drawn golden stops depending on the
  disks of whichever machine runs it. The password and the passphrase are typed
  masked, since neither is visible to correct and a mistyped one is a machine
  the user cannot log into.
- **The installer owns the console.** It clears the screen at its entry, which
  holds a login banner and a discovery line above it, and everything after that
  is drawn in one centred, titled box naming the image. On a Fedora live
  environment the console itself is kmscon, which draws TrueType through DRM;
  where it cannot start, systemd hands tty1 back to the kernel VT and the same
  screen draws in that font instead.
- **Esc inside a field goes back to the form and changes nothing.** A field the
  user leaves keeps what it had, which is nothing the first time; `Install` then
  names it among the values it is waiting for. No question in the installer can
  end the run.
- **Esc on the form itself is a question, not an exit**, and so is Ctrl+C
  anywhere: keep going, start again from the first question, or leave to a
  shell. Only the third leaves, and it exits 0 having touched nothing.
- The two forms whose name ends in `passphrase` wait for a passphrase before
  `Install`, and `tpm2-luks-pin` waits for a PIN, typed and confirmed the same
  way. The `tpm2-` forms are not drawn at all on a machine with no
  `/dev/tpmrm0`. For `tpm2-luks` and `tpm2-luks-pin` the installer formats the
  containers with a generated recovery key and shows it once at the end on the
  last screen or on stdout, so the user has to record it. A staged copy stays
  inside the encrypted root until the first boot enrols the TPM, and the
  enrolment deletes it. The PIN is typed at every unlock, on top of the TPM
  policy. A stolen machine still needs the PIN, so the TPM cannot unlock it
  alone and an attacker has no passphrase to attack offline.
- The installer reads `bootc`'s output a line at a time. On a terminal the
  lines go into a bounded pane under a bar that keeps moving while `bootc` is
  silent; with no terminal each line is printed as it arrives. **The bar's two ends differ by glyph as
  well as by colour**, so a console that drops the colour escapes still reads a
  partial bar.
- **The whole of that output is written to `tect-install.log`**, which is what
  makes the bounded region safe: the screen is no longer the only copy of the
  transcript a failure on someone else's machine is diagnosed from. It goes
  beside the payload where the `TECT` partition can be remounted writable, and
  in `/run` where there is none, because an iso-only boot has no writable
  partition at all. Which of the two happened is on screen under the gauge and
  said again at the end.
- **The recovery key is the one thing that file never holds.** A key written to
  removable media turns the stick into the thing that opens the disk, so it
  stays on screen and nowhere else, said once more after the install finishes.
- **Finishing offers the restart**, defaulting to it, rather than returning
  silently to a root shell with the stick still in the machine.
- **A payload wins over a repository, which is the opposite of how a repository
  wins over a booted image everywhere else.** For authoring, the repository is
  the source and is the more specific answer. For installing, the payload is the
  artifact: it is already built, and rebuilding it to reach the same bytes is the
  slowest possible way to be less certain.
- It refuses a root that holds a `repo.kdl` and nothing built, naming
  `tect build`, and refuses a root that holds neither, naming both files. Both
  refusals come before any question, so no user is asked for a disk to erase on
  a root that cannot install.
- The disk, the username and the password have no default. With nobody to ask,
  a missing one fails naming its flag, and there is no default disk. The hostname
  defaults to the name the payload carries, and an unset `--encryption` installs
  unencrypted.
- The installer hashes the password with `openssl passwd -6` before it changes
  the disk, so a password `openssl` cannot hash stops the run with the disk
  untouched. `useradd --password` writes the hash into the installed
  deployment, and no file receives the plaintext.
- `user.groups` names the *target's* admin group, `sudo` on Debian and `wheel`
  on Fedora. The installer skips a listed group the installed deployment has
  not got and warns, because `useradd` refuses the whole call when it receives
  one.

###### **Options:**

* `--from <dir>` — the payload root, else the one a TECT partition or this media carries
* `--disk <dev>` — the block device to erase and install onto
* `--hostname <name>` — what the installed machine is called, else the published name the recipe carries
* `--user <name>` — the account to create, in the target's admin group
* `--password <secret>` — its password, hashed before it is written anywhere
* `--encryption <type>` — none, tpm2-luks, luks-passphrase, tpm2-luks-passphrase or tpm2-luks-pin; none by default
* `--passphrase <secret>` — what unlocks the disk, for the two forms that are named for it
* `--pin <secret>` — typed at every unlock alongside the TPM policy, for tpm2-luks-pin alone
* `--no-tui` — ask nothing, and fail naming the flag a missing answer needs




# The installer

`tect-installer` installs a built bootc image onto this machine's disk, from a
payload the media carries rather than from a registry. It is its own binary and
its own repository; the tool that builds an image and the binary that writes one
to a disk are released apart.

This file is what the installer does and how it is run.
[What an image and a recipe must carry](image-contract.md) is the other half:
the fields a recipe must hold and the paths an image must carry, for a bootc
workflow that never ran `tect`.

## Running it

Installs a built tectonic image onto this machine's disk, from a payload rather
than from a registry. `fisherman` is the backend and does the partitioning, the
LUKS work and the `bootc install`; this finds the payload, completes its recipe
and hands it over.

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

#### Flags:
    --from <dir>          the payload root, else the one a TECT partition or
                          this media carries
    --disk <dev>          the block device to erase and install onto
    --hostname <name>     what the installed machine is called, else the
                          published name the recipe carries
    --user <name>         the account to create, in the target's admin group
    --password <secret>   its password, hashed before it is written anywhere
    --encryption <type>   none, tpm2-luks, luks-passphrase,
                          tpm2-luks-passphrase or tpm2-luks-pin; none by
                          default
    --passphrase <secret> what unlocks the disk, for the two forms that
                          are named for it
    --pin <secret>        typed at every unlock alongside the TPM policy,
                          for tpm2-luks-pin alone

#### Notes:
- **A payload root is a directory carrying `install-recipe.json`.** That is the
  whole discovery rule. The document is what `tect recipe` emits and what
  `tect vm build iso` bakes onto installer media, and it names the image and the
  store beside it — so nothing here opens a container image or re-derives a boot
  chain.
- **Where the root comes from, when no `--from` names one, is the filesystem
  label `TECT`.** One partition carrying it is mounted read-only at
  `/run/tect-payload` and classified there; more than one is refused naming
  every one of them, because picking wrong erases a disk from the wrong image.
  With none, the root is `/usr/share/tectonic`, which is where installer media
  carries its own recipe beside the manifest — so a labelled partition
  overrides the media, and someone who attached one did that deliberately.
- **On a terminal there is one screen and every field is on it**, answered
  where it stands. The flags and the payload seed a form — disk, computer name,
  username, password, confirm, encryption, passphrase — and enter on a row edits
  it in place: text is typed into the row, and the disk list opens *under* its
  own row. Nothing hides the other answers while one of them is being changed.
- **`Install` is dim and unpickable until nothing is missing**, with what it is
  waiting for written under it: a disk, a username, a password, and — for the
  two encryption forms named for one — a passphrase. It is also where the two
  halves of the password are compared, since both are on screen at once and can
  be checked against each other instead of asked for twice.
- **`Exit to shell`, `Restart now` and `Shut down` sit beside it** and are never
  blocked. A screen nobody can leave is worse than one nobody can finish. `Exit
  to shell` asks first, under a box headed `Close Installer`, because it stops
  the installer and hands the console a root shell: *To restart the installer
  run 'tect-installer'.*
- **Taking `Install` asks once more**, over a summary of what it would do: a box
  headed `Installation Summary`, `Ready to install?` over the rows, the cost in
  one line (*Everything on `/dev/sda` will be erased.*), and `Start
  Installation` and `Go back` as buttons under them. The summary rows cannot be
  landed on, left and right move between the buttons, and esc is `Go back`.
- The disk is offered as the whole disks `/sys/block` holds, with their size,
  model and whether they are removable. `$TECT_SYS_BLOCK` names that directory
  instead where it is set, which is how the drawn golden stops depending on the
  disks of whichever machine runs it. The password and the passphrase are typed
  masked, since neither is visible to correct and a mistyped one is a machine
  nobody can log into.
- **The installer owns the console.** It clears the screen at its entry — what
  is above it is a login banner and a discovery line — and everything after that
  is drawn in one centred, titled box naming the image. On a Fedora live
  environment the console itself is kmscon, which draws TrueType through DRM;
  where it cannot start, systemd hands tty1 back to the kernel VT and the same
  screen draws in that font instead.
- **Esc inside a field goes back to the form and changes nothing.** A field you
  leave keeps what it had, which is nothing the first time; `Install` then names
  it among the values it is waiting for. No question in the installer can end
  the run.
- **Esc on the form itself is a question, not an exit**, and so is Ctrl+C
  anywhere: keep going, start again from the first question, or leave to a
  shell. Only the third leaves, and it exits 0 having touched nothing.
- Encryption is fisherman's, and the two forms whose name ends in `passphrase`
  are the two it refuses the recipe without one; `tpm2-luks-pin` is the one it
  refuses without a PIN, typed and confirmed the same way. The `tpm2-` forms
  are not drawn at all on a machine with no `/dev/tpmrm0`. For
  `tpm2-luks` and `tpm2-luks-pin` fisherman prints a recovery key once, and the
  install echoes it on a line of its own: it is the only copy there will ever
  be. The PIN is typed at every unlock, on top of the TPM policy: a stolen
  machine gets neither the automatic unlock a bare TPM gives nor an offline
  attack on a passphrase.
- Fisherman writes a JSON event per line. On a terminal they are drawn as a
  bar with the step under it and the last few messages in a bounded pane under
  that; with no terminal each one is rendered as a line — `[ 45%] 7/12 install
  OS` — over whatever else it prints. **The bar's two ends differ by glyph as
  well as by colour**, so a console that drops the colour escapes still reads a
  partial bar.
- **The whole of that output is written to `tect-install.log`**, which is what
  makes the bounded region safe: the screen is no longer the only copy of the
  transcript a failure on someone else's machine is diagnosed from. It goes
  beside the payload where the `TECT` partition can be remounted writable, and
  in `/run` where there is none — an iso-only boot has no writable partition at
  all. Which of the two happened is on screen under the gauge and said again at
  the end.
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
- A root holding a `repo.kdl` and nothing built is refused naming `tect build`,
  and a root holding neither is refused naming both files. Either refusal comes
  before anything is asked, so nobody is asked for a disk to erase on a root that
  cannot install.
- The three flags are the person's half of the recipe and nothing derives them.
  With nobody to ask, each missing one fails naming itself; there is no default
  disk.
- The password is hashed with `openssl passwd -6` before it reaches the recipe,
  and the completed recipe is written `0600` under `$TMPDIR`. Fisherman hands the
  field to `chpasswd`, and only a `$`-prefixed crypt string takes its `-e`
  branch: a plaintext one goes through PAM and dies after the OS is already on
  the disk.
- `user.groups` is merged rather than replaced. The group in the emitted recipe
  is the *target's* admin group — `sudo` on Debian, `wheel` on Fedora — and
  `useradd` refuses the whole call when it is handed a group the target has not
  got.


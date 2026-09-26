# What an image and a recipe must carry

`tect-installer` installs a bootc image from a payload root the media carries.
It takes two inputs, and neither of them is this project's build tool: a recipe
document, and the image the recipe names. This file states what each one must
hold, so a bootc workflow that never ran `tect` can write the same input.

Nothing here opens the image to read a plan out of it. Every image fact the
installer uses is either a field of the recipe or a file it probes for by
running the image under podman, and both lists are below in full.

## The recipe floor

The payload root is any directory holding `install-recipe.json`. That document
is the installer's own format. `InstallRecipe::read` in `src/recipe.rs` holds
every field it accepts, and it reads the document when the root is classified,
before anything is asked.

### Five fields are the floor

A hand-written recipe installs with five fields:

    {
      "image": "quay.io/fedora/fedora-bootc:42",
      "hostname": "workstation",
      "filesystem": "ext4",
      "bootloader": "grub2",
      "additionalImageStores": ["/var/lib/tectonic/store"]
    }

`InstallRecipe::read` refuses the root naming whichever of the five is missing
or empty. `bootloader` is `grub2` or `systemd`, and it decides whether the
whole-disk layout cuts a separate `/boot`, whether bootc takes
`--bootloader systemd`, and with `composeFsBackend` whether the root is found by
its partition type, so no default stands in for it. `additionalImageStores` is
required because the install runs with `--pull=never`, so it reads the image
from the media store and contacts no registry. `validate_recipe` in `src/run.rs` asks podman for the image before
the cut, so a store that lacks it is refused with the disk untouched.

`filesystem` is the whole-disk root's filesystem, and the editor draws its
automatic plan with it. A manual root takes the filesystem the user chooses.
`tect` writes `xfs`, `ext4` or `btrfs` here. `prepare` in `src/layout.rs` checks
every filesystem the plan formats before the cut, so a filesystem it has no
`mkfs` for is refused with the disk untouched. `composeFsBackend` needs fs-verity on the
root, which `xfs` has not got, so `InstallRecipe::read` refuses that pair. The
installer formats the whole-disk root and an opened old root with the recipe's
`filesystem`, and the manual editor applies the same rule to any other root.

### A field the installer does not implement is refused

`InstallRecipe::read` refuses a field outside its list, naming it, and it
refuses a field that occurs twice. `user` takes `groups` and nothing else. The
refusal comes before the cut because a recipe carrying `varDisk`, `zfsPoolName`
or `btrfsSubvolumes` names a layout this installer does not write, and
installing without it would give the user a machine the image did not describe.

### What the user's answers add

The form supplies the disk, the hostname, the username, the password and the
encryption kind, with a passphrase or a PIN where the kind needs one. None of
them is written back into the recipe. The installer hashes the password with
`openssl passwd -6` before it changes the disk, and `useradd --password` writes
the hash into the installed deployment.

`user.groups` names the target's admin group, which differs by family. A listed
group the installed deployment has not got and warns, because `useradd`
refuses the whole call when it receives one.

### What the recipe is read for and does not have to hold

| Field | Read by | Absent |
| --- | --- | --- |
| `targetImgref` | `bootc_command`, as `--target-imgref` | the installed machine updates from `image` |
| `composeFsBackend` | the root filesystem rules and `bootc_command` | treated as false, and a root may be any filesystem |
| `genericImage` | `bootc_command`, as `--generic-image` | treated as false |
| `boot` | `require_signed_boot_chain`, `configure_boot_chain` | no chain is declared, and the existing boot path installs |
| `luksInitramfs` | `kinds` in `src/disks.rs` | no encrypted kind is drawn on the form |
| `user.groups` | `account::configure` | the account is made in no extra group |

Each of these is optional. The installer reads a default, and the form does not
draw the option. An image that is sealed with composefs and omits
`composeFsBackend` is installed unsealed, and it loses the rule that holds its
root to ext4 or btrfs. No screen names the field.

### The two fields that describe the image

`boot` and `luksInitramfs` are claims about the image that the installer never
passes to bootc.

`boot` names the UKI trust chain the image was built for, `uki-db` or
`uki-shim`. A recipe declaring one is refused before the disk is cut unless the
image carries `/usr/share/secureboot/signed`.

`luksInitramfs` claims the image's initramfs carries a LUKS userspace driver. It
gates every encrypted kind on the form. A recipe without it installs plain or
does not install, whatever the machine's TPM says, so an image whose initramfs
opens LUKS must say so here.

## The image contract

Five paths inside the image. The installer probes for each by running the image
under `podman run --rm --pull=never --net=none`, so an image that carries none
of them still installs unencrypted on the existing boot path.

| Path | Read by | When | Absent |
| --- | --- | --- | --- |
| `/usr/share/secureboot/signed` | `require_signed_boot_chain` | before the cut, where `boot` is declared | the install is refused, and the disk is untouched |
| `/usr/share/secureboot/sb_cert.pem` | `owner_certificate` in `src/panel.rs` | while the firmware is read, where `boot` is declared and a foreign key holds the platform key | the firmware reads as holding a foreign key. On the `uki-db` chain the last screen then asks for the vendor's setup-mode steps |
| `/usr/share/secureboot/pcr-policy.pem` | `pcr_policy_in` | while `/etc` is written, where the install creates or opens a container with TPM2 | the first-boot token binds to PCR 7 alone |
| `/usr/libexec/secureboot-enrolment` | `run_enrolment` | after bootc, on the `uki-shim` chain | the install fails with the disk already written |
| `/usr/libexec/grub-menu-from-bls` | `run_renderer` | after bootc | the step is skipped, and the install succeeds |

Of those five, only the signed marker is a refusal, and only it is read before
anything is written. An image declaring `uki-shim` must carry the enrolment
helper as well as the marker, because nothing checks for the helper until
bootc has finished. That chain also ignores the owner certificate when it
asks for the next steps: it always asks for the shim steps, whatever the
firmware holds.

The renderer is for an image whose signed GRUB reads no BLS entries. An image
that does not ship it exits 3 from the probe, which is the skip.

Two more probes read paths no vendor owns. `require_tpm2_enrolment` runs
`/usr/bin/systemd-cryptenroll` in the image, because the image enrols that token
itself on its first boot. **It is the second refusal that runs before the cut**,
and it runs for every `tpm2-` kind and wherever a manual layout opened a
container with TPM2. `image_gb` in
`src/payload.rs` sizes the smallest root the install accepts with `podman image
inspect`, which runs no container.

### Paths of the media, not of the image

`/usr/share/tectonic/manifest.json` names the directory that `--from` defaults
to. It belongs to the media this installer boots from, not to the image being
installed, and the image being installed is never asked for it. A payload
partition labelled `TECT` overrides that default.

## Where this is checked

`a_hand_written_recipe_names_the_media_store` in `src/tests/payload.rs` writes
the five-field recipe above, classifies it, asserts the defaults the table
names, asserts that no encrypted kind is offered, and then asserts that
`classify` refuses the same recipe without its store. `a_recipe_without_a_bootloader_is_refused_by_name` in
`src/tests/recipe.rs` holds the `bootloader` refusal, and
`a_sealed_recipe_on_a_filesystem_without_verity_is_refused` beside it holds the
fs-verity rule. `a_recipe_field_the_installer_does_not_implement_is_refused_by_name`
in `src/tests/recipe.rs` adds `varDisk` to an emitted recipe and asserts the
refusal names it. `a_plan_the_payload_cannot_answer_refuses_before_the_cut` in
`src/tests/layout.rs` holds the whole-disk refusals. A change that adds a sixth
required field breaks the first test.

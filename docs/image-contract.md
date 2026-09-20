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
is fisherman's own recipe format with two fields added, listed under [The two
fields outside fisherman's schema](#the-two-fields-outside-fishermans-schema).

### Three fields are the floor

A hand-written recipe installs with three fields:

    {
      "image": "quay.io/fedora/fedora-bootc:42",
      "hostname": "workstation",
      "filesystem": "ext4"
    }

`classify` in `src/payload.rs` requires `image` and `hostname`, and refuses the
root naming the missing one. It does not require `filesystem`, and fisherman
does: `Validate` accepts only `xfs`, `ext4`, `btrfs` or `zfs` on the
auto-partitioning branch. `complete` in `src/recipe.rs` never writes that field,
so a recipe without it reaches fisherman without it. **Nothing catches that
before the disk is cut.** `run` in `src/run.rs` partitions first and starts
fisherman after, so a recipe missing `filesystem` fails `Validate` on a machine
whose old partition table is already gone. Write it by hand.

`filesystem` also constrains two other fields. `composeFsBackend` needs
fs-verity, so fisherman refuses it on `xfs`. `zfs` refuses LUKS encryption.

### What the user's answers add

`complete` merges the form into the recipe before fisherman sees it. The user
supplies `disk`, `hostname`, `user.username`, `user.password` and
`encryption.type`, with `encryption.passphrase` and `encryption.pin` where the
chosen kind needs them. The password is hashed with `openssl passwd -6` first.

`user` is merged and not replaced, so `user.groups` in the recipe survives. That
group is the target's admin group, which differs by family, and `useradd`
refuses the whole call when it names a group the target has not got.

A manual layout writes `customMounts` and removes `varDisk`. The removal is
unconditional, because fisherman refuses `customMounts` beside a `varDisk` that
sets a size or asks to be encrypted.

### What the recipe is read for and does not have to hold

| Field | Read by | Absent |
| --- | --- | --- |
| `bootloader` | the panel, the summary the user agrees to, and fisherman | read as `grub2`, and a separate `/boot` is cut |
| `composeFsBackend` | the manual root filesystem rule | treated as false, and a manual root may be any filesystem |
| `boot` | `require_signed_boot_chain`, `configure_boot_chain` | no chain is declared, and the existing boot path installs |
| `luksInitramfs` | `kinds` in `src/disks.rs` | no encrypted kind is drawn on the form |

The install refuses none of these. It reads a default, or the option is never
drawn. Two of those defaults are worth stating outright. An image that
boots with systemd-boot and omits `bootloader` is installed as though it used
GRUB, so it is given a separate `/boot` partition it does not want. An image
that is sealed with composefs and omits `composeFsBackend` loses the rule that
holds a manual root to ext4 or btrfs, and nothing on any screen names either
field.

### The two fields outside fisherman's schema

`boot` and `luksInitramfs` are claims about the image that fisherman never
reads.

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
| `/usr/share/secureboot/pcr-policy.pem` | `pcr_policy_in` | while `/etc` is written, where a manual layout opened a container with TPM2 | the first-boot token binds to PCR 7 alone |
| `/usr/libexec/secureboot-enrolment` | `run_enrolment` | after fisherman, on the `uki-shim` chain | the install fails with the disk already written |
| `/usr/libexec/grub-menu-from-bls` | `run_renderer` | after fisherman | the step is skipped, and the install succeeds |

Of those five, only the signed marker is a refusal, and only it is read before
anything is written. An image declaring `uki-shim` must carry the enrolment
helper as well as the marker, because nothing checks for the helper until
fisherman has finished. That chain also ignores the owner certificate when it
asks for the next steps: it always asks for the shim steps, whatever the
firmware holds.

The renderer is for an image whose signed GRUB reads no BLS entries. An image
that does not ship it exits 3 from the probe, which is the skip.

Two more probes read paths no vendor owns. `require_tpm2_enrolment` runs
`/usr/bin/systemd-cryptenroll` in the image, because the image enrols that token
itself on its first boot. **It is the second refusal that runs before the cut**,
and it runs where a manual layout opened a container with TPM2. `image_gb` in
`src/payload.rs` runs `podman image inspect`, not a container, to size the root
a separate home must leave room for.

### Paths of the media, not of the image

`/usr/share/tectonic/manifest.json` names the directory that `--from` defaults
to. It belongs to the media this installer boots from, not to the image being
installed, and the image being installed is never asked for it. A payload
partition labelled `TECT` overrides that default.

## Where this is checked

`three_hand_written_fields_are_the_floor_for_a_generic_bootc_image` in
`src/tests/payload.rs` writes the three-field recipe above, classifies it,
asserts every optional field is absent, asserts that no encrypted kind is
offered, and runs `complete` over it to prove `disk`, `hostname`, `image` and
`filesystem` are all present afterwards. Fisherman's `Validate` requires the
first, the second and the last of those; `classify` requires `image`, and
fisherman does not, because bootc can read the reference off the running
container. A change that adds a fourth hand-written field breaks the test.

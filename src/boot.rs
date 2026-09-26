use super::*;

/// Names the file an image writes to witness that its boot chain was signed.
/// The owner decided on 2026-09-20 that this marker is vendor-neutral. Every
/// bootc image that writes the path gets the chain it declares. The
/// `boot/uki` module writes it.
const SIGNED_MARKER: &str = "/usr/share/secureboot/signed";

/// Names the menu renderer a deb image ships and a fedora image does not. The
/// signed GRUB a deb family boots reads no BLS entries. The renderer runs out
/// of the image, because a composefs deployment on the disk is sealed erofs
/// with no walkable `/usr`.
const RENDERER: &str = "/usr/libexec/grub-menu-from-bls";

/// Names the directory podman binds into the image as `/target`. The target's
/// boot filesystem mounts one level down at `<TARGET>/boot`, because the
/// renderer takes a root and looks under `<root>/boot`.
const TARGET: &str = "/run/tect-target";

/// Places the chain-specific files bootc cannot place itself. The `uki-db`
/// chain needs an explicit enrolment policy in `loader.conf`. The `uki-shim`
/// chain needs Microsoft's shim as the fallback loader, with the owner-signed
/// systemd-boot left under the name shim is compiled to load.
pub(crate) fn configure_boot_chain(image: &str, disk: &str, chain: &str) -> Result<(), String> {
    if chain.is_empty() {
        return Ok(());
    }
    let at = PathBuf::from(TARGET);
    let boot = at.join("boot");
    std::fs::create_dir_all(&boot).map_err(|err| format!("{TARGET}: {err}"))?;
    let Some((device, root)) = boot_partition(disk, &boot)? else {
        return Err(format!(
            "no partition of {disk} carries `loader/entries`, so the {chain} chain cannot be configured"
        ));
    };
    let esp = match root {
        "/target" => boot.clone(),
        _ => boot.join("boot"),
    };

    let configured = match chain {
        "uki-db" => {
            let missing = ["PK", "KEK", "db"]
                .into_iter()
                .map(|name| esp.join(format!("loader/keys/auto/{name}.auth")))
                .find(|key| !key.is_file());
            match missing {
                Some(key) => Err(format!(
                    "{}: bootc did not install the {chain} enrollment key",
                    key.display()
                )),
                None => {
                    let loader = esp.join("loader/loader.conf");
                    let current = std::fs::read_to_string(&loader).unwrap_or_default();
                    let mut lines: Vec<&str> = current
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("secure-boot-enroll "))
                        .collect();
                    lines.push("secure-boot-enroll force");
                    std::fs::write(&loader, format!("{}\n", lines.join("\n")))
                        .map_err(|err| format!("{}: {err}", loader.display()))
                }
            }
        }
        "uki-shim" => run_enrolment(image, &esp),
        _ => Err(format!("unknown boot chain `{chain}`")),
    };
    let unmounted = Command::new("umount").arg(&boot).output();
    configured?;
    match unmounted {
        Ok(out) if out.status.success() => {
            eprintln!("{PROGRAM}: configured {chain} on {device}");
            Ok(())
        }
        Ok(out) => Err(format!(
            "unmounting {device}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(err) => Err(format!("unmounting {device}: {err}")),
    }
}

pub(crate) fn require_signed_boot_chain(image: &str, chain: &str) -> Result<(), String> {
    if chain.is_empty() {
        return Ok(());
    }
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "/usr/bin/test",
            image,
            "-e",
            SIGNED_MARKER,
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what verifies the {chain} chain"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} does not carry {SIGNED_MARKER}, which its declared {chain} chain needs; \
         build it with the Secure Boot private key, or with a `boot/uki` new enough to write \
         that path, before installing"
    ))
}

/// Refuses an image that cannot enrol a TPM2 token on its own first boot. The
/// installer asks before it writes to the disk. A staged enrolment the image
/// cannot perform leaves the user a machine that asks for a passphrase the
/// installer promised it would not need.
pub(crate) fn require_tpm2_enrolment(image: &str) -> Result<(), String> {
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            image,
            "/usr/bin/systemd-cryptenroll",
            "--version",
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what checks {image} for TPM2"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{image} has no /usr/bin/systemd-cryptenroll, so it cannot enrol the TPM2 token this answer asks for"
    ))
}

/// Reports whether the image carries a signed PCR 11 policy, which `boot/uki`
/// writes when its build had the PCR signing key. An image without the policy
/// still installs. Its first-boot token then binds to PCR 7 alone, which is
/// what every chain bound to before the policy existed.
pub(crate) fn pcr_policy_in(image: &str) -> bool {
    Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            image,
            "/usr/bin/test",
            "-s",
            "/usr/share/secureboot/pcr-policy.pem",
        ])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn run_enrolment(image: &str, esp: &Path) -> Result<(), String> {
    let target = format!("{}:/esp", esp.display());
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--net=none",
            "--security-opt",
            "label=disable",
            "--entrypoint",
            "",
            "-v",
            &target,
            image,
            "/usr/libexec/secureboot-enrolment",
            "/esp",
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what installs the shim chain"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "/usr/libexec/secureboot-enrolment in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Writes the menu the installed machine boots from, after bootc has
/// finished. `bootc` installs the bootloader before it writes
/// the entries, so no step during the install can render them. An image that
/// ships no renderer is skipped. A renderer that fails for any other reason
/// fails the install.
///
/// `render_menu` in `assets/scripts/vm.sh` is this algorithm over a raw disk
/// file. That copy runs unprivileged and has to `losetup -P` first, which is
/// why the two are separate.
pub(crate) fn render_menu(image: &str, disk: &str) -> Result<(), String> {
    let at = PathBuf::from(TARGET);
    let boot = at.join("boot");
    std::fs::create_dir_all(&boot).map_err(|err| format!("{TARGET}: {err}"))?;

    let Some((device, root)) = boot_partition(disk, &boot)? else {
        return Err(format!(
            "no partition of {disk} carries `loader/entries`, so there is no menu to render"
        ));
    };
    let rendered = run_renderer(image, root);
    let _ = Command::new("umount").arg(&boot).output();
    match rendered {
        Ok(true) => {
            eprintln!("{PROGRAM}: wrote the boot menu {device} needs, since its GRUB reads no BLS");
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(err) => Err(err),
    }
}

/// Mounts the first partition of `disk` whose filesystem carries the boot
/// entries at `boot`, and returns it with the root the renderer wants. A
/// separate /boot partition renders from the top. A /boot directory on the
/// root renders one level down. Nothing in this crate labels a boot
/// partition, so the search reads each filesystem's content.
fn boot_partition(disk: &str, boot: &Path) -> Result<Option<(String, &'static str)>, String> {
    let listed = Command::new("lsblk")
        .args(["-nrpo", "NAME", disk])
        .output()
        .map_err(|err| format!("lsblk: {err}, and it is what lists a disk's partitions"))?;
    for device in labelled(&String::from_utf8_lossy(&listed.stdout)) {
        if device == disk {
            continue;
        }
        let mounted = Command::new("mount")
            .args([&device, &boot.to_string_lossy().to_string()])
            .output();
        if !matches!(&mounted, Ok(out) if out.status.success()) {
            continue;
        }
        if boot.join("loader/entries").is_dir() || boot.join("EFI/Linux").is_dir() {
            return Ok(Some((device, "/target")));
        }
        if boot.join("boot/loader/entries").is_dir() || boot.join("boot/EFI/Linux").is_dir() {
            return Ok(Some((device, "/target/boot")));
        }
        let _ = Command::new("umount").arg(boot).output();
    }
    Ok(None)
}

/// Answers `false` where the image ships no renderer. A fedora target is
/// skipped that way, without this crate listing which families ship
/// `blscfg`.
fn run_renderer(image: &str, root: &str) -> Result<bool, String> {
    let out = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--net=none",
            "--security-opt",
            "label=disable",
            // A built image inheriting an entrypoint would take the shell
            // line as arguments to that entrypoint. `vm.sh` and the scan
            // workflow clear it for the same reason.
            "--entrypoint",
            "",
            "-v",
            &format!("{TARGET}:/target"),
            image,
            "/bin/sh",
            "-c",
            &format!("test -x {RENDERER} || exit 3; exec {RENDERER} {root}"),
        ])
        .output()
        .map_err(|err| format!("podman: {err}, and it is what runs the image's renderer"))?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(3) => Ok(false),
        _ => Err(format!(
            "{RENDERER} in {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

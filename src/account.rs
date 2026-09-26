use super::*;
use std::ffi::OsString;

pub(crate) fn configure(
    sysroot: &Path,
    deployment: &Path,
    composefs: bool,
    answers: &Answers,
    groups: &[String],
    password_hash: &str,
) -> Result<Vec<PathBuf>, String> {
    if !deployment.starts_with(sysroot) {
        return Err(format!(
            "{}: the deployment is not under the mounted root {}",
            deployment.display(),
            sysroot.display()
        ));
    }
    let etc = deployment.join("etc");
    let retained = present_groups(&etc, groups)?;
    for group in groups.iter().filter(|group| !retained.contains(group)) {
        eprintln!("{PROGRAM}: target has no group {group:?}; skipping it");
    }
    let hostname = etc.join("hostname");
    std::fs::write(&hostname, format!("{}\n", answers.hostname))
        .map_err(|err| format!("{}: {err}", hostname.display()))?;
    set_mode(&hostname, 0o644)?;
    let argv = useradd_args(
        deployment,
        composefs,
        &answers.user,
        &retained,
        password_hash,
    );
    run_useradd(&argv).map_err(|why| format!("{}: {why}", deployment.display()))?;
    let tmpfiles_dir = etc.join("tmpfiles.d");
    let snippet = tmpfiles_dir.join(format!("tect-home-{}.conf", answers.user));
    std::fs::create_dir_all(&tmpfiles_dir)
        .map_err(|err| format!("{}: {err}", tmpfiles_dir.display()))?;
    std::fs::write(&snippet, tmpfiles(&answers.user))
        .map_err(|err| format!("{}: {err}", snippet.display()))?;
    set_mode(&snippet, 0o644)?;
    Ok(paths_to_label(&etc, &snippet))
}

/// Says whether an account name is in the portable set this installer
/// accepts. `useradd` runs after bootc has written the disk, so a name outside
/// the set is refused before the cut.
pub(crate) fn portable_name(user: &str) -> bool {
    let mut letters = user.chars();
    letters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && letters.all(|letter| {
            letter.is_ascii_lowercase() || letter.is_ascii_digit() || letter == '_' || letter == '-'
        })
        && user.len() <= 32
}

/// A missing group would make `useradd` refuse the whole account.
pub(crate) fn present_groups(etc: &Path, requested: &[String]) -> Result<Vec<String>, String> {
    let at = etc.join("group");
    let held = std::fs::read_to_string(&at).map_err(|err| format!("{}: {err}", at.display()))?;
    Ok(requested
        .iter()
        .filter(|group| {
            held.lines()
                .any(|line| line.split(':').next() == Some(group.as_str()))
        })
        .cloned()
        .collect())
}

/// An ostree deployment is a full rootfs, while composefs exposes only
/// writable state.
pub(crate) fn useradd_args(
    deployment: &Path,
    composefs: bool,
    user: &str,
    groups: &[String],
    password_hash: &str,
) -> Vec<OsString> {
    let mut argv: Vec<OsString> = match composefs {
        true => vec![
            "useradd".into(),
            "--root".into(),
            deployment.as_os_str().to_os_string(),
            "--no-create-home".into(),
        ],
        false => vec![
            "chroot".into(),
            deployment.as_os_str().to_os_string(),
            "useradd".into(),
            "--no-create-home".into(),
        ],
    };
    argv.extend([
        "--shell".into(),
        "/bin/bash".into(),
        "--password".into(),
        password_hash.into(),
    ]);
    if !groups.is_empty() {
        argv.extend(["--groups".into(), groups.join(",").into()]);
    }
    argv.push(user.into());
    argv
}

/// Runs the account command without copying its password hash into an error.
fn run_useradd(argv: &[OsString]) -> Result<(), String> {
    let (program, args) = argv.split_first().expect("useradd_args builds a command");
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("{}: {err}", program.to_string_lossy()))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    match said.is_empty() {
        true => Err(format!(
            "{}: exited with no message",
            program.to_string_lossy()
        )),
        false => Err(said),
    }
}

/// `C` sets ownership on the whole copied tree only when it copies the
/// skeleton. A recursive `Z` line would reset ownership in the home on each
/// boot.
pub(crate) fn tmpfiles(user: &str) -> String {
    format!("C /var/home/{user} 0700 {user} {user} - /etc/skel\n")
}

pub(crate) fn paths_to_label(etc: &Path, snippet: &Path) -> Vec<PathBuf> {
    let mut at: Vec<PathBuf> = [
        "hostname", "passwd", "shadow", "group", "gshadow", "subuid", "subgid", "passwd-",
        "shadow-", "group-", "gshadow-", "subuid-", "subgid-",
    ]
    .into_iter()
    .map(|name| etc.join(name))
    .collect();
    at.push(snippet.to_path_buf());
    at.retain(|path| path.exists());
    at
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|err| format!("{}: {err}", path.display()))
}

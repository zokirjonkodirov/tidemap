use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct Resource {
    pub absolute: PathBuf,
    pub root: Option<PathBuf>,
    pub relative: String,
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn identity(root: &Path, relative: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(root.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(relative.as_bytes());
    format!("{:x}", digest.finalize())[..32].to_owned()
}
pub(crate) fn eligible(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("mmd")
        })
}
pub(crate) fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .context("File is outside the selected folder")?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("Invalid project-relative file path.");
    }
    Ok(relative
        .to_str()
        .context("This filename is not valid Unicode")?
        .replace('\\', "/"))
}
pub(crate) fn valid_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !path
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part.eq_ignore_ascii_case(".git"))
}

pub(crate) fn resolve(resource: &Resource) -> Result<PathBuf> {
    let metadata =
        fs::symlink_metadata(&resource.absolute).context("This file is missing or unavailable")?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("Symbolic links and non-regular files cannot be edited.");
    }
    let resolved = fs::canonicalize(&resource.absolute)?;
    if let Some(root) = &resource.root {
        if !resolved.starts_with(root)
            || !fs::canonicalize(
                resource
                    .absolute
                    .parent()
                    .context("File has no parent folder")?,
            )?
            .starts_with(root)
        {
            bail!("The file resolves outside its project. Open it explicitly to grant access.");
        }
    }
    Ok(resolved)
}

pub(crate) fn read_bounded(path: &Path, limit: usize, root: Option<&Path>) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("Only regular, non-symlink files can be opened.");
    }
    let canonical = fs::canonicalize(path)?;
    if root.is_some_and(|root| !canonical.starts_with(root)) {
        bail!("The file resolves outside the selected folder.");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let mut file: File = options.open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.file_type().is_symlink() {
        bail!("The path changed to a link or special file while opening.");
    }
    if before.len() > limit as u64 {
        bail!(
            "This source exceeds the {} MiB editing limit. Split it into smaller files.",
            limit / 1024 / 1024
        );
    }
    let mut bytes = Vec::with_capacity((before.len() as usize).saturating_add(1));
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() > limit
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
        || fs::canonicalize(path)? != canonical
    {
        bail!("The file changed while being read. Reopen it for a stable revision.");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current = fs::metadata(path)?;
        if before.ino() != current.ino() || before.dev() != current.dev() {
            bail!("The file was replaced while being read. Reopen it.");
        }
    }
    Ok(bytes)
}

pub(crate) fn decode(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).context(
        "This file is not valid UTF-8. Convert its encoding before editing; no bytes were changed",
    )
}

pub(crate) fn naturally_compare(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut a = left.chars().peekable();
    let mut b = right.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut first = String::new();
                let mut second = String::new();
                while a.peek().is_some_and(char::is_ascii_digit) {
                    first.push(a.next().unwrap());
                }
                while b.peek().is_some_and(char::is_ascii_digit) {
                    second.push(b.next().unwrap());
                }
                let lhs = first.trim_start_matches('0');
                let rhs = second.trim_start_matches('0');
                let order = lhs
                    .len()
                    .cmp(&rhs.len())
                    .then_with(|| lhs.cmp(rhs))
                    .then_with(|| first.len().cmp(&second.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                let order = x
                    .to_lowercase()
                    .to_string()
                    .cmp(&y.to_lowercase().to_string());
                if order != Ordering::Equal {
                    return order;
                }
            }
            (None, None) => return left.cmp(right),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
        }
    }
}

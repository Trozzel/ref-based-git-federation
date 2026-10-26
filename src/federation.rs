use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs, io};

use tracing::{error, info};

use crate::git::utils::{get_remotes, remove_remote_if_exists};

pub const FEDERATION_DIR: &str = ".federation";
pub const FOLLOWING_FILE: &str = "following";
pub const ALLOW_FAST_FORWARD_MERGE_FILE: &str = "allow-fast-forward-merge";
const LEGACY_ALLOW_FAST_FORWARD_MERGE_FILE: &str = "allow-ffm";
pub const FOLLOW_ALL_SITES: &str = "*";
const FOLLOWING_RECONCILE_INTERVAL: Duration = Duration::from_secs(2);

pub fn following_file_path(repo_path: impl AsRef<Path>) -> PathBuf {
    repo_path.as_ref().join(FEDERATION_DIR).join(FOLLOWING_FILE)
}

pub fn allow_fast_forward_merge_file_path(repo_path: impl AsRef<Path>) -> PathBuf {
    repo_path
        .as_ref()
        .join(FEDERATION_DIR)
        .join(ALLOW_FAST_FORWARD_MERGE_FILE)
}

fn legacy_allow_fast_forward_merge_file_path(repo_path: impl AsRef<Path>) -> PathBuf {
    repo_path
        .as_ref()
        .join(FEDERATION_DIR)
        .join(LEGACY_ALLOW_FAST_FORWARD_MERGE_FILE)
}

fn ensure_federation_file(
    repo_path: impl AsRef<Path>,
    file_name: &str,
    default_contents: &str,
) -> io::Result<PathBuf> {
    let repo_path = repo_path.as_ref();
    let federation_dir = repo_path.join(FEDERATION_DIR);
    fs::create_dir_all(&federation_dir)?;

    let path = federation_dir.join(file_name);
    if !path.exists() {
        fs::write(&path, default_contents)?;
    }

    Ok(path)
}

pub fn ensure_following_file(repo_path: impl AsRef<Path>) -> io::Result<PathBuf> {
    ensure_federation_file(repo_path, FOLLOWING_FILE, &format!("{FOLLOW_ALL_SITES}\n"))
}

pub fn ensure_allow_fast_forward_merge_file(repo_path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let repo_path = repo_path.as_ref();
    let legacy_path = legacy_allow_fast_forward_merge_file_path(repo_path);
    let current_path = allow_fast_forward_merge_file_path(repo_path);

    if !current_path.exists() && legacy_path.exists() {
        fs::create_dir_all(repo_path.join(FEDERATION_DIR))?;
        fs::rename(&legacy_path, &current_path)?;
        return Ok(current_path);
    }

    ensure_federation_file(repo_path, ALLOW_FAST_FORWARD_MERGE_FILE, "")
}

fn load_site_list(path: &Path) -> io::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }

    let contents = fs::read_to_string(path)?;

    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn write_site_list(path: &Path, sites: &HashSet<String>) -> io::Result<()> {
    let mut ordered_sites = sites.iter().cloned().collect::<BTreeSet<_>>();
    let wildcard_present = ordered_sites.remove(FOLLOW_ALL_SITES);

    let mut lines = Vec::new();
    if wildcard_present {
        lines.push(FOLLOW_ALL_SITES.to_string());
    }
    lines.extend(ordered_sites);

    let contents = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(path, contents)
}

pub fn load_following_sites(repo_path: impl AsRef<Path>) -> io::Result<HashSet<String>> {
    let following_path = following_file_path(repo_path);
    if !following_path.exists() {
        return Ok(HashSet::from([FOLLOW_ALL_SITES.to_string()]));
    }

    load_site_list(&following_path)
}

pub fn load_allow_fast_forward_merge_sites(
    repo_path: impl AsRef<Path>,
) -> io::Result<HashSet<String>> {
    let current_path = allow_fast_forward_merge_file_path(&repo_path);
    if current_path.exists() {
        return load_site_list(&current_path);
    }

    let legacy_path = legacy_allow_fast_forward_merge_file_path(repo_path);
    load_site_list(&legacy_path)
}

pub fn write_following_sites(
    repo_path: impl AsRef<Path>,
    sites: &HashSet<String>,
) -> io::Result<()> {
    ensure_following_file(&repo_path)?;
    write_site_list(&following_file_path(repo_path), sites)
}

pub fn write_allow_fast_forward_merge_sites(
    repo_path: impl AsRef<Path>,
    sites: &HashSet<String>,
) -> io::Result<()> {
    ensure_allow_fast_forward_merge_file(&repo_path)?;
    write_site_list(&allow_fast_forward_merge_file_path(repo_path), sites)
}

pub fn follows_site(repo_path: impl AsRef<Path>, site_name: &str) -> io::Result<bool> {
    let followed_sites = load_following_sites(repo_path)?;
    Ok(followed_sites.contains(FOLLOW_ALL_SITES) || followed_sites.contains(site_name))
}

pub fn allows_ffm_site(repo_path: impl AsRef<Path>, site_name: &str) -> io::Result<bool> {
    Ok(load_allow_fast_forward_merge_sites(repo_path)?.contains(site_name))
}

pub fn add_following_site(repo_path: impl AsRef<Path>, site_name: &str) -> io::Result<()> {
    let trimmed_site = site_name.trim();
    if trimmed_site.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "site name must not be empty",
        ));
    }

    let mut sites = load_following_sites(&repo_path)?;
    sites.insert(trimmed_site.to_string());
    write_following_sites(repo_path, &sites)
}

pub fn set_allow_fast_forward_merge_site(
    repo_path: impl AsRef<Path>,
    site_name: &str,
    enabled: bool,
) -> io::Result<()> {
    let trimmed_site = site_name.trim();
    if trimmed_site.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "site name must not be empty",
        ));
    }

    let mut sites = load_allow_fast_forward_merge_sites(&repo_path)?;
    if enabled {
        sites.insert(trimmed_site.to_string());
    } else {
        sites.remove(trimmed_site);
    }
    write_allow_fast_forward_merge_sites(repo_path, &sites)
}

pub fn reconcile_following_remotes(
    repo_path: impl AsRef<Path>,
    followed_sites: &HashSet<String>,
) -> io::Result<()> {
    if followed_sites.contains(FOLLOW_ALL_SITES) {
        return Ok(());
    }

    let repo_path = repo_path.as_ref();
    let repo_path_str = repo_path.to_string_lossy();
    let remotes = get_remotes(&repo_path_str)?;

    for remote in remotes {
        if followed_sites.contains(&remote) {
            continue;
        }

        info!(
            "removing remote '{}' because it is no longer present in {}",
            remote,
            following_file_path(repo_path).display()
        );
        remove_remote_if_exists(&repo_path_str, &remote)
            .map_err(|err| io::Error::other(err.to_string()))?;
    }

    Ok(())
}

pub async fn reconcile_following_remotes_loop(repo_path: String) {
    let mut interval = tokio::time::interval(FOLLOWING_RECONCILE_INTERVAL);
    let mut previous_followed_sites: Option<HashSet<String>> = None;

    loop {
        interval.tick().await;

        let followed_sites = match load_following_sites(&repo_path) {
            Ok(followed_sites) => followed_sites,
            Err(err) => {
                error!(
                    "failed to read following file for repo {}: {}",
                    repo_path, err
                );
                continue;
            }
        };

        if previous_followed_sites.as_ref() == Some(&followed_sites) {
            continue;
        }

        if let Err(err) = reconcile_following_remotes(&repo_path, &followed_sites) {
            error!(
                "failed to reconcile remotes from following file for repo {}: {}",
                repo_path, err
            );
            continue;
        }

        previous_followed_sites = Some(followed_sites);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use super::{
        FOLLOW_ALL_SITES, add_following_site, allow_fast_forward_merge_file_path, allows_ffm_site,
        ensure_allow_fast_forward_merge_file, ensure_following_file, following_file_path,
        follows_site, load_allow_fast_forward_merge_sites, load_following_sites,
        set_allow_fast_forward_merge_site,
    };

    #[test]
    fn ensure_following_file_creates_repo_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();

        let following_path = ensure_following_file(temp_dir.path()).unwrap();

        assert_eq!(following_path, following_file_path(temp_dir.path()));
        assert!(following_path.exists());
        assert_eq!(fs::read_to_string(&following_path).unwrap(), "*\n");
    }

    #[test]
    fn load_following_sites_ignores_blank_lines_and_whitespace() {
        let temp_dir = tempfile::tempdir().unwrap();
        let following_path = ensure_following_file(temp_dir.path()).unwrap();

        fs::write(&following_path, " alpha \n\nbeta\n alpha\n").unwrap();

        let followed_sites = load_following_sites(temp_dir.path()).unwrap();

        assert_eq!(followed_sites.len(), 2);
        assert!(followed_sites.contains("alpha"));
        assert!(followed_sites.contains("beta"));
    }

    #[test]
    fn load_following_sites_defaults_to_wildcard_when_file_is_missing() {
        let temp_dir = tempfile::tempdir().unwrap();

        let followed_sites = load_following_sites(temp_dir.path()).unwrap();

        assert_eq!(
            followed_sites,
            HashSet::from([FOLLOW_ALL_SITES.to_string()])
        );
    }

    #[test]
    fn load_following_sites_preserves_wildcard_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let following_path = ensure_following_file(temp_dir.path()).unwrap();

        fs::write(&following_path, "*\nalpha\n").unwrap();

        let followed_sites = load_following_sites(temp_dir.path()).unwrap();

        assert!(followed_sites.contains(FOLLOW_ALL_SITES));
        assert!(followed_sites.contains("alpha"));
    }

    #[test]
    fn follows_site_respects_wildcard_and_trimmed_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let following_path = ensure_following_file(temp_dir.path()).unwrap();

        fs::write(&following_path, " alpha \n").unwrap();
        assert!(follows_site(temp_dir.path(), "alpha").unwrap());
        assert!(!follows_site(temp_dir.path(), "beta").unwrap());

        fs::write(&following_path, " * \n").unwrap();
        assert!(follows_site(temp_dir.path(), "beta").unwrap());
    }

    #[test]
    fn ensure_allow_fast_forward_merge_file_creates_repo_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();

        let allow_fast_forward_merge_path =
            ensure_allow_fast_forward_merge_file(temp_dir.path()).unwrap();

        assert_eq!(
            allow_fast_forward_merge_path,
            allow_fast_forward_merge_file_path(temp_dir.path())
        );
        assert!(allow_fast_forward_merge_path.exists());
        assert_eq!(
            fs::read_to_string(&allow_fast_forward_merge_path).unwrap(),
            ""
        );
    }

    #[test]
    fn add_following_site_persists_site() {
        let temp_dir = tempfile::tempdir().unwrap();
        ensure_following_file(temp_dir.path()).unwrap();

        add_following_site(temp_dir.path(), "site-2").unwrap();

        let followed_sites = load_following_sites(temp_dir.path()).unwrap();
        assert!(followed_sites.contains(FOLLOW_ALL_SITES));
        assert!(followed_sites.contains("site-2"));
    }

    #[test]
    fn set_allow_fast_forward_merge_site_round_trips() {
        let temp_dir = tempfile::tempdir().unwrap();
        ensure_allow_fast_forward_merge_file(temp_dir.path()).unwrap();

        set_allow_fast_forward_merge_site(temp_dir.path(), "site-2", true).unwrap();
        assert!(allows_ffm_site(temp_dir.path(), "site-2").unwrap());
        assert_eq!(
            load_allow_fast_forward_merge_sites(temp_dir.path()).unwrap(),
            HashSet::from(["site-2".to_string()])
        );

        set_allow_fast_forward_merge_site(temp_dir.path(), "site-2", false).unwrap();
        assert!(!allows_ffm_site(temp_dir.path(), "site-2").unwrap());
        assert!(
            load_allow_fast_forward_merge_sites(temp_dir.path())
                .unwrap()
                .is_empty()
        );
    }
}

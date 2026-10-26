use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fmt, io};

use tracing::{debug, info};

use crate::axum::models::ConflictResolutionChoice;

#[derive(Debug)]
pub struct GitError(pub String);

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<git2::Error> for GitError {
    fn from(err: git2::Error) -> Self {
        GitError(err.to_string())
    }
}

/// Get commit hash of a ref, branch name, or descriptor like "HEAD~2"
pub fn get_commit_hash(repo_path: &str, git_ref: &str) -> io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("rev-parse")
        .arg("--verify")
        .arg(git_ref)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git rev-parse failed for git_ref='{}': {}",
            git_ref,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn get_merge_base(
    repo_path: &str,
    left_ref: &str,
    right_ref: &str,
) -> io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge-base")
        .arg(left_ref)
        .arg(right_ref)
        .output()?;

    match output.status.code() {
        Some(0) => {}
        Some(1) => {
            return Err(io::Error::other(format!(
                "no merge base for left_ref='{}' right_ref='{}'",
                left_ref, right_ref
            )));
        }
        _ => {
            return Err(io::Error::other(format!(
                "git merge-base failed for left_ref='{}' right_ref='{}': {}",
                left_ref,
                right_ref,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn read_file_at_ref(
    repo_path: &str,
    git_ref: &str,
    file_path: &str,
) -> io::Result<Option<String>> {
    let object = format!("{git_ref}:{file_path}");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("show")
        .arg(&object)
        .output()?;

    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if git_show_missing_path(&stderr) {
        return Ok(None);
    }

    Err(io::Error::other(format!(
        "git show failed for object='{}': {}",
        object,
        stderr.trim()
    )))
}

fn git_show_missing_path(stderr: &str) -> bool {
    stderr.contains("does not exist in")
        || stderr.contains("exists on disk, but not in")
        || stderr.contains("path not in")
}

#[derive(Debug, Clone)]
struct GitIndexStageEntry {
    mode: String,
    object: String,
}

fn temp_index_path(repo_path: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Path::new(repo_path).join(".git").join(format!(
        "ref-based-fed-index-{}-{}",
        std::process::id(),
        nanos
    ))
}

fn git_status_with_index(repo_path: &str, index_path: &Path, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .env("GIT_INDEX_FILE", index_path)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "git command failed with args={args:?} using temp index '{}'",
            index_path.display()
        )));
    }

    Ok(())
}

fn git_output_with_index(repo_path: &str, index_path: &Path, args: &[&str]) -> io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .env("GIT_INDEX_FILE", index_path)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git command failed with args={args:?} using temp index '{}': {}",
            index_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn get_index_stage_entry(
    repo_path: &str,
    index_path: &Path,
    file_path: &str,
    stage: u8,
) -> io::Result<Option<GitIndexStageEntry>> {
    let output =
        git_output_with_index(repo_path, index_path, &["ls-files", "-s", "--", file_path])?;
    if output.is_empty() {
        return Ok(None);
    }

    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let Some(mode) = fields.next() else {
            continue;
        };
        let Some(object) = fields.next() else {
            continue;
        };
        let Some(stage_field) = fields.next() else {
            continue;
        };
        if stage_field == stage.to_string() {
            return Ok(Some(GitIndexStageEntry {
                mode: mode.to_string(),
                object: object.to_string(),
            }));
        }
    }

    Ok(None)
}

fn set_index_to_stage_choice(
    repo_path: &str,
    index_path: &Path,
    file_path: &str,
    choice: ConflictResolutionChoice,
) -> io::Result<()> {
    let stage = match choice {
        ConflictResolutionChoice::Ours => 2,
        ConflictResolutionChoice::Theirs => 3,
    };

    if let Some(entry) = get_index_stage_entry(repo_path, index_path, file_path, stage)? {
        git_status_with_index(
            repo_path,
            index_path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{},{},{}", entry.mode, entry.object, file_path),
            ],
        )?;
        return Ok(());
    }

    git_status_with_index(
        repo_path,
        index_path,
        &["update-index", "--force-remove", "--", file_path],
    )
}

pub(crate) fn resolve_conflicted_proposal_ref(
    repo_path: &str,
    proposal_ref: &str,
    resolutions: &[(String, ConflictResolutionChoice)],
) -> io::Result<String> {
    let base = get_merge_base(repo_path, "main", proposal_ref)?;
    let proposal_commit = get_commit_hash(repo_path, proposal_ref)?;
    let main_commit = get_commit_hash(repo_path, "main")?;
    let index_path = temp_index_path(repo_path);

    let result = (|| -> io::Result<String> {
        git_status_with_index(
            repo_path,
            &index_path,
            &["read-tree", "-m", &base, "main", proposal_ref],
        )?;

        for (file_path, choice) in resolutions {
            set_index_to_stage_choice(repo_path, &index_path, file_path, *choice)?;
        }

        let tree = git_output_with_index(repo_path, &index_path, &["write-tree"])?;
        let message = format!("Resolve conflicts for {}", proposal_ref);
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("commit-tree")
            .arg(&tree)
            .arg("-p")
            .arg(&main_commit)
            .arg("-p")
            .arg(&proposal_commit)
            .arg("-m")
            .arg(&message)
            .output()?;

        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git commit-tree failed for proposal_ref='{}': {}",
                proposal_ref,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let new_commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("update-ref")
            .arg(proposal_ref)
            .arg(&new_commit)
            .status()?;

        if !status.success() {
            return Err(io::Error::other(format!(
                "git update-ref failed for proposal_ref='{}'",
                proposal_ref
            )));
        }

        Ok(new_commit)
    })();

    let _ = std::fs::remove_file(&index_path);
    result
}

/// Fetch a named from from `source_ref` (remote or local) to `target_ref` (local only). This is
/// used to import proposal refs from remotes to local `refs/fed` namespace.
/// Returns the commit hash of the fetched ref.
pub(crate) fn fetch_named_ref(
    repo_path: &str,
    remote: &str,
    source_ref: &str,
    target_ref: &str,
) -> io::Result<String> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("fetch")
        .arg(remote)
        .arg(format!("{source_ref}:{target_ref}"))
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "git fetch failed for remote='{}' source_ref='{}' target_ref='{}'",
            remote, source_ref, target_ref
        )));
    }

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("rev-parse")
        .arg("--verify")
        .arg(target_ref)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git rev-parse failed for target_ref='{}': {}",
            target_ref,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn merge_proposal_into_main(repo_path: &str, proposal_ref: &str) -> io::Result<()> {
    let checkout = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("checkout")
        .arg("main")
        .status()?;

    if !checkout.success() {
        return Err(io::Error::other("git checkout main failed"));
    }

    let merge = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge")
        .arg("--no-ff")
        .arg(proposal_ref)
        .arg("-m")
        .arg(format!("Accept proposal {}", proposal_ref))
        .status()?;

    if !merge.success() {
        return Err(io::Error::other(format!(
            "git merge failed for proposal_ref='{}'",
            proposal_ref
        )));
    }

    Ok(())
}

pub(crate) fn fast_forward_proposal_into_main(
    repo_path: &str,
    proposal_ref: &str,
) -> io::Result<()> {
    let checkout = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("checkout")
        .arg("main")
        .status()?;

    if !checkout.success() {
        return Err(io::Error::other("git checkout main failed"));
    }

    let merge = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge")
        .arg("--ff-only")
        .arg(proposal_ref)
        .status()?;

    if !merge.success() {
        return Err(io::Error::other(format!(
            "git merge --ff-only failed for proposal_ref='{}'",
            proposal_ref
        )));
    }

    Ok(())
}

pub(crate) fn ref_diff_from_base(
    repo_path: &str,
    base_ref: &str,
    proposal_ref: &str,
) -> io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("diff")
        .arg(format!("{base_ref}..{proposal_ref}"))
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git diff failed for base_ref='{}' proposal_ref='{}'",
            base_ref, proposal_ref
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) fn push_main(repo_path: &str) -> io::Result<()> {
    let push = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("push")
        .arg("origin")
        .arg("main")
        .status()?;

    if !push.success() {
        return Err(io::Error::other("git push origin main failed"));
    }

    Ok(())
}

pub(crate) fn delete_ref(repo_path: &str, git_ref: &str) -> io::Result<()> {
    println!("Deleting git ref: {}", git_ref);
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("update-ref")
        .arg("-d")
        .arg(git_ref)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "git update-ref -d failed for '{}'",
            git_ref
        )));
    }

    Ok(())
}

pub(crate) fn move_ref(repo_path: &str, old_ref: &str, new_ref: &str) -> io::Result<()> {
    println!("Moving git ref from {} to {}", old_ref, new_ref);
    let old_hash = get_commit_hash(repo_path, old_ref)?;

    let update = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("update-ref")
        .arg(new_ref)
        .arg(&old_hash)
        .status()?;

    if !update.success() {
        return Err(io::Error::other(format!(
            "git update-ref failed for creating '{}' from '{}'",
            new_ref, old_hash
        )));
    }

    delete_ref(repo_path, old_ref)
}

pub(crate) fn is_ancestor(
    repo_path: &str,
    ancestor_commit: &str,
    child_commit: &str,
) -> io::Result<bool> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(ancestor_commit)
        .arg(child_commit)
        .output()?;

    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(io::Error::other(format!(
            "git merge-base --is-ancestor failed for ancestor_commit='{}' child_commit='{}': {}",
            ancestor_commit,
            child_commit,
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Check if merging `git_ref` into main would cause conflicts. If so, return the merge tree output
/// which includes conflict markers.
/// If no conflicts, return empty `BTreeSet`
pub(crate) fn get_conflicted_files(repo_path: &str, git_ref: &str) -> io::Result<BTreeSet<String>> {
    let base = get_merge_base(repo_path, "main", git_ref)?;

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge-tree")
        .arg(&base)
        .arg("main")
        .arg(git_ref)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other("git merge-tree failed"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(parse_conflicted_files_from_merge_tree(&stdout))
}

/// Parse this output:
/// # Conflict example 1
/// ```text
/// % git merge-tree 6a8d51d1067caab7d398579933047deabe90aee3 main refs/fed/proposals/site-1/site-2/00004
/// changed in both
///   base   100644 994c2dd97e9cb700c4366e96c016c938600c141c stats.txt
///   our    100644 4cad86ccd28cada46bcfd358d8d0ab157996bcc0 stats.txt
///   their  100644 17f68069e0e603a1d7c3d9714b7605fc947f5af6 stats.txt
/// @@ -1,2 +1,6 @@
///  site-1 initial state
/// +<<<<<<< .our
///  conflicting change from site-1 for site-1
/// +=======
/// +new change from site-2 for site-1
/// +>>>>>>> .their
/// ```
///
/// # Conflict example 2
/// ```text
/// % git merge-tree 487dc880071f68887e253e93301654f74d735147 main refs/fed/proposals/site-2/site-3/00001
/// added in both
///   our    100644 bc4f71af1fcbf1615f63c44afab3d9cbc8b786ae new-file.txt
///   their  100644 941f4cf069138bda16b2381894680692764a7fb8 new-file.txt
/// @@ -1 +1,5 @@
/// +<<<<<<< .our
///  site-2
/// +=======
/// +site-3
/// +>>>>>>> .their
/// ```
///
/// # Example no conflict
/// ```text
/// % git merge-tree 3c128e52b0b1da723714ee1d000eaf7346d9580f main refs/fed/proposals/site-3/site-2/00001
/// merged
///   result 100644 20c7e9745286276be85aaad4e4eb8629905d057d stats.txt
///   our    100644 3118dce45e5e54bd8dc3a13344d64e8b3148a8cf stats.txt
/// @@ -1 +1,2 @@
///  site-3 initial state
/// +changes from site-2
/// ```
pub(crate) fn parse_conflicted_files_from_merge_tree(output: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    let mut in_conflict_section = false;

    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line == "merged" {
            return BTreeSet::new();
        }
        if matches!(
            line,
            "changed in both" | "added in both" | "removed in both"
        ) {
            in_conflict_section = true;
            continue;
        }

        // A blank line usually means the section ended.
        if line.is_empty() {
            in_conflict_section = false;
            continue;
        }

        if in_conflict_section
            && (line.starts_with("base") || line.starts_with("our") || line.starts_with("their"))
            && let Some(path) = line.split_whitespace().last()
            && !path.is_empty()
        {
            files.insert(path.to_string());
        }
    }

    files
}

pub(crate) fn get_remotes(repo_path: &str) -> io::Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("remote")
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git -C remote {repo_path} failed; Error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let remotes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim())
        .filter(|&line| !line.is_empty())
        .filter(|&line| line != "origin")
        .map(|s| s.to_owned())
        .collect();
    Ok(remotes)
}

/// Fetch _all_ updates from a remote. This is best effort, if fetch fails, we will try again on
/// next update.
pub(crate) fn fetch_remote(repo_path: &str, remote: &str) -> io::Result<()> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("fetch")
        .arg("--prune")
        .arg(remote)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "git fetch -C {repo_path} {remote} failed"
        )));
    }

    Ok(())
}

pub(crate) fn fetch_remotes(repo_path: &str, remotes: &[impl AsRef<str>]) -> io::Result<()> {
    let mut errors = Vec::new();
    for remote in remotes {
        let remote = remote.as_ref();
        if let Err(err) = fetch_remote(repo_path, remote) {
            errors.push(format!("{remote}: {err}"));
        }
    }

    if !errors.is_empty() {
        let err_string = errors.join("\n");
        return Err(io::Error::other(err_string));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{parse_conflicted_files_from_merge_tree, read_file_at_ref};

    #[test]
    fn parse_conflicted_files_returns_empty_for_merged_output() {
        let output = "\
merged
  result 100644 20c7e9745286276be85aaad4e4eb8629905d057d stats.txt
  our    100644 3118dce45e5e54bd8dc3a13344d64e8b3148a8cf stats.txt
@@ -1 +1,2 @@
 site-3 initial state
+changes from site-2
";

        assert_eq!(
            parse_conflicted_files_from_merge_tree(output),
            BTreeSet::new()
        );
    }

    #[test]
    fn parse_conflicted_files_collects_paths_from_conflict_output() {
        let output = "\
changed in both
  base   100644 994c2dd97e9cb700c4366e96c016c938600c141c stats.txt
  our    100644 4cad86ccd28cada46bcfd358d8d0ab157996bcc0 stats.txt
  their  100644 17f68069e0e603a1d7c3d9714b7605fc947f5af6 stats.txt

added in both
  our    100644 bc4f71af1fcbf1615f63c44afab3d9cbc8b786ae new-file.txt
  their  100644 941f4cf069138bda16b2381894680692764a7fb8 new-file.txt
";

        assert_eq!(
            parse_conflicted_files_from_merge_tree(output),
            BTreeSet::from(["new-file.txt".to_string(), "stats.txt".to_string()])
        );
    }

    #[test]
    fn read_file_at_ref_returns_contents_for_existing_file_and_none_for_missing_file() {
        let temp_dir = tempdir().unwrap();
        let repo_path = temp_dir.path();

        let init = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("init")
            .output()
            .unwrap();
        assert!(init.status.success());

        let config_name = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("config")
            .arg("user.name")
            .arg("Test User")
            .output()
            .unwrap();
        assert!(config_name.status.success());

        let config_email = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("config")
            .arg("user.email")
            .arg("test@example.com")
            .output()
            .unwrap();
        assert!(config_email.status.success());

        std::fs::write(repo_path.join("conflict.txt"), "hello from main\n").unwrap();
        let add = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("add")
            .arg("conflict.txt")
            .output()
            .unwrap();
        assert!(add.status.success());

        let commit = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("commit")
            .arg("-m")
            .arg("initial")
            .output()
            .unwrap();
        assert!(commit.status.success());

        let contents =
            read_file_at_ref(repo_path.to_str().unwrap(), "HEAD", "conflict.txt").unwrap();
        assert_eq!(contents, Some("hello from main\n".to_string()));

        let missing = read_file_at_ref(repo_path.to_str().unwrap(), "HEAD", "missing.txt").unwrap();
        assert_eq!(missing, None);
    }
}

/// Add a remote.
pub(crate) fn add_remote(repo_path: &str, remote_name: &str, remote_url: &str) -> io::Result<()> {
    debug!("adding remote:  remote_name: {remote_name}, remote_url: {remote_url}");

    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("remote")
        .arg("add")
        .arg(remote_name)
        .arg(remote_url)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "git -C {repo_path} remote add {remote_name} {remote_url} failed"
        )));
    }

    info!("remote added:  remote_name: {remote_name}, remote_url: {remote_url}");

    Ok(())
}

/// Checks if a git remote with the given name already exists in the repository.
pub(crate) fn remote_exists(repo_path: &str, remote_name: &str) -> Result<bool, GitError> {
    let repo = git2::Repository::open(repo_path)?;
    let remotes = repo.remotes()?;
    for remote in remotes.iter() {
        if remote == Some(remote_name) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Removes a git remote from the repository.
pub fn remove_remote(repo_path: &str, remote_name: &str) -> Result<(), GitError> {
    let repo = git2::Repository::open(repo_path)?;
    repo.remote_delete(remote_name)?;
    Ok(())
}

/// Removes a git remote if it exists. Returns whether a remote was removed.
pub(crate) fn remove_remote_if_exists(
    repo_path: &str,
    remote_name: &str,
) -> Result<bool, GitError> {
    if !remote_exists(repo_path, remote_name)? {
        return Ok(false);
    }

    remove_remote(repo_path, remote_name)?;
    Ok(true)
}

//! `kiwami host push` - record this machine's config in the flake, on GitHub.
//!
//! `kiwami install` scaffolds hosts/<name>/ and stages it, which is all a
//! flake needs in order to build from it. Nothing committed it, so the config
//! describing a machine's hardware and disk layout lived only on that machine
//! - and an ephemeral root leaves no accumulated state to reconstruct it from.
//! hosts/xps spent its first week exactly one dead SSD from being unrecoverable.
//!
//! The dangerous version of this command is `git push`. A working tree
//! collects unrelated commits, and a command that pushes HEAD sends whatever
//! happens to be sitting there. So the branch is not built from HEAD at all:
//! it is built in a throwaway worktree checked out at origin/main, with the
//! host directory copied in and nothing else. Unrelated work cannot ride along
//! because it is never in the tree that gets pushed - a property of how the
//! commit is made, not a check that could be wrong.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::paths;

pub fn push(name: Option<String>, no_pr: bool) -> Result<(), String> {
    let repo = paths::repo();
    if !repo.join(".git").exists() {
        return Err(format!("{} is not a git checkout", repo.display()));
    }

    let name = match name {
        Some(n) => n,
        None => hostname()?,
    };
    let rel = format!("hosts/{name}");
    let dir = repo.join(&rel);
    if !dir.is_dir() {
        return Err(format!(
            "{} does not exist.\nThis machine's config is scaffolded by `kiwami install`; \
             pass a name if it is called something else.",
            dir.display()
        ));
    }

    println!("==> fetching origin");
    git(&repo, &["fetch", "--quiet", "origin"])?;
    let base = base_ref(&repo)?;

    // A detached worktree at origin/main. The user's working tree, index and
    // branch are untouched throughout - nothing here checks anything out in
    // the repository itself.
    let work = std::env::temp_dir().join(format!("kiwami-host-{}", std::process::id()));
    let _ = git(&repo, &["worktree", "remove", "--force", &work.to_string_lossy()]);
    println!("==> building a branch from {base}, with {rel} and nothing else");
    git(&repo, &["worktree", "add", "--quiet", "--detach", &work.to_string_lossy(), &base])?;

    let result = build_and_push(&repo, &work, &name, &rel, &dir, &base, no_pr);

    // Always clean up: a leaked worktree makes the next run fail on a path
    // that already exists, which is a confusing way to learn about the first
    // failure.
    let _ = git(&repo, &["worktree", "remove", "--force", &work.to_string_lossy()]);
    result
}

#[allow(clippy::too_many_arguments)]
fn build_and_push(
    repo: &Path,
    work: &Path,
    name: &str,
    rel: &str,
    src: &Path,
    base: &str,
    no_pr: bool,
) -> Result<(), String> {
    let dest = work.join(rel);
    fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    copy_dir(src, &dest)?;

    git(work, &["add", "--", rel])?;
    // Nothing staged means origin already has exactly these files.
    if git(work, &["diff", "--cached", "--quiet"]).is_ok() {
        println!("\n{rel} on {base} already has exactly this - nothing to push");
        return Ok(());
    }

    let branch = format!("host/{name}");

    // Fetch the host branch by name rather than trusting remote-tracking refs
    // to exist. `kiwami install` clones shallow and single-branch, so the
    // configured refspec is main only and refs/remotes/origin/host/<name> is
    // never created by a plain fetch - which left the check below unable to
    // fire and --force-with-lease with no lease to compare, failing the second
    // push with "stale info". The first test of this passed because it used an
    // ordinary clone, which is the easier machine again.
    let remote_branch = format!("origin/{branch}");
    let refspec = format!("+refs/heads/{branch}:refs/remotes/{remote_branch}");
    let on_remote = git(work, &["fetch", "--quiet", "origin", &refspec]).is_ok();

    // Asked before anything is reported as changed. The diff below is against
    // main, which legitimately does not carry this host yet - so printing it
    // and then "nothing to push" read as a contradiction.
    //
    // Scoped to the host directory: comparing whole trees calls the host
    // changed as soon as main gains any commit, because the branch was built
    // on an older main.
    if on_remote && git(work, &["diff", "--cached", "--quiet", &remote_branch, "--", rel]).is_ok() {
        println!("\n{branch} already carries exactly this - nothing to push");
        println!("    (not on {base} yet: the pull request is open)");
        return Ok(());
    }

    println!("\n    to push:");
    let stat = git(work, &["diff", "--cached", "--stat"])?;
    for line in stat.lines() {
        println!("    {line}");
    }

    let subject = format!("hosts: {name}");
    let body = format!(
        "Written by `kiwami install` on the machine itself and pushed with \
         `kiwami host push`.\n\n{}",
        facts(src)
    );
    git(work, &["commit", "--quiet", "-m", &subject, "-m", &body])?;

    println!("\n==> pushing {branch}");
    // The branch is regenerated from origin/main every time, so it is expected
    // to be replaced rather than appended to. --force-with-lease still refuses
    // if someone else moved it since the fetch above - but only means anything
    // when there is a remote-tracking ref to lease against, so a branch that
    // does not exist yet is pushed plainly. That still refuses a non-fast
    // -forward, so nothing is overwritten unseen either way.
    let dest = format!("HEAD:refs/heads/{branch}");
    // The lease carries the commit that was just fetched, rather than relying
    // on git to find it: the bare --force-with-lease form reads the
    // remote-tracking ref and its reflog, neither of which a shallow
    // single-branch clone maintains for a branch it was never configured to
    // fetch. It rejected every second push with "stale info".
    let lease = if on_remote {
        git(work, &["rev-parse", &remote_branch]).ok().map(|sha| {
            format!("--force-with-lease=refs/heads/{branch}:{}", sha.trim())
        })
    } else {
        None
    };
    let mut args = vec!["push"];
    if let Some(l) = &lease {
        args.push(l);
    }
    args.extend_from_slice(&["origin", &dest]);
    git(work, &args)?;

    if no_pr {
        println!("\npushed. Open a pull request when you want it on {base}.");
        return Ok(());
    }
    open_pr(repo, &branch, &subject, &body, base)
}

/// A pull request, so the diff gets looked at before it lands on main. Falls
/// back to printing the URL: gh not being logged in should not lose the push
/// that already succeeded.
fn open_pr(repo: &Path, branch: &str, title: &str, body: &str, base: &str) -> Result<(), String> {
    let target = base.strip_prefix("origin/").unwrap_or(base);

    if !gh_ready() {
        println!("\npushed, but gh is not authenticated - open it here:");
        println!("    {}", compare_url(repo, branch).unwrap_or_else(|| "(unknown remote)".into()));
        println!("\nOr run `kiwami auth --login` and try again.");
        return Ok(());
    }

    println!("==> opening a pull request");
    let out = Command::new("gh")
        .current_dir(repo)
        .args(["pr", "create", "--head", branch, "--base", target, "--title", title, "--body", body])
        .output()
        .map_err(|e| format!("gh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();

    if out.status.success() {
        println!("    {text}");
        return Ok(());
    }
    // Most often: a pull request for this branch is already open, which is not
    // a failure - the push updated it.
    println!("    {err}");
    if let Some(url) = compare_url(repo, branch) {
        println!("    {url}");
    }
    Ok(())
}

/// What the machine is, so the diff can be reviewed without going to look.
fn facts(dir: &Path) -> String {
    let mut out = Vec::new();
    if let Some(model) = read_trim("/sys/class/dmi/id/product_name") {
        out.push(format!("- machine: {model}"));
    }
    out.push(format!("- arch: {}", std::env::consts::ARCH));
    if let Ok(disk) = fs::read_to_string(dir.join("disk.nix")) {
        if let Some(line) = disk.lines().find(|l| l.contains("/dev/disk/by-id/")) {
            if let Some(dev) = line.split('"').nth(1) {
                out.push(format!("- disk: {dev}"));
            }
        }
    }
    out.join("\n")
}

fn read_trim(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// origin/main, or origin/master on a repository that predates the rename.
/// Whether hosts/<name> in this checkout differs from what origin already
/// carries - the question the installer needs answered before it claims the
/// host is unpushed.
///
/// Three ways to differ, and the first is the one that is easy to miss: a
/// brand new host is untracked, so `git diff` reports nothing about it and a
/// naive check calls it identical.
pub fn differs_from_origin(repo: &Path, name: &str) -> Result<bool, String> {
    let rel = format!("hosts/{name}");
    git(repo, &["fetch", "--quiet", "origin"])?;
    let base = base_ref(repo)?;

    if git(repo, &["ls-files", "--", &rel])?.trim().is_empty() {
        return Ok(true); // untracked: origin has never seen it
    }
    if !git(repo, &["ls-files", "--others", "--exclude-standard", "--", &rel])?
        .trim()
        .is_empty()
    {
        return Ok(true); // a new file inside a tracked host
    }
    Ok(git(repo, &["diff", "--quiet", &base, "--", &rel]).is_err())
}

fn base_ref(repo: &Path) -> Result<String, String> {
    for candidate in ["origin/main", "origin/master"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err("neither origin/main nor origin/master exists".into())
}

fn hostname() -> Result<String, String> {
    read_trim("/etc/hostname").ok_or_else(|| "cannot read /etc/hostname".into())
}

fn gh_ready() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn compare_url(repo: &Path, branch: &str) -> Option<String> {
    let remote = git(repo, &["remote", "get-url", "origin"]).ok()?;
    let slug = github_slug(remote.trim())?;
    Some(format!("https://github.com/{slug}/compare/{branch}?expand=1"))
}

/// owner/repo from either remote form. Kept separate so it can be tested
/// without a repository.
fn github_slug(url: &str) -> Option<String> {
    let rest = if let Some(r) = url.strip_prefix("git@github.com:") {
        r
    } else if let Some(r) = url.strip_prefix("https://github.com/") {
        r
    } else if let Some(r) = url.strip_prefix("ssh://git@github.com/") {
        r
    } else {
        return None;
    };
    let slug = rest.trim_end_matches('/').trim_end_matches(".git");
    if slug.split('/').filter(|s| !s.is_empty()).count() == 2 {
        Some(slug.to_string())
    } else {
        None
    }
}

fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    for entry in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if entry.path().is_dir() {
            fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_from_both_remote_forms() {
        let want = Some("kiwamios/kiwami".to_string());
        assert_eq!(github_slug("git@github.com:kiwamios/kiwami.git"), want);
        assert_eq!(github_slug("https://github.com/kiwamios/kiwami.git"), want);
        assert_eq!(github_slug("https://github.com/kiwamios/kiwami"), want);
        assert_eq!(github_slug("ssh://git@github.com/kiwamios/kiwami.git"), want);
    }

    #[test]
    fn slug_declines_what_it_cannot_parse() {
        // A self-hosted remote gets no GitHub URL rather than a wrong one.
        assert_eq!(github_slug("git@gitlab.com:kiwamios/kiwami.git"), None);
        assert_eq!(github_slug("/srv/git/kiwami.git"), None);
        assert_eq!(github_slug("https://github.com/jimzer"), None);
    }
}

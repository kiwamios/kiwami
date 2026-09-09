//! `kiwami persist` - what this machine will lose at the next boot.
//!
//! Not a check. `kiwami doctor` used to carry one that walked the whole root
//! and subtracted a list of paths it believed were regenerated, then reported
//! how many files were "at risk". The question has no good answer: on an
//! ephemeral root *everything* undeclared is lost, which is the design rather
//! than a finding. It reported 4 paths one hour and 9015 the next, tracking
//! how recently nix had built something, and the entries at the top were nix
//! caches - regenerable by definition and supposed to be lost. It never once
//! found a real problem.
//!
//! The mistake was answering a question nobody asked. What is actually useful
//! is looking, deliberately, at what is about to be destroyed - and being able
//! to choose how closely. So this shows exactly that, at whatever depth you
//! ask for, and judges nothing.
//!
//! Depth 0 is the shape of the loss; deeper is where you find the service that
//! needs declaring.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub fn lost(depth: usize, show_size: bool, under: Option<String>) -> Result<(), String> {
    if !Path::new("/persist").is_dir() {
        println!("not an ephemeral-root machine - nothing is wiped at boot");
        return Ok(());
    }

    // Narrowing is a path, not a list of things we think are boring. The
    // check this replaced tried to guess which losses mattered and got it
    // wrong in both directions; here the person looking says where to look.
    let start = under.clone().unwrap_or_else(|| "/".to_string());
    let start = Path::new(&start);
    if !start.is_dir() {
        return Err(format!("{} is not a directory", start.display()));
    }

    let root_dev = fs::metadata("/").map_err(|e| format!("/: {e}"))?.dev();
    let mut found: BTreeMap<String, u64> = BTreeMap::new();
    walk(start, 0, depth, root_dev, &mut found);

    if found.is_empty() {
        println!("nothing under {} that a reboot destroys.", start.display());
        return Ok(());
    }

    println!("These are destroyed at the next boot. Anything worth keeping");
    println!("belongs in kiwami.persist.directories.\n");

    for (path, size) in &found {
        if show_size {
            println!("  {:>9}  {path}", human(*size));
        } else {
            println!("  {path}");
        }
    }

    println!("\n{} path(s) at depth {depth}.", found.len());
    let scope = under
        .as_ref()
        .map(|u| format!(" --under {u}"))
        .unwrap_or_default();
    if depth < 8 {
        println!("Deeper:  kiwami persist lost --depth {}{scope}", depth + 1);
    }
    Ok(())
}

/// Everything on the root filesystem, to a given depth.
///
/// Two things are skipped, and neither is a judgement about whether the
/// contents matter:
///
///   - anything on another device. A persisted path is a bind mount out of
///     /persist, so it is not on the root and is not lost. So are /nix, /boot
///     and the pseudo-filesystems. This is what makes "declared" and "safe"
///     the same test rather than two lists that can disagree.
///   - symlinks, which on NixOS point into the store and are rebuilt by
///     activation.
fn walk(
    dir: &Path,
    level: usize,
    max: usize,
    root_dev: u64,
    out: &mut BTreeMap<String, u64>,
) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };

        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.dev() != root_dev {
            continue;
        }

        if level >= max || !meta.is_dir() {
            // Measured always, because the size is what decides whether it is
            // listed: an empty directory loses nothing, and 31 of the 57
            // entries on the first real run were empty. More than half the
            // output was places with nothing in them.
            let size = bytes_under(&path, 0);
            if size > 0 {
                out.insert(path.to_string_lossy().into_owned(), size);
            }
        } else {
            walk(&path, level + 1, max, root_dev, out);
        }
    }
}

/// Size on disk, bounded.
///
/// Stops at a depth rather than walking a whole tree: this is here to tell
/// you which of these is worth caring about, and a number that takes a minute
/// to produce is a number nobody waits for.
fn bytes_under(path: &Path, depth: usize) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if !meta.is_dir() {
        return meta.len();
    }
    if depth > 8 {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| bytes_under(&e.path(), depth + 1))
        .sum()
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// The paths this machine has been told to keep, as they appear on the root.
///
/// Shared by `declared` and `orphans`, because the second is only useful if it
/// asks exactly the same question as the first and subtracts the answer.
fn declared_paths() -> Result<Vec<PathBuf>, String> {
    let raw = fs::read_to_string("/etc/kiwami/persist.json")
        .map_err(|e| format!("/etc/kiwami/persist.json: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("persist.json: {e}"))?;

    let user = v.get("user").and_then(|u| u.as_str()).unwrap_or("");
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };

    let mut paths: Vec<PathBuf> = list("directories").iter().map(PathBuf::from).collect();
    paths.extend(list("files").iter().map(PathBuf::from));
    if !user.is_empty() {
        let home = PathBuf::from("/home").join(user);
        paths.extend(list("userDirectories").iter().map(|d| home.join(d)));
        paths.extend(list("userFiles").iter().map(|f| home.join(f)));
    }
    paths.sort();
    Ok(paths)
}

/// What this machine has been told to keep.
pub fn declared() -> Result<(), String> {
    let paths = declared_paths()?;

    println!("Declared, and so surviving a boot:\n");
    for p in &paths {
        // Declared is not the same as present: a path can be listed and not
        // yet exist, which is normal on a machine that has not run the
        // service yet - and worth seeing rather than hiding.
        let mark = if p.exists() { " " } else { "?" };
        println!("  {mark} {}", p.display());
    }
    println!("\n{} declared. ? means nothing is there yet.", paths.len());
    Ok(())
}

/// What /persist holds that nothing asks for any more.
///
/// impermanence never deletes. It bind-mounts the declared paths out of
/// /persist over a wiped root, and that is the whole of what it does - so
/// removing a line from kiwami.persist removes the *mount*, not the bytes.
/// The data stays where it was, unreachable, because nothing mounts it any
/// more; `ls` on the old location shows an empty directory and the disk keeps
/// the copy.
///
/// That default is right - a typo in a config should never delete a card
/// collection - but it is silent in both directions. Nothing says the data
/// stayed, and nothing says it is still in every backup, because the backup
/// takes /persist whole. Dropping one entry left 131M here, which was 98% of
/// the /persist on that machine and had been going to R2 for weeks.
///
/// So this is the third question, and the only one that finds a problem:
/// `lost` is what dies at the next boot, `declared` is what is kept, and this
/// is what is kept that nobody asked for.
pub fn orphans(show_size: bool) -> Result<(), String> {
    if !Path::new("/persist").is_dir() {
        println!("not an ephemeral-root machine - nothing is bind-mounted out of /persist");
        return Ok(());
    }

    let declared = declared_paths()?;
    let mut found: Vec<(String, u64)> = Vec::new();
    walk_persist(Path::new("/persist"), &declared, &mut found);

    if found.is_empty() {
        println!("Nothing in /persist that is not declared.");
        return Ok(());
    }

    found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total: u64 = found.iter().map(|(_, n)| *n).sum();

    println!("In /persist, and declared by nothing:\n");
    for (path, size) in &found {
        if show_size {
            println!("  {:>9}  {path}", human(*size));
        } else {
            println!("  {path}");
        }
    }

    println!("\n{} path(s), {} in total.", found.len(), human(total));
    println!("These survive every boot and are in every backup. Nothing reads");
    println!("them. Deleting one is `sudo rm -rf` on the path as printed -");
    println!("check what it was before you do, because this cannot tell you.");
    Ok(())
}

/// Walk /persist, reporting what no declaration covers.
///
/// Three cases at each entry, and the middle one is the reason this is not a
/// flat listing: a directory can be undeclared itself and still hold the
/// declared path. /persist/home/kiwami/.local/share is nobody's declaration,
/// but Anki2 lives inside it, so it has to be descended into rather than
/// reported.
fn walk_persist(dir: &Path, declared: &[PathBuf], out: &mut Vec<(String, u64)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();

        // The path this would be on the root, which is what a declaration
        // names: /persist/home/x/.ssh is a declaration of /home/x/.ssh.
        let Ok(rel) = path.strip_prefix("/persist") else { continue };
        let on_root = Path::new("/").join(rel);

        if declared.iter().any(|d| on_root.starts_with(d)) {
            // Declared, or inside something declared.
            continue;
        }

        if declared.iter().any(|d| d.starts_with(&on_root)) {
            // Holds a declaration further down.
            walk_persist(&path, declared, out);
            continue;
        }

        // Reported as the whole subtree rather than descended into: the thing
        // worth seeing is "nvim, 131M", not four thousand plugin files.
        let size = bytes_under(&path, 0);
        out.push((path.to_string_lossy().into_owned(), size));
    }
}

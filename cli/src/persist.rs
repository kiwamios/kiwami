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

pub fn lost(depth: usize, show_size: bool) -> Result<(), String> {
    if !Path::new("/persist").is_dir() {
        println!("not an ephemeral-root machine - nothing is wiped at boot");
        return Ok(());
    }

    let root_dev = fs::metadata("/").map_err(|e| format!("/: {e}"))?.dev();
    let mut found: BTreeMap<String, u64> = BTreeMap::new();
    walk(Path::new("/"), 0, depth, root_dev, show_size, &mut found);

    if found.is_empty() {
        println!("nothing on the root but what was declared.");
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
    if depth < 6 {
        println!("Deeper:  kiwami persist --depth {}", depth + 1);
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
    show_size: bool,
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
            let size = if show_size { bytes_under(&path, 0) } else { 0 };
            out.insert(path.to_string_lossy().into_owned(), size);
        } else {
            walk(&path, level + 1, max, root_dev, show_size, out);
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

/// What this machine has been told to keep.
pub fn declared() -> Result<(), String> {
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

//! `kiwami wallpaper` - the pictures, and which one is showing.
//!
//! Deliberately touches no Nix. Editing the flake to change a wallpaper would
//! need a checkout to edit, a push, and a rebuild - and this machine keeps no
//! checkout on purpose. So the split is:
//!
//!   content    files in a directory        add, remove
//!   selection  one line in a state file    set, next
//!   policy     kiwami.wallpaper.*          rotate, interval - a rebuild
//!
//! Only the third is configuration, and it is the one you change twice a
//! year. The two you touch daily are files, and files need no rebuild.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = "/etc/kiwami/wallpaper.json";

/// Formats this machine's Qt can decode.
///
/// Checked when adding rather than when displaying: a format Qt cannot read
/// fails silently at paint time, and the only symptom is a wallpaper that did
/// not change. Notably absent is webp - the plugin is not in the closure, so
/// accepting one would mean saving a file that can never be shown.
const READABLE: [&str; 6] = ["png", "jpg", "jpeg", "svg", "gif", "bmp"];

struct Paths {
    directory: PathBuf,
    state: PathBuf,
}

fn paths() -> Result<Paths, String> {
    let raw = fs::read_to_string(MANIFEST)
        .map_err(|e| format!("cannot read {MANIFEST}: {e}\nThis system was not built by Kiwami."))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let get = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| format!("{MANIFEST} has no {k}"))
    };
    Ok(Paths { directory: get("directory")?, state: get("state")? })
}

/// The images, sorted - the same order the shell uses, because "the first
/// one" has to mean the same thing in both places.
fn images(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && readable(p))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn readable(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| READABLE.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn current(st: &Path) -> Option<String> {
    fs::read_to_string(st).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn list() -> Result<(), String> {
    let p = paths()?;
    let all = images(&p.directory);
    if all.is_empty() {
        println!("no wallpapers in {}", p.directory.display());
        println!("add one:  kiwami wallpaper add <url or path>");
        return Ok(());
    }
    let now = current(&p.state);
    for img in &all {
        let n = name(img);
        let mark = if Some(&n) == now.as_ref() { "*" } else { " " };
        let size = fs::metadata(img).map(|m| m.len() / 1024).unwrap_or(0);
        println!("{mark} {n}  ({size} KB)");
    }
    if now.is_none() {
        println!("\nnothing chosen - showing the first one");
    }
    Ok(())
}

/// Fetch or copy an image in.
///
/// A URL is downloaded, a path copied. Either way the result is checked for
/// being an image this machine can actually display before it is kept, since
/// the alternative is a file that sits in the directory forever and silently
/// never appears.
pub fn add(source: String, set_it: bool) -> Result<(), String> {
    let p = paths()?;
    fs::create_dir_all(&p.directory)
        .map_err(|e| format!("{}: {e}", p.directory.display()))?;

    let is_url = source.starts_with("http://") || source.starts_with("https://");
    let base = pick_name(&source, is_url);
    let dest = free_path(&p.directory, &base);

    if is_url {
        println!("==> fetching {source}");
        let out = Command::new("curl")
            .args(["-fsSL", "-o", &dest.to_string_lossy(), &source])
            .status()
            .map_err(|e| format!("curl: {e}"))?;
        if !out.success() {
            let _ = fs::remove_file(&dest);
            return Err(format!("could not download {source}"));
        }
    } else {
        let src = PathBuf::from(shellexpand(&source));
        if !src.is_file() {
            return Err(format!("{} is not a file", src.display()));
        }
        fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }

    if let Err(why) = looks_like_an_image(&dest) {
        let _ = fs::remove_file(&dest);
        return Err(why);
    }

    let n = name(&dest);
    let size = fs::metadata(&dest).map(|m| m.len() / 1024).unwrap_or(0);
    println!("    {n}  ({size} KB)");

    if set_it {
        set(n)?;
    } else {
        println!("\n    show it now:  kiwami wallpaper set {n}");
    }
    Ok(())
}

/// A name from a URL or path, falling back to something ordered.
///
/// Rotation is alphabetical, so a name matters more than it looks: files
/// called wallpaper.jpg, wallpaper(1).jpg would rotate in an order nobody
/// chose.
fn pick_name(source: &str, is_url: bool) -> String {
    let tail = source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("wallpaper")
        .to_string();

    let looks_named = tail.contains('.') && readable(Path::new(&tail));
    if looks_named {
        return tail;
    }
    // A URL with no filename - picsum, unsplash - gets a timestamp, which at
    // least sorts by when you added it.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if is_url { format!("{stamp}.jpg") } else { format!("{stamp}-{tail}") }
}

/// Never overwrite an image that is already there.
fn free_path(dir: &Path, base: &str) -> PathBuf {
    let candidate = dir.join(base);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = base.rsplit_once('.').unwrap_or((base, ""));
    for n in 2..1000 {
        let p = dir.join(format!("{stem}-{n}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    candidate
}

/// Is this actually an image, and one Qt here can read?
///
/// `file` rather than trusting the extension: a download that 404'd into an
/// HTML error page is still called .jpg, and would sit in the directory
/// looking like a wallpaper that simply never shows up.
fn looks_like_an_image(p: &Path) -> Result<(), String> {
    if fs::metadata(p).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err("that produced an empty file".into());
    }
    let out = Command::new("file")
        .args(["-b", "--mime-type", &p.to_string_lossy()])
        .output()
        .map_err(|e| format!("file: {e}"))?;
    let mime = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if mime.starts_with("image/") || mime.contains("svg") || mime == "text/xml" {
        return Ok(());
    }
    Err(format!("that is {mime}, not an image"))
}

pub fn set(which: String) -> Result<(), String> {
    let p = paths()?;
    let all = images(&p.directory);
    if all.is_empty() {
        return Err(format!("no wallpapers in {}", p.directory.display()));
    }

    // Accepts a bare name, a partial one, or a path, because typing a full
    // filename to look at a picture is a silly thing to have to do.
    let want = Path::new(&which);
    let hit = all
        .iter()
        .find(|p| name(p) == which)
        .or_else(|| all.iter().find(|p| p.as_path() == want))
        .or_else(|| all.iter().find(|p| name(p).to_lowercase().contains(&which.to_lowercase())));

    let Some(hit) = hit else {
        return Err(format!(
            "no wallpaper matching {which}. What is there:\n{}",
            all.iter().map(|p| format!("  {}", name(p))).collect::<Vec<_>>().join("\n")
        ));
    };

    write_state(&p.state, &name(hit))?;
    println!("{}", name(hit));
    Ok(())
}

pub fn next() -> Result<(), String> {
    let p = paths()?;
    let all = images(&p.directory);
    if all.len() < 2 {
        return Err("need at least two wallpapers to move between them".into());
    }
    let now = current(&p.state);
    let at = now
        .and_then(|n| all.iter().position(|p| name(p) == n))
        .unwrap_or(0);
    let next = &all[(at + 1) % all.len()];
    write_state(&p.state, &name(next))?;
    println!("{}", name(next));
    Ok(())
}

pub fn remove(which: String) -> Result<(), String> {
    let p = paths()?;
    let all = images(&p.directory);
    let Some(hit) = all.iter().find(|p| name(p) == which) else {
        return Err(format!("no wallpaper called {which}"));
    };
    fs::remove_file(hit).map_err(|e| format!("{}: {e}", hit.display()))?;
    println!("removed {which}");

    // The shell would show the fallback for a moment and then rotate on. Say
    // so rather than leaving it to be noticed.
    if current(&p.state).as_deref() == Some(which.as_str()) {
        if let Some(first) = images(&p.directory).first() {
            write_state(&p.state, &name(first))?;
            println!("that was the one showing; now {}", name(first));
        }
    }
    Ok(())
}

/// Written atomically, because the shell is watching this file and a
/// half-written name is a wallpaper that does not exist.
fn write_state(state: &Path, name: &str) -> Result<(), String> {
    if let Some(dir) = state.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = state.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        writeln!(f, "{name}").map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, state).map_err(|e| format!("{}: {e}", state.display()))
}

fn shellexpand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => p.to_string(),
        },
        None => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_with_a_filename_keeps_it() {
        assert_eq!(pick_name("https://x.test/forest.jpg", true), "forest.jpg");
        assert_eq!(pick_name("https://x.test/a/b/sea.png?w=10", true), "sea.png");
    }

    /// picsum and friends serve an image from a path with no filename. The
    /// name still has to sort predictably, because rotation is alphabetical.
    #[test]
    fn a_url_without_one_gets_a_timestamp() {
        let n = pick_name("https://picsum.photos/seed/x/2560/1600", true);
        assert!(n.ends_with(".jpg"), "{n}");
        assert!(n.trim_end_matches(".jpg").chars().all(|c| c.is_ascii_digit()), "{n}");
    }

    #[test]
    fn webp_is_not_accepted_because_qt_here_cannot_show_it() {
        assert!(!readable(Path::new("a.webp")));
        assert!(readable(Path::new("a.PNG")), "extensions are not case sensitive");
    }
}

//! `kiwami doctor` - find what drifted, and check that what is declared works.
//!
//! On NixOS packages are declared by construction, so this is not a package
//! diff. It looks for the escape hatches someone used, and for the failures
//! that are silent: a unit that restarts forever still shows up in pgrep, a
//! target that never activates just means nothing bound to it ever starts.

use std::fmt;
use std::fs;
use std::process::Command;

use crate::paths;

#[derive(PartialEq, Clone, Copy)]
pub enum Level {
    Ok,
    Warn,
    Fail,
    /// Not applicable here - e.g. a session check run over SSH. Skipped is not
    /// a failure; reporting it as one trains people to ignore the output.
    Skip,
}

pub struct Finding {
    level: Level,
    title: String,
    detail: Option<String>,
    remedy: Option<String>,
}

impl Finding {
    fn new(level: Level, title: impl Into<String>) -> Self {
        Finding { level, title: title.into(), detail: None, remedy: None }
    }
    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    fn remedy(mut self, r: impl Into<String>) -> Self {
        self.remedy = Some(r.into());
        self
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mark, colour) = match self.level {
            Level::Ok => ("✔", "\x1b[32m"),
            Level::Warn => ("⚠", "\x1b[33m"),
            Level::Fail => ("✘", "\x1b[31m"),
            Level::Skip => ("–", "\x1b[90m"),
        };
        write!(f, "  {colour}{mark}\x1b[0m {}", self.title)?;
        if let Some(d) = &self.detail {
            for line in d.lines() {
                write!(f, "\n      \x1b[90m{line}\x1b[0m")?;
            }
        }
        if let Some(r) = &self.remedy {
            write!(f, "\n      \x1b[36m→ {r}\x1b[0m")?;
        }
        Ok(())
    }
}

/// Run a command, returning stdout on success. None if it is not installed or
/// failed - a missing tool is not a finding, it is simply nothing to check.
fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn have(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn user_systemctl(args: &[&str]) -> Option<String> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    let out = Command::new("systemctl")
        .args(&full)
        .env("XDG_RUNTIME_DIR", format!("/run/user/{}", uid()))
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn uid() -> u32 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()
        })
        .unwrap_or(1000)
}

// --- drift ---------------------------------------------------------------

fn nix_profile() -> Finding {
    match output("nix", &["profile", "list"]) {
        None => Finding::new(Level::Skip, "nix profile unavailable"),
        Some(out) if out.trim().is_empty() => {
            Finding::new(Level::Ok, "no imperative nix profile installs")
        }
        Some(out) => {
            let n = out.lines().filter(|l| !l.trim().is_empty()).count();
            Finding::new(Level::Fail, format!("{n} package(s) installed imperatively"))
                .detail(out.trim().to_string())
                .remedy("declare them in the flake, then: nix profile remove --all")
        }
    }
}

fn nix_env() -> Finding {
    match output("nix-env", &["-q"]) {
        None => Finding::new(Level::Skip, "nix-env unavailable"),
        Some(out) if out.trim().is_empty() => Finding::new(Level::Ok, "nix-env profile empty"),
        Some(out) => Finding::new(Level::Fail, "packages installed with nix-env")
            .detail(out.trim().to_string())
            .remedy("nix-env -e '*' once they are in the flake"),
    }
}

fn stray_binaries() -> Finding {
    // The invoking user's home, not root's: doctor runs under sudo.
    let home = paths::user_home();
    let dirs = [home.join(".local/bin"), home.join("bin")];
    let mut found = Vec::new();

    for dir in dirs.iter().filter(|d| d.is_dir()) {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            // A symlink into the store is Nix-managed; a real file here is not.
            if p.is_symlink() {
                if let Ok(t) = fs::read_link(&p) {
                    if t.starts_with("/nix/store") {
                        continue;
                    }
                }
            }
            found.push(format!("{}", p.display()));
        }
    }

    if found.is_empty() {
        Finding::new(Level::Ok, "no stray executables in ~/.local/bin or ~/bin")
    } else {
        Finding::new(Level::Fail, format!("{} executable(s) not managed by nix", found.len()))
            .detail(found.join("\n"))
            .remedy("package them in the flake, or delete them")
    }
}

fn language_managers() -> Vec<Finding> {
    let checks: [(&str, &[&str], &str); 3] = [
        ("npm", &["ls", "-g", "--depth=0", "--parseable"], "npm -g"),
        ("cargo", &["install", "--list"], "cargo install"),
        ("pipx", &["list", "--short"], "pipx"),
    ];

    checks
        .iter()
        .filter(|(bin, _, _)| have(bin))
        .map(|(bin, args, label)| match output(bin, args) {
            Some(out) => {
                // npm always prints its own prefix line; ignore it.
                let items: Vec<&str> = out
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.ends_with("/lib"))
                    .collect();
                if items.is_empty() {
                    Finding::new(Level::Ok, format!("no {label} installs"))
                } else {
                    Finding::new(Level::Warn, format!("{} {label} install(s)", items.len()))
                        .detail(items.join("\n"))
                        .remedy("fine for throwaway tools; package anything you rely on")
                }
            }
            None => Finding::new(Level::Skip, format!("{label} not queryable")),
        })
        .collect()
}

// --- health --------------------------------------------------------------

fn failed_units() -> Finding {
    match output("systemctl", &["--failed", "--no-legend", "--plain"]) {
        None => Finding::new(Level::Skip, "systemctl unavailable"),
        Some(out) if out.trim().is_empty() => Finding::new(Level::Ok, "no failed system units"),
        Some(out) => Finding::new(Level::Fail, "failed system units")
            .detail(out.trim().to_string())
            .remedy("systemctl status <unit>"),
    }
}

fn failed_user_units() -> Finding {
    match user_systemctl(&["--failed", "--no-legend", "--plain"]) {
        None => Finding::new(Level::Skip, "user manager unavailable"),
        Some(out) if out.trim().is_empty() => Finding::new(Level::Ok, "no failed user units"),
        Some(out) => Finding::new(Level::Fail, "failed user units")
            .detail(out.trim().to_string())
            .remedy("systemctl --user status <unit>"),
    }
}

fn graphical_session() -> Finding {
    let active = user_systemctl(&["is-active", "graphical-session.target"])
        .map(|s| s.trim() == "active")
        .unwrap_or(false);

    if active {
        Finding::new(Level::Ok, "graphical-session.target active")
    } else if std::env::var("WAYLAND_DISPLAY").is_err() && !session_env_has_wayland() {
        // Run over SSH with no desktop: nothing is wrong.
        Finding::new(Level::Skip, "no graphical session here")
    } else {
        Finding::new(Level::Fail, "graphical-session.target inactive")
            .detail("units bound to it never start, silently")
            .remedy("check that the session launches via uwsm")
    }
}

fn session_env_has_wayland() -> bool {
    user_systemctl(&["show-environment"])
        .map(|s| s.contains("WAYLAND_DISPLAY="))
        .unwrap_or(false)
}

fn shell_unit() -> Finding {
    let state = user_systemctl(&["is-active", "kiwami-shell.service"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if state.is_empty() || state == "inactive" {
        if !session_env_has_wayland() {
            return Finding::new(Level::Skip, "shell not running (no graphical session)");
        }
        return Finding::new(Level::Fail, "kiwami-shell is not running")
            .remedy("systemctl --user start kiwami-shell");
    }

    let restarts: u32 = user_systemctl(&["show", "-p", "NRestarts", "--value", "kiwami-shell.service"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    // A flapping unit is active often enough to look fine. This is the check
    // that would have caught the shell being broken in CI for two green runs.
    if restarts > 5 {
        Finding::new(Level::Fail, format!("kiwami-shell has restarted {restarts} times"))
            .detail("it is running, but crashing and being restarted")
            .remedy("journalctl --user -u kiwami-shell -n 50")
    } else if state == "active" {
        Finding::new(Level::Ok, format!("kiwami-shell active ({restarts} restarts)"))
    } else {
        Finding::new(Level::Warn, format!("kiwami-shell is {state}"))
    }
}

fn theme_applied() -> Finding {
    match crate::theme::current() {
        Some(name) => {
            let colors = paths::current_theme().join("colors.json");
            if colors.is_file() {
                Finding::new(Level::Ok, format!("theme '{name}' applied"))
            } else {
                Finding::new(Level::Fail, format!("theme '{name}' recorded but not generated"))
                    .remedy("kiwami theme set")
            }
        }
        None => Finding::new(Level::Fail, "no theme applied")
            .detail("everything renders on fallback colours")
            .remedy("kiwami theme set kiwami"),
    }
}

// --- hygiene -------------------------------------------------------------

fn generations() -> Finding {
    let count = output("nix-env", &["--list-generations", "-p", "/nix/var/nix/profiles/system"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    if count == 0 {
        return Finding::new(Level::Skip, "generations not readable (needs root)");
    }
    if count > 10 {
        Finding::new(Level::Warn, format!("{count} system generations"))
            .remedy("sudo nix-collect-garbage --delete-older-than 14d")
    } else {
        Finding::new(Level::Ok, format!("{count} system generations"))
    }
}

/// How far behind the flake this machine is.
///
/// The question the checkout used to answer, minus the checkout. `kiwami
/// update` records the commit it built, so this compares that against what
/// main points at now - which cannot go stale the way a directory could, and
/// exists even on a machine that has never had a copy of its own config.
fn commit_drift() -> Finding {
    let Some(current) = crate::update::current_commit() else {
        return Finding::new(
            Level::Skip,
            "no recorded commit - this system predates `kiwami update`",
        );
    };
    let short = &current[..current.len().min(7)];

    let out = std::process::Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "flake",
            "metadata",
            "github:kiwamios/kiwami",
            "--refresh",
            "--json",
        ])
        .output();
    let Ok(out) = out else {
        return Finding::new(Level::Ok, format!("running {short} (cannot reach the flake)"));
    };
    if !out.status.success() {
        return Finding::new(Level::Ok, format!("running {short} (cannot reach the flake)"));
    }
    let head = serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .and_then(|v| v.get("revision").and_then(|r| r.as_str()).map(String::from));

    match head {
        Some(head) if head == current => Finding::new(Level::Ok, format!("up to date ({short})")),
        Some(head) => Finding::new(
            Level::Warn,
            format!("running {short}, main is {}", &head[..head.len().min(7)]),
        )
        .remedy("sudo kiwami update"),
        None => Finding::new(Level::Ok, format!("running {short}")),
    }
}

fn lock_age() -> Finding {
    let lock = paths::repo().join("flake.lock");
    let Ok(meta) = fs::metadata(&lock) else {
        return Finding::new(Level::Skip, "no flake.lock here");
    };
    let Ok(modified) = meta.modified() else {
        return Finding::new(Level::Skip, "flake.lock timestamp unreadable");
    };
    let days = modified
        .elapsed()
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);

    if days > 30 {
        Finding::new(Level::Warn, format!("flake.lock is {days} days old"))
            .remedy("nix flake update")
    } else {
        Finding::new(Level::Ok, format!("flake.lock is {days} days old"))
    }
}

/// The committed hardware facts are a snapshot of install day, not a live
/// query. Move the disk to another machine, swap the wifi card, and the file
/// keeps describing the old one - which fails at the worst moment, in the
/// initrd, with no shell to debug from. So re-ask the machine and diff.
fn hardware_drift() -> Finding {
    let repo = paths::repo();
    if !repo.join("flake.nix").exists() {
        return Finding::new(Level::Skip, "no checkout here to compare hardware against");
    }
    if !have("nixos-generate-config") {
        return Finding::new(Level::Skip, "nixos-generate-config not available");
    }

    let Some(host) = fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string())
    else {
        return Finding::new(Level::Skip, "hostname unreadable");
    };

    // The directory is named for the flake attribute, which need not match
    // networking.hostName. Fall back to a single unambiguous match.
    let Some(dir) = host_dir(&repo, &host) else {
        return Finding::new(Level::Skip, format!("no hosts/ entry matching {host}"));
    };

    let committed = match fs::read_to_string(dir.join("hardware.nix")) {
        Ok(c) => c,
        Err(_) => {
            return Finding::new(Level::Warn, format!("{} has no hardware.nix", dir.display()))
                .remedy("kiwami install --regen-hardware, or generate it by hand")
        }
    };

    // No --root here: the tool rejects `--root /` outright ("no need to
    // specify / with --root, it is the default"). At install time the target
    // is /mnt and the flag is required.
    let Some(fresh) = output(
        "nixos-generate-config",
        &["--show-hardware-config", "--no-filesystems"],
    ) else {
        return Finding::new(Level::Skip, "hardware detection failed (needs root)");
    };

    // Compare the settings, not the bytes: comments and blank lines differ
    // between nixpkgs revisions and would report drift on every bump.
    let a = significant(&committed);
    let b = significant(&fresh);
    if a == b {
        return Finding::new(Level::Ok, "hardware.nix matches this machine");
    }

    // availableKernelModules cannot be checked this way, in either direction.
    //
    // nixos-generate-config reports drivers for storage that is attached when
    // it runs, not the set the initrd needs to reach root. So the committed
    // file gains uas and sd_mod from the installer USB, and loses them the
    // moment the stick is pulled - and a drive plugged in next week adds
    // entries that were never missing. Both readings are noise.
    //
    // The true question - can the initrd still reach root - is not answerable
    // from this comparison either. This VM's root is virtio_blk, which is in
    // no committed list here and boots regardless, because NixOS ships a
    // default set and some drivers are built into the kernel.
    //
    // So the module lists are reported and not judged. Everything else in the
    // file - CPU vendor, microcode, hostPlatform, the imports - has no such
    // ambiguity and still has to match exactly.
    let is_modules = |l: &String| l.contains("KernelModules");
    let committed_modules: Vec<String> =
        a.iter().filter(|l| is_modules(l)).flat_map(|l| list_items(l)).collect();
    let detected_modules: Vec<String> =
        b.iter().filter(|l| is_modules(l)).flat_map(|l| list_items(l)).collect();

    let a_rest: Vec<&String> = a.iter().filter(|l| !is_modules(l)).collect();
    let b_rest: Vec<&String> = b.iter().filter(|l| !is_modules(l)).collect();
    let added: Vec<String> =
        b_rest.iter().filter(|l| !a_rest.contains(l)).map(|l| (*l).clone()).collect();
    let gone: Vec<String> =
        a_rest.iter().filter(|l| !b_rest.contains(l)).map(|l| (*l).clone()).collect();

    if added.is_empty() && gone.is_empty() {
        let only_here: Vec<&String> =
            committed_modules.iter().filter(|m| !detected_modules.contains(m)).collect();
        let only_now: Vec<&String> =
            detected_modules.iter().filter(|m| !committed_modules.contains(m)).collect();
        let note = if only_here.is_empty() && only_now.is_empty() {
            "initrd modules also match".to_string()
        } else {
            format!(
                "initrd modules differ ({} committed-only, {} detected-only), which \
                 reflects what was plugged in and is not checked",
                only_here.len(),
                only_now.len()
            )
        };
        return Finding::new(Level::Ok, "hardware.nix matches this machine").detail(note);
    }

    let mut detail = String::new();
    for l in gone.iter().take(4) {
        detail.push_str(&format!("- {l}\n"));
    }
    for l in added.iter().take(4) {
        detail.push_str(&format!("+ {l}\n"));
    }

    Finding::new(Level::Warn, "hardware.nix no longer matches this machine")
        .detail(detail.trim_end())
        .remedy("nixos-generate-config --show-hardware-config --no-filesystems > \
                 hosts/<name>/hardware.nix")
}

/// Which hosts/ directory describes this machine. The directory name is a
/// flake attribute and need not equal networking.hostName - hosts/vm-aarch64
/// is called kiwami-vm - so fall back to whichever host declares this
/// hostname, and finally to the only one there is.
fn host_dir(repo: &std::path::Path, hostname: &str) -> Option<std::path::PathBuf> {
    let by_name = repo.join("hosts").join(hostname);
    if by_name.is_dir() {
        return Some(by_name);
    }

    let mut dirs: Vec<_> = fs::read_dir(repo.join("hosts"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let declares = format!("hostName = \"{hostname}\"");
    let mut matched = dirs
        .iter()
        .filter(|d| {
            fs::read_to_string(d.join("default.nix"))
                .map(|c| c.contains(&declares))
                .unwrap_or(false)
        })
        .cloned();
    if let (Some(one), None) = (matched.next(), matched.next()) {
        return Some(one);
    }

    (dirs.len() == 1).then(|| dirs.remove(0))
}

/// Lines that carry meaning: no comments, no blank lines, whitespace
/// collapsed so reindentation is not drift.
///
/// List elements are sorted too. The kernel enumerates modules in whatever
/// order it found the devices, so two runs on an unchanged machine produce
/// the same set in a different order - and a check that reports that as drift
/// is a check people learn to ignore.
fn significant(nix: &str) -> Vec<String> {
    nix.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|l| sort_list_literal(&l))
        .collect()
}

/// The quoted items inside a `[ "a" "b" ]` literal.
fn list_items(line: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) else {
        return Vec::new();
    };
    if close < open {
        return Vec::new();
    }
    line[open + 1..close].split_whitespace().map(str::to_string).collect()
}

/// Sort the elements of a `[ "a" "b" ]` literal, leaving everything else be.
fn sort_list_literal(line: &str) -> String {
    let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) else {
        return line.to_string();
    };
    if close < open {
        return line.to_string();
    }
    let inner = &line[open + 1..close];
    let mut items: Vec<&str> = inner.split_whitespace().collect();
    items.sort_unstable();
    format!("{}[ {} ]{}", &line[..open], items.join(" "), &line[close + 1..])
}


/// Whether a stored hash is still the installer's default.
///
/// Hashes are salted, so they cannot be compared directly. Re-hashing the
/// known default with the salt from the stored hash reproduces it exactly if
/// and only if the password is unchanged.
fn is_default_password(file: &std::path::Path) -> bool {
    let Ok(stored) = fs::read_to_string(file) else { return false };
    let stored = stored.trim();
    // $6$<salt>$<hash>
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() < 4 {
        return false;
    }
    let salt = parts[2];
    let Some(rehashed) = hash_with_salt("kiwami", salt) else { return false };
    rehashed.trim() == stored
}

fn hash_with_salt(password: &str, salt: &str) -> Option<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("mkpasswd")
        .args(["-m", "sha-512", "-S", salt, "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(password.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Whether somebody has run `passwd` on a machine where it does nothing.
///
/// With mutableUsers off, passwd still writes to /etc/shadow and still says
/// it succeeded. Activation then regenerates the file from the configured
/// hash, so the change disappears - at the next rebuild, or at the next boot
/// on a machine whose root is wiped. The user is told it worked and finds out
/// otherwise at a login prompt.
fn password_is_declarative() -> Finding {
    let Ok(dir) = fs::read_to_string("/etc/kiwami/password-dir") else {
        return Finding::new(Level::Skip, "passwords are managed by passwd here");
    };
    let dir = dir.trim();
    if dir.is_empty() {
        return Finding::new(Level::Skip, "passwords are managed by passwd here");
    }

    let Some(user) = fs::read_to_string("/etc/kiwami/persist.json")
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("user")?.as_str().map(str::to_string))
    else {
        return Finding::new(Level::Skip, "cannot tell which user to check");
    };

    let file = std::path::Path::new(dir).join(&user);
    if file.is_file() {
        // Still the one the installer wrote? The hash is salted, so the check
        // is to re-hash the default with the stored salt and compare - which
        // is the only way to tell without keeping a copy of the plaintext.
        if is_default_password(&file) {
            return Finding::new(Level::Warn, format!("{user} still has the install default"))
                .detail("anyone who has read the docs knows it")
                .remedy("sudo kiwami passwd");
        }
        Finding::new(Level::Ok, format!("{user} has a password of their own"))
            .detail(format!("hashed in {}", file.display()))
    } else {
        Finding::new(Level::Fail, format!("{user} has no persistent password"))
            .detail(
                "this machine wipes /etc/shadow at every boot, so `passwd` reports\n                 success and changes nothing that lasts",
            )
            .remedy("sudo kiwami passwd")
    }
}

/// Whether the root was actually wiped this boot.
///
/// Only meaningful on a machine with an ephemeral root, and the reason it
/// exists is that the failure is silent in the direction that looks healthy:
/// if the initrd rollback does not run, nothing is wiped, everything
/// persists, and the machine behaves perfectly well while quietly
/// accumulating exactly the state it was set up to discard.
///
/// A subvolume's creation time is when it was made. Restored from a blank
/// snapshot at boot, it is younger than the boot; left alone, it dates from
/// the install.
fn root_was_wiped() -> Finding {
    if !std::path::Path::new("/persist").is_dir() {
        return Finding::new(Level::Skip, "not an ephemeral-root machine");
    }
    let Some(show) = output("btrfs", &["subvolume", "show", "/"]) else {
        return Finding::new(Level::Skip, "cannot read the root subvolume (needs root)");
    };
    let Some(created) = show
        .lines()
        .find(|l| l.trim_start().starts_with("Creation time:"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
    else {
        return Finding::new(Level::Skip, "no creation time for the root subvolume");
    };

    // Uptime is the cheapest boot clock available and needs no parsing of
    // timezones from btrfs's output beyond a date call.
    let uptime: f64 = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0);
    let created_epoch: Option<i64> = output("date", &["-d", &created, "+%s"])
        .and_then(|s| s.trim().parse().ok());
    let now_epoch: Option<i64> =
        output("date", &["+%s"]).and_then(|s| s.trim().parse().ok());

    match (created_epoch, now_epoch) {
        (Some(c), Some(n)) => {
            let age = n - c;
            if (age as f64) <= uptime + 120.0 {
                Finding::new(Level::Ok, "the root was wiped this boot")
                    .detail(format!("root subvolume is {age}s old, uptime {uptime:.0}s"))
            } else {
                Finding::new(Level::Fail, "the root was NOT wiped this boot")
                    .detail(format!(
                        "root subvolume is {age}s old but the machine booted {uptime:.0}s ago,\n\
                         so it survived the reboot - the initrd rollback did not run"
                    ))
                    .remedy("journalctl -b -u initrd-rollback, or check the initrd unit")
            }
        }
        _ => Finding::new(Level::Skip, "could not compare the root's age to uptime"),
    }
}


/// Whether activation writes this again, so losing it costs nothing.
///
/// A short list rather than a set of directories to search: the walk stays
/// general, and only the exceptions are named - each because something
/// demonstrably rewrites it.
///
/// /etc/shadow is on it now, and was the whole reason this report exists.
/// Under mutable users it held password hashes and was genuinely lost; with
/// users immutable it is regenerated at activation from kiwami.passwordFile,
/// so it is not state any more. The list changed because the system did.
fn regenerated(path: &str) -> bool {
    const WRITTEN_BY_ACTIVATION: [&str; 11] = [
        // update-users-groups.pl, from the configuration and the hash file
        "/etc/passwd", "/etc/group", "/etc/shadow", "/etc/subuid", "/etc/subgid",
        // markers and mounts, recreated on every boot
        "/etc/.clean", "/etc/.updated", "/etc/NIXOS", "/etc/mtab",
        // written by the resolver and by systemd
        "/etc/resolv.conf", "/etc/machine-id",
    ];
    if WRITTEN_BY_ACTIVATION.contains(&path) {
        return true;
    }

    // environment.etc builds /etc/static as a symlink tree into the store and
    // mirrors it into /etc. So NixOS keeps its own record of what it manages,
    // and anything with a counterpart there is rebuilt at activation - which
    // covers the directories it creates as well as the files, without naming
    // any of them.
    if let Some(rest) = path.strip_prefix("/etc/") {
        if std::path::Path::new("/etc/static").join(rest).exists() {
            return true;
        }
    }

    // home-manager keeps the same kind of record: its generation carries a
    // home-files tree mirroring every path it manages. So anything it writes
    // into ~/.config is rebuilt at activation, which is the whole point of
    // declaring it - a config file in home that is not in the flake is the
    // bug, not the wipe. Self-updating for the same reason /etc/static is.
    if let Some(managed) = home_manager_files() {
        let home = paths::user_home();
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(&home) {
            if managed.join(rest).exists() {
                return true;
            }
        }
    }

    // systemd recreates what its units and tmpfiles rules declare: a service's
    // StateDirectory reappears empty when it starts, a `d` rule reappears at
    // boot. Both come from the units themselves, so this updates itself too -
    // adding a service adds its rules and this notices.
    tmpfiles_paths().iter().any(|t| path == t || path.starts_with(&format!("{t}/")))
}

/// The tree of files home-manager manages, from its current generation.
fn home_manager_files() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let root = paths::user_home().join(".local/state/home-manager/gcroots/current-home");
            let generation = fs::canonicalize(root).ok()?;
            let files = generation.join("home-files");
            files.is_dir().then_some(files)
        })
        .clone()
}

/// Paths systemd-tmpfiles will create, read from the running configuration
/// rather than written down.
fn tmpfiles_paths() -> Vec<String> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let Some(out) = output("systemd-tmpfiles", &["--cat-config"]) else {
                return Vec::new();
            };
            out.lines()
                .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
                .filter_map(|l| {
                    let mut f = l.split_whitespace();
                    let kind = f.next()?;
                    // Only the types that bring a path into existence.
                    if !matches!(
                        kind.trim_end_matches(['!', '-', '=', '+', '^']),
                        "d" | "D" | "v" | "f" | "F" | "L" | "C"
                    ) {
                        return None;
                    }
                    f.next().map(str::to_string)
                })
                .collect()
        })
        .clone()
}

/// Everything on the root that will not survive the next boot.
///
/// Walks the root filesystem rather than a list of directories somebody
/// thought of, and stays on it: anything with a different device id is on
/// another filesystem and is not the root's to lose. That single test excludes
/// /boot, /nix, /persist, the pseudo-filesystems and every bind mount out of
/// the persist subvolume, because btrfs gives each subvolume its own id.
///
/// What remains is filtered by one more property rather than by name: a
/// symlink into the store is rebuilt at activation and costs nothing, while a
/// real file is something a program wrote. That is what surfaces /etc/shadow
/// without anybody having thought of /etc.
fn collect_ephemeral_loss(declared: &[String], out: &mut Vec<String>) {
    use std::os::unix::fs::MetadataExt;

    let Ok(root_meta) = fs::metadata("/") else { return };
    let root_dev = root_meta.dev();

    let mut stack = vec![(std::path::PathBuf::from("/"), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(Result::ok) {
            let path = e.path();
            let p = path.to_string_lossy().to_string();

            if declared.iter().any(|d| p == *d || p.starts_with(&format!("{d}/"))) {
                continue;
            }
            if regenerated(&p) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };

            // Another filesystem: not on the root, so not lost with it.
            if meta.dev() != root_dev {
                continue;
            }
            // Regenerated at activation from the store.
            if meta.is_symlink() {
                continue;
            }

            if meta.is_dir() {
                if depth < 3 {
                    stack.push((path, depth + 1));
                } else {
                    out.push(p);
                }
            } else if meta.is_file() {
                out.push(p);
            }
        }
    }
}

/// What would be lost if the root filesystem were wiped.
///
/// Not a wipe, and nothing here changes anything: the point is to answer
/// "what is accumulating that nothing declared" with a list rather than a
/// worry. Nix guarantees cover /nix/store; /etc and /var are ordinary mutable
/// directories that nothing garbage-collects, so a service removed from the
/// config leaves its state behind forever and nothing ever mentions it.
fn unpersisted_state() -> Finding {
    let Ok(raw) = fs::read_to_string("/etc/kiwami/persist.json") else {
        return Finding::new(Level::Skip, "no persist list generated");
    };
    let Ok(list) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Finding::new(Level::Warn, "persist list is not valid JSON");
    };
    let declared: Vec<String> = ["directories", "files"]
        .iter()
        .filter_map(|k| list.get(*k))
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    // /var/lib is where services keep state, so it is the honest place to
    // look. /etc is mostly store symlinks; the unmanaged files there are a
    // separate question and a noisier one.
    let mut unlisted: Vec<String> = Vec::new();

    // What gets destroyed is not a list of directories to check. It is
    // everything on the root subvolume that is not bound out of /persist -
    // the blank snapshot is empty, so the root's entire contents are
    // destined for deletion.
    //
    // The filter is therefore not "which places are interesting" but "which
    // of those will not come back". A symlink into the store is rebuilt at
    // activation and costs nothing; a real file is something a program wrote.
    // That rule finds /etc/shadow without anybody having thought of /etc,
    // which is the point - the previous version scanned three hardcoded
    // directories and was blind to the one that mattered.
    collect_ephemeral_loss(&declared, &mut unlisted);

    unlisted.sort();

    let shown: Vec<String> = unlisted.iter().take(12).cloned().collect();
    let more = unlisted.len().saturating_sub(shown.len());
    let mut detail = shown.join("\n");
    if more > 0 {
        detail.push_str(&format!("\n... and {more} more"));
    }

    // Not a failure. Most of this is legitimately disposable - the point is
    // that nobody has decided which, and an ephemeral root makes that
    // decision for you whether you meant it or not.
    Finding::new(
        Level::Warn,
        format!("{} paths on the root will not survive a reboot", unlisted.len()),
    )
    .detail(detail)
    .remedy("decide which matter and add them to kiwami.persist.directories")
}

// --- driver --------------------------------------------------------------

pub fn run() -> Result<(), ()> {
    let sections: [(&str, Vec<Finding>); 3] = [
        ("drift", {
            let mut v = vec![nix_profile(), nix_env(), stray_binaries()];
            v.extend(language_managers());
            v
        }),
        ("health", vec![
            failed_units(),
            failed_user_units(),
            graphical_session(),
            shell_unit(),
            theme_applied(),
        ]),
        ("hygiene", vec![generations(), commit_drift(), lock_age(), hardware_drift(), root_was_wiped(), password_is_declarative(), unpersisted_state()]),
    ];

    let mut fails = 0;
    let mut warns = 0;

    for (name, findings) in sections.iter() {
        println!("\n\x1b[1m{name}\x1b[0m");
        for f in findings {
            println!("{f}");
            match f.level {
                Level::Fail => fails += 1,
                Level::Warn => warns += 1,
                _ => {}
            }
        }
    }

    // Reported, never prompted for: doctor is safe to run anywhere and does
    // not open browsers. It only says the gap exists and where to go.
    let unauthenticated = crate::auth::missing_count();
    if unauthenticated > 0 {
        println!("\n\x1b[1mcredentials\x1b[0m");
        println!("  {unauthenticated} tool(s) not authenticated - run `kiwami auth`");
    }

    println!();
    if fails == 0 && warns == 0 {
        println!("\x1b[32mno problems\x1b[0m");
        Ok(())
    } else {
        println!("{fails} problem(s), {warns} warning(s)");
        if fails > 0 { Err(()) } else { Ok(()) }
    }
}

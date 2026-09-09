//! `kiwami update` - rebuild this machine from the flake, which lives on
//! GitHub and nowhere else.
//!
//! The machine deliberately keeps no checkout of its own configuration. Every
//! bug in this area came from having one: a copy that could be older than
//! GitHub, older than the disk it describes, or restored from a backup taken
//! before a layout change - each one able to revert the machine's config
//! silently at the next rebuild. None of that is possible if the copy does not
//! exist.
//!
//! What that costs: changing the machine needs a network. Running it does not
//! - the built system is in /nix/store and boots without ever reading a flake.
//!
//! Hacking on the config locally is still ordinary: clone anywhere and
//! `nixos-rebuild switch --flake /path#host`. It is simply not the default, so
//! an uncommitted edit cannot quietly become what the machine is.

use std::fs;
use std::process::Command;

/// What went wrong running a command, said in a way that names the cause.
///
/// `Command::status()` failing with ENOENT means the binary is not on PATH,
/// but the error renders as "No such file or directory (os error 2)" - which
/// reads like a missing *file* and sends you looking at the flake. It cost a
/// round-trip here: `kiwami update` under a systemd unit, whose PATH does not
/// include the system profile, reported "nix flake metadata: No such file or
/// directory" and said nothing about nix being absent.
fn ran(what: &str, e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!(
            "{what} is not on PATH.\n\n\
             Kiwami rebuilds by shelling out to it, so it has to be findable. \
             Running under a systemd unit or a cron job is the usual reason - \
             those get a minimal PATH that omits /run/current-system/sw/bin."
        )
    } else {
        format!("{what}: {e}")
    }
}

/// Where this machine's configuration lives, and what it is called there -
/// written by Nix at build time from `kiwami.flake` and `kiwami.host`.
///
/// This used to be a constant pointing at the author's repository. That works
/// for exactly one person: anybody else's machine would have rebuilt itself
/// from somebody else's configuration, or - more likely - failed to find a
/// host by its name and stopped, with an error naming a repository they have
/// never heard of.
const ORIGIN: &str = "/etc/kiwami/origin.json";

struct Origin {
    flake: String,
    host: String,
}

fn origin() -> Result<Origin, String> {
    let raw = fs::read_to_string(ORIGIN)
        .map_err(|e| format!("cannot read {ORIGIN}: {e}\nThis system was not built by Kiwami."))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("{ORIGIN}: {e}"))?;
    let flake = v.get("flake").and_then(|f| f.as_str()).unwrap_or("").to_string();
    let host = v.get("host").and_then(|h| h.as_str()).unwrap_or("").to_string();

    // Said here rather than after a failed build, because the fix is one line
    // in the machine's own configuration and the error should name it.
    if flake.is_empty() {
        return Err(format!(
            "this machine does not say where its configuration lives.\n\n\
             Set it in the host's configuration and rebuild once:\n\n  \
             kiwami.flake = \"github:you/your-config\";\n\n\
             After that `kiwami update` builds from there."
        ));
    }
    if host.is_empty() {
        return Err("this machine has no host name to build".into());
    }
    Ok(Origin { flake, host })
}

/// Where the commit this system was built from is recorded. Under
/// /var/lib/kiwami because that path is persisted, so the answer survives the
/// root being wiped.
const STAMP: &str = "/var/lib/kiwami/commit";

pub fn run(commit: Option<String>, dry: bool) -> Result<(), String> {
    if !crate::install::is_root() {
        return Err("must run as root (try: sudo kiwami update)".into());
    }

    let Origin { flake: base, host } = origin()?;

    // Resolved to a commit before anything is built.
    //
    // Two reasons. The output can then say exactly what you are running rather
    // than "main", which is a moving target; and two machines updated a minute
    // apart get the same system instead of whatever main happened to be.
    let rev = match commit {
        Some(c) => c,
        None => resolve_head(&base)?,
    };
    let flake = format!("{base}/{rev}");

    println!("==> {host} from {rev}");
    if !host_exists(&flake, &host)? {
        return Err(format!(
            "{base} does not describe a machine called {host}.\n\
             Its configuration has not been pushed yet:\n\n  \
             sudo kiwami host push\n\n\
             A machine whose config is not in the flake cannot be rebuilt from it."
        ));
    }

    let action = if dry { "build" } else { "switch" };
    let status = Command::new("nixos-rebuild")
        .args([action, "--flake", &format!("{flake}#{host}")])
        .status()
        .map_err(|e| ran("nixos-rebuild", e))?;
    if !status.success() {
        return Err("nixos-rebuild failed".into());
    }

    if dry {
        println!("\nbuilt, not switched");
        return Ok(());
    }

    // Recorded so `kiwami doctor` can answer "is this machine current?"
    // without a checkout to compare against - the question the checkout used
    // to answer, minus the copy that could lie about it.
    if let Some(dir) = std::path::Path::new(STAMP).parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(STAMP, format!("{rev}\n"));

    println!("\nnow running {rev}");
    Ok(())
}

/// The commit main points at right now.
///
/// --refresh because github: flakes are cached for an hour by default, which
/// twice today served a commit older than the fix being tested. Baked in here
/// so it can never be the explanation for "but I pushed that".
fn resolve_head(base: &str) -> Result<String, String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "flake",
            "metadata",
            base,
            "--refresh",
            "--json",
        ])
        .output()
        .map_err(|e| ran("nix", e))?;
    if !out.status.success() {
        return Err(format!(
            "cannot reach {base}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("nix flake metadata: {e}"))?;
    v.get("revision")
        .and_then(|r| r.as_str())
        .map(String::from)
        .ok_or_else(|| "no revision in flake metadata".into())
}

fn host_exists(flake: &str, host: &str) -> Result<bool, String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--json",
            &format!("{flake}#nixosConfigurations"),
            "--apply",
            "builtins.attrNames",
        ])
        .output()
        .map_err(|e| ran("nix", e))?;
    if !out.status.success() {
        return Err(format!(
            "cannot evaluate {flake}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let names: Vec<String> = serde_json::from_slice(&out.stdout).unwrap_or_default();
    Ok(names.iter().any(|n| n == host))
}

/// Build an installer image carrying this machine's whole system.
///
/// Boot it and the install needs no network: `nixos-install --system` copies a
/// closure that is already on the stick, instead of evaluating a flake and
/// downloading several gigabytes.
///
/// A command rather than a documented `nix build` line, for the same reason
/// `kiwami update` exists: an incantation you have to look up is one you get
/// wrong at the moment you are least able to afford it.
pub fn image(host: Option<String>, out: String) -> Result<(), String> {
    let o = origin()?;
    let host = host.unwrap_or(o.host);
    let rev = resolve_head(&o.flake)?;
    let attr = format!(
        "{}/{rev}#nixosConfigurations.installer-{host}.config.system.build.isoImage",
        o.flake
    );

    println!("==> building an installer for {host} from {rev}");
    println!("    this compresses the whole system, so it is minutes, not seconds");

    let status = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "build",
            &attr,
            "--print-build-logs",
            "--out-link",
            &out,
        ])
        .status()
        .map_err(|e| ran("nix", e))?;
    if !status.success() {
        return Err(format!(
            "could not build an image for {host}.\n\
             Is its configuration pushed? The image is built from the flake, not from here."
        ));
    }

    // The path, because the next thing anyone does is write it to a stick.
    let iso = std::fs::read_dir(format!("{out}/iso"))
        .ok()
        .and_then(|mut d| d.next().and_then(|e| e.ok()))
        .map(|e| e.path().display().to_string());

    println!();
    match iso {
        Some(path) => {
            let size = std::fs::metadata(&path).map(|m| m.len() / 1_000_000).unwrap_or(0);
            println!("  {path}  ({size} MB)");
            println!("\n  sudo scripts/flash-linux.sh {path}");
        }
        None => println!("  built: {out}"),
    }
    Ok(())
}

/// The commit this system was last built from, if it was built by us.
pub fn current_commit() -> Option<String> {
    fs::read_to_string(STAMP).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

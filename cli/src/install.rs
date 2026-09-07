//! The Kiwami installer.
//!
//! Runs from the live ISO. Everything here is destructive, so the ordering is
//! deliberate: detect, then select, then confirm, and only then touch a disk.
//! Nothing before the confirmation writes anything.

use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::net;
use crate::nix::{self, field, noted, raw, s, Field, Nix};

/// Give generated files back to whoever owns the checkout.
///
/// The installer runs as root, so anything it writes into your repository is
/// root-owned - you would need sudo to edit your own machine's config, and
/// tools that rewrite the tree (rsync, git checkout) fail outright.
fn chown_to_repo(repo: &Path, target: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::metadata(repo) else { return Ok(()) };
    let (uid, gid) = (meta.uid(), meta.gid());

    let mut stack = vec![target.to_path_buf()];
    while let Some(p) = stack.pop() {
        let _ = std::os::unix::fs::chown(&p, Some(uid), Some(gid));
        if p.is_dir() {
            if let Ok(entries) = fs::read_dir(&p) {
                stack.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
            }
        }
    }
    Ok(())
}

/// Where the key that opens a second encrypted disk lives, on the already
/// decrypted root.
///
/// Not under /etc: NixOS builds that from the store, and store paths are
/// world-readable - a secret placed there is a secret published. /var/lib is
/// plain mutable state, survives rebuilds, and stays mode 0600.
const KEYFILE: &str = "/var/lib/kiwami/home.key";

/// Refuse anything smaller than this. Below it the install fails partway
/// through, which is worse than refusing up front.
const MIN_BYTES: u64 = 20 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct Disk {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub model: String,
    pub removable: bool,
    /// Existing partitions. Non-empty means we are about to destroy something.
    pub partitions: Vec<String>,
    /// A filesystem from this disk is mounted right now. On the ISO that is
    /// normal for the USB stick; on a running system it is your root.
    pub in_use: bool,
}

impl Disk {
    pub fn is_empty(&self) -> bool {
        self.partitions.is_empty()
    }
    fn gib(&self) -> f64 {
        self.bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

impl fmt::Display for Disk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:<12} {:>8.1} GiB  {}", self.path.display(), self.gib(), self.model)?;
        if self.removable {
            write!(f, " [removable]")?;
        }
        if !self.is_empty() {
            write!(f, "  ** {} existing partition(s) **", self.partitions.len())?;
        }
        if self.in_use {
            write!(f, "  ** IN USE - mounted right now **")?;
        }
        Ok(())
    }
}

/// Enumerate real block devices. Skips loop/ram/zram devices and optical
/// drives, which are never install targets and would only pad the menu.
pub fn disks() -> io::Result<Vec<Disk>> {
    let mounted = mounted_devices();
    let mut out = Vec::new();
    for entry in fs::read_dir("/sys/block")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("loop")
            || name.starts_with("ram")
            || name.starts_with("zram")
            || name.starts_with("sr")
            || name.starts_with("fd")
            || name.starts_with("dm-")
        {
            continue;
        }
        // NVMe exposes a controller-scoped alias of every namespace as
        // nvme<N>c<M>n<K>, pointing at the same storage as nvme<N>n<K>.
        // Listing both offers the same disk twice under two names.
        if is_nvme_controller_alias(&name) {
            continue;
        }
        let base = entry.path();

        // /sys reports size in 512-byte sectors regardless of physical sector size.
        let sectors: u64 = fs::read_to_string(base.join("size"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if sectors == 0 {
            continue;
        }

        let model = fs::read_to_string(base.join("device/model"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".into());
        let removable = fs::read_to_string(base.join("removable"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false);

        // A partition appears as a subdirectory carrying its own `partition` file.
        let mut partitions: Vec<String> = fs::read_dir(&base)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("partition").exists())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        partitions.sort();

        let in_use = mounted.iter().any(|m| m == &name || partitions.contains(m));

        out.push(Disk {
            path: PathBuf::from(format!("/dev/{name}")),
            name,
            bytes: sectors * 512,
            model,
            removable,
            partitions,
            in_use,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `nvme0c0n1` is the controller alias for `nvme0n1` - same device, second
/// name. Matches nvme<digits>c<digits>n<digits>.
fn is_nvme_controller_alias(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("nvme") else { return false };
    let Some((ctrl, rest)) = rest.split_once('c') else { return false };
    let Some((inst, ns)) = rest.split_once('n') else { return false };
    !ctrl.is_empty()
        && !inst.is_empty()
        && !ns.is_empty()
        && ctrl.chars().all(|c| c.is_ascii_digit())
        && inst.chars().all(|c| c.is_ascii_digit())
        && ns.chars().all(|c| c.is_ascii_digit())
}


/// Read one answer.
///
/// End of input is an error, not an empty answer. Every menu here loops until
/// it gets something valid, so returning "" on EOF makes each of them spin
/// forever printing its retry message - which is exactly what a closed stdin
/// looks like to a piped test or an unattended run.
fn prompt(question: &str) -> io::Result<String> {
    print!("{question}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more input"));
    }
    Ok(line.trim().to_string())
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {} failed ({status})", args.join(" ")))
    }
}

pub struct Options {
    pub disk: Option<String>,
    /// Host attribute to install. Resolved against the flake, and prompted
    /// for when omitted - there is no sensible default, and guessing one
    /// means discovering it was wrong after the disk is already erased.
    pub host: Option<String>,
    pub flake: String,
    pub assume_yes: bool,
    pub force: bool,
    /// Create hosts/<name>/ if the flake does not define it yet.
    pub new_host: bool,
    /// Rewrite hardware.nix even if one is already committed.
    pub regen_hardware: bool,
    /// Ask the layout questions again for a host that already declares them.
    pub relayout: bool,
    /// Offer the steps that come before an install - networking, and being
    /// reachable for help - rather than assuming they were done already.
    pub guided: bool,
}

pub fn run_install(opts: Options) -> Result<(), String> {
    // Refuse to run on a machine that is already installed.
    //
    // This used to test for /etc/NIXOS, on the belief that only an installed
    // system has it. The live ISO has it too, so the guard fired on the one
    // place the installer is meant to run. Nothing caught it because the
    // automated install passes --force.
    if !on_installer_media() && !opts.force {
        return Err("this looks like an installed NixOS system, not the installer ISO.\n\
                    Refusing to continue. Pass --force if you really mean it."
            .into());
    }

    // Check this before anything else. Discovering it after the user has
    // confirmed a destructive action means failing halfway through
    // partitioning, which is the worst possible moment.
    if !is_root() {
        return Err("must run as root (try: sudo kiwami install)".into());
    }

    // The install downloads its whole closure from the binary cache, so this
    // has to pass before anything else. `--yes` implies unattended, and an
    // unattended run must not sit on a wifi password prompt.
    if opts.guided {
        println!("\nKiwami installer\n");
    }

    println!("==> checking network");
    // A medium carrying a system does not need one to install. It is still
    // wanted - the restore reads from R2, and a machine that cannot reach
    // anything is only half back - but "no wifi here" should not stop you
    // getting a bootable laptop out of a stick that already holds the whole
    // system.
    let carries_system = Path::new("/iso/kiwami/system").exists();
    match net::ensure(!opts.assume_yes) {
        Ok(()) => {}
        Err(e) if carries_system => {
            println!("    no network: {e}");
            println!("    continuing anyway - this medium carries the system.");
            println!("    the restore needs a network, so it will be offered later.");
        }
        Err(e) => return Err(e),
    }

    // Offered here rather than left as something to remember: an install is
    // exactly when a second pair of eyes is useful, and afterwards the machine
    // has rebooted and the offer is gone.
    if opts.guided && !opts.assume_yes {
        let answer = prompt(
            "\nMake this machine reachable over your tailnet, so someone can help?\n\
             It opens a URL to approve from a browser elsewhere. [y/N] ",
        )
        .map_err(|e| e.to_string())?;
        if answer.eq_ignore_ascii_case("y") {
            crate::remote::run(false)?;
        }
    }

    // Resolve the host now, while the disk is still intact. This costs one
    // cheap eval - `builtins.attrNames` does not force the configurations -
    // and it is the difference between "unknown host" and "unknown host,
    // reported after your disk was erased".
    println!("==> resolving {}", opts.flake);
    let mut flake = opts.flake.clone();
    let mut checkout = local_checkout(&flake);

    // Whether a new machine is possible, not whether it is possible yet. The
    // clone happens after the choice is made - asking first, then refusing
    // because nothing had been cloned, made the menu offer something it then
    // would not do.
    let can_write = checkout.is_some() || clone_url(&flake).is_some();
    if opts.relayout && !can_write {
        return Err(format!(
            "--relayout rewrites hosts/<name>/disk.nix, and {flake} can neither be\n\
             written to nor cloned."
        ));
    }
    let host = resolve_host(
        &flake,
        opts.host.clone(),
        opts.assume_yes,
        opts.new_host,
        can_write,
    )?;

    // Now that a new machine has actually been chosen, give it somewhere to
    // be written down.
    if (host.create || opts.relayout) && checkout.is_none() {
        let dir = clone_flake(&flake, opts.assume_yes)?;
        flake = dir.to_string_lossy().to_string();
        checkout = Some(dir);
    }
    println!("    host: {}", host.name);
    if host.create {
        println!("    will scaffold hosts/{}", host.name);
    }

    let disks = disks().map_err(|e| format!("cannot enumerate disks: {e}"))?;
    if disks.is_empty() {
        return Err("no disks found".into());
    }
    // select_disk consumes the list; the wizard still needs the others to
    // offer as a /home target.
    let disks_snapshot = disks.clone();

    // An existing host already declares its disks, so that is the target -
    // unless --relayout, which is how a machine changes its layout at all.
    // Editing disk.nix on a running system does nothing: it is a recipe for
    // formatting, and the disk is already formatted. So the layout changes
    // where formatting happens, which is here.
    let declared = if host.create || opts.relayout {
        Vec::new()
    } else {
        host_disks(&flake, &host.name)?
    };

    let mut targets: Vec<Disk> = if declared.is_empty() {
        // No declaration yet - a new host, whose disk.nix is written from
        // this choice.
        let target = match &opts.disk {
            Some(want) => {
                let want =
                    if want.starts_with("/dev/") { want.clone() } else { format!("/dev/{want}") };
                disks
                    .into_iter()
                    .find(|d| d.path.to_string_lossy() == want)
                    .ok_or_else(|| format!("no such disk: {want}"))?
            }
            None => select_disk(disks)?,
        };
        vec![target]
    } else {
        if let Some(want) = &opts.disk {
            let want =
                if want.starts_with("/dev/") { want.clone() } else { format!("/dev/{want}") };
            if !declared.iter().any(|d| d.to_string_lossy() == want) {
                return Err(format!(
                    "--disk {want} is not what {} declares.\n\
                     Its disk.nix names: {}\n\
                     Edit disk.nix instead - the layout is what formats, not this flag.",
                    host.name,
                    declared.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        let mut found = Vec::new();
        for dev in &declared {
            let d = disks.iter().find(|d| &d.path == dev).ok_or_else(|| {
                format!("{} is declared by {} but not present here", dev.display(), host.name)
            })?;
            found.push(d.clone());
        }
        found
    };

    for t in &targets {
        if t.in_use && !opts.force {
            return Err(format!(
                "{} is mounted right now - this looks like the system you are running.\n\
                 Refusing to erase it. Pass --force if you really mean it.",
                t.path.display()
            ));
        }
        if t.bytes < MIN_BYTES {
            return Err(format!(
                "{} is {:.1} GiB; Kiwami needs at least {} GiB",
                t.path.display(),
                t.gib(),
                MIN_BYTES / (1024 * 1024 * 1024)
            ));
        }
    }

    // A host with no declaration gets one now, from four questions - and it
    // is written and reviewed before anything is erased, so the last thing
    // seen before the point of no return is the actual layout.
    if declared.is_empty() {
        let Some(repo) = &checkout else {
            return Err(format!(
                "{} is not a local checkout, so a disk layout cannot be written into it.\n\
                 Clone the flake first, then install from that path.",
                opts.flake
            ));
        };
        let layout = ask_layout(&disks_snapshot, &targets[0])?;
        let host_dir = repo.join("hosts").join(&host.name);
        // Whether this directory is ours to remove again if the review is
        // abandoned. Leaving a half-written host behind is not harmless: the
        // next run finds it already declared and skips the wizard entirely.
        let ours = !host_dir.exists();
        fs::create_dir_all(&host_dir).map_err(|e| e.to_string())?;
        let path = host_dir.join("disk.nix");

        // Replacing a layout is not the same act as writing a first one. The
        // old file is committed, describes the disk currently in the machine,
        // and is about to stop being true - so what changes is shown before
        // anything is written, not just before anything is erased.
        let previous = fs::read_to_string(&path).ok();
        let rendered = render_disk_nix(&layout)?;
        if let Some(before) = &previous {
            if *before == rendered {
                println!("\n==> the layout is unchanged");
            } else {
                println!("\n==> {} is being replaced", path.display());
                describe_layout_change(before, &layout);
                if !opts.assume_yes {
                    let answer = prompt("\nReplace it? [y/N] ").map_err(|e| e.to_string())?;
                    if !answer.eq_ignore_ascii_case("y") {
                        return Err("layout left as it was; nothing erased".into());
                    }
                }
            }
        }
        fs::write(&path, &rendered).map_err(|e| e.to_string())?;

        // The rest of the host has to exist now too. disko's script is built
        // from this host, so a directory holding only disk.nix cannot be
        // formatted from - and a half-written host would break evaluation of
        // every other machine in the flake.
        if !host_dir.join("default.nix").exists() {
            scaffold_host(&host_dir, &host.name, &flake)?;
        }
        // Only when there is nothing to lose. --relayout runs this same branch
        // for a host that already exists, and writing the placeholder there
        // destroyed a real, committed hardware.nix - after which the "detect
        // hardware" step below saw a file present and skipped, so the machine
        // installed with nothing but nixpkgs.hostPlatform.
        //
        // The result was a laptop with no redistributable firmware: no wifi
        // (ath10k found no firmware) and no i915 DMC. It booted perfectly and
        // looked fine until the first attempt to use the network.
        if !host_dir.join("hardware.nix").exists() {
            placeholder_hardware(&host_dir)?;
        }
        chown_to_repo(repo, &host_dir)?;

        // Staged only once the layout is accepted - the flake must not be
        // left carrying a host nobody agreed to.
        // Before formatting: disko enrols this into the new volume's keyslot.
        if layout.encrypt && layout.home.is_some() {
            make_keyfile()?;
        }

        if let Err(e) = review_layout(&path, opts.assume_yes) {
            if ours {
                let _ = fs::remove_dir_all(&host_dir);
            } else if let Some(before) = &previous {
                // Put the old layout back. Abandoning the review erases
                // nothing, so the file describing the disk must go back to
                // describing the disk - otherwise declining the change leaves
                // it silently applied on paper, and the next install without
                // --relayout formats to the layout you just refused.
                let _ = fs::write(&path, before);
                println!("    {} restored", path.display());
            }
            return Err(e);
        }
        if repo.join(".git").exists() {
            git_add(repo, &host_dir)?;
        }

        // Every disk the layout touches, not just the one picked first.
        let mut wanted = vec![layout.system.clone()];
        wanted.extend(layout.home.clone());
        for dev in &wanted {
            if let Some(d) = disks_snapshot.iter().find(|d| &d.path == dev) {
                if d.in_use && !opts.force {
                    return Err(format!(
                        "{} is mounted right now. Refusing to erase it.",
                        d.path.display()
                    ));
                }
            }
        }
        targets = wanted
            .iter()
            .filter_map(|dev| disks_snapshot.iter().find(|d| &d.path == dev).cloned())
            .collect();
    }

    // Everything above this line is read-only.
    confirm(&targets, opts.assume_yes)?;

    // One step, from the host's own disk.nix: disko wipes, partitions,
    // formats and mounts. The layout is not restated here, which is the point
    // - it used to live twice, as parted calls in this file and as a
    // fileSystems module, agreeing only by comment.
    println!("\n==> partitioning and formatting from {}#{}", flake, host.name);
    run_disko(&flake, &host.name)?;

    // The key has to exist on the installed root, not just in the installer's
    // memory, or /home asks for a passphrase from the second boot onward.
    if Path::new(KEYFILE).exists() {
        install_keyfile()?;
    }

    // Only now is /mnt populated, which is what nixos-generate-config reads.
    if let Some(repo) = &checkout {
        let host_dir = repo.join("hosts").join(&host.name);
        let hardware = host_dir.join("hardware.nix");

        if host.create && !host_dir.join("default.nix").exists() {
            println!("==> scaffolding hosts/{}", host.name);
            scaffold_host(&host_dir, &host.name, &flake)?;
        }

        // A committed hardware.nix may have been tuned by hand; replacing it
        // silently on a reinstall is not ours to do. `kiwami doctor` reports
        // when it has drifted from what the machine actually reports.
        if opts.regen_hardware || host.create || !hardware.exists() {
            println!("==> detecting hardware");
            generate_hardware(&host_dir)?;
        } else {
            println!("==> keeping the committed hardware.nix (--regen-hardware to replace)");
        }

        chown_to_repo(repo, &host_dir)?;

        // Only when the checkout is a git repo. A plain directory flake
        // copies everything, so there is nothing to stage.
        if repo.join(".git").exists() {
            git_add(repo, &host_dir)?;
        }
    } else if host.create || opts.regen_hardware {
        return Err(format!(
            "{} is not a local checkout, so nothing can be written into it.\n\
             Clone the flake first, then install from that path.",
            opts.flake
        ));
    }

    // A closure on the medium is installed directly. Evaluating the flake
    // and downloading ~6 GiB is the slow part of an install and the only part
    // that needs a network; when the stick already carries the system, both
    // disappear.
    match prebuilt_system(&host.name) {
        Some(path) => {
            println!("==> installing the system from this medium (no download)");
            run("nixos-install", &["--system", &path, "--no-root-passwd"])?;
        }
        None => {
            println!("==> installing {} (this takes a while)", flake);
            let flake_ref = format!("{}#{}", flake, host.name);
            run("nixos-install", &["--flake", &flake_ref, "--no-root-passwd"])?;
        }
    }

    println!("==> seeding the password");
    seed_password(&flake, &host.name)?;

    println!("==> setting the boot order");
    fix_boot_order()?;

    // The restore runs first, and the installer's own network state is laid
    // down after it. Both write the same files, so which goes last decides
    // what the machine wakes up with - see carry_network_state.
    //
    // Its failure is deliberately not fatal. The system is installed by this
    // point; a restore that could not reach R2 or could not unlock Bitwarden
    // is a missing convenience, not a broken machine, and aborting here would
    // skip carrying the network over - stranding a machine that is fine, with
    // no wifi and no tailnet, precisely when you need to reach it to retry.
    // So it is reported at the end, where it cannot be mistaken for success.
    let outcome = offer_restore(opts.guided, opts.assume_yes);
    let restored = matches!(outcome, Ok(true));

    println!("==> carrying network state over");
    carry_network_state(restored)?;

    if let Err(e) = &outcome {
        println!("\n    the restore did not finish: {e}");
        println!("    the system is installed and will boot. Retry it after boot:");
        println!("      sudo kiwami snapshot restore --identity");
    }

    // Deliberately no checkout is placed on the installed machine.
    //
    // It used to copy one into ~/kiwami so the machine could rebuild itself.
    // That copy was a second source of truth: it could be older than GitHub,
    // older than the disk layout it described, or restored from a backup taken
    // before that layout changed - and any of those silently reverts the
    // machine's configuration at the next rebuild. It also only worked when
    // installing from a local checkout, so installing from a flake URL left an
    // empty directory and a machine that could not rebuild at all.
    //
    // `kiwami update` rebuilds from the flake on GitHub instead, and a
    // workspace to hack in is an ordinary clone in ~/Projects.

    // A host that was written or changed here exists only on this machine
    // until it is pushed. Offered now rather than left as a note: the machine
    // is about to reboot, the network is up, and "do it later" is how
    // hosts/xps spent its first week one dead disk from unrecoverable.
    if opts.relayout || host.create {
        offer_host_push(&checkout, &host.name, opts.assume_yes)?;
    }

    println!("\n==> done. Reboot into the installed system.");
    if !Path::new("/mnt/persist/var/lib/kiwami/backup/env").exists() {
        // Said here because the restore is the difference between a machine
        // that is yours and one that merely boots, and the moment to notice
        // is before the installer is gone.
        println!("\n    Nothing was restored, so this machine will come up without");
        println!("    its keys, tokens or wifi. To restore now, before rebooting:");
        println!("      sudo kiwami snapshot restore --target /mnt --identity");
    }
    Ok(())
}

fn select_disk(disks: Vec<Disk>) -> Result<Disk, String> {
    println!("Disks:\n");
    for (i, d) in disks.iter().enumerate() {
        println!("  {}) {d}", i + 1);
    }
    println!();

    // A single disk still gets shown and confirmed rather than auto-picked:
    // "it only found one" is exactly when a wrong guess goes unnoticed.
    loop {
        let answer = prompt(&format!("Install to which disk? [1-{}] ", disks.len()))
            .map_err(|e| e.to_string())?;
        match answer.parse::<usize>() {
            Ok(n) if n >= 1 && n <= disks.len() => {
                return Ok(disks.into_iter().nth(n - 1).unwrap())
            }
            _ => println!("Enter a number between 1 and {}.", disks.len()),
        }
    }
}

fn confirm(targets: &[Disk], assume_yes: bool) -> Result<(), String> {
    println!();
    for t in targets {
        println!("About to install to {}", t.path.display());
        if !t.is_empty() {
            println!(
                "\n  WARNING: {} already has {} partition(s): {}",
                t.path.display(),
                t.partitions.len(),
                t.partitions.join(", ")
            );
            println!("  Everything on it will be destroyed.");
        }
    }

    if assume_yes {
        println!("\n--yes given, continuing.");
        return Ok(());
    }

    // Typing the whole word, not "y". This is the last chance to stop.
    let answer = prompt("\nType 'yes' to erase this disk and install: ")
        .map_err(|e| e.to_string())?;
    if answer != "yes" {
        return Err("cancelled; nothing was written".into());
    }
    Ok(())
}




/// Base names of block devices backing a current mount, e.g. ["vda1", "vda2"].
fn mounted_devices() -> Vec<String> {
    fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|dev| dev.strip_prefix("/dev/"))
        .map(|d| d.to_string())
        .collect()
}

pub fn is_root() -> bool {
    // No libc dependency for one number: the kernel reports it in /proc.
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
        })
        .map(|uid| uid == "0")
        .unwrap_or(false)
}

/// The hosts this flake can install, straight from the flake itself.
///
/// `--extra-experimental-features` is passed explicitly because the stock
/// installer ISO does not enable flakes, and the failure without it is an
/// unhelpful "experimental feature not enabled" from a command the user
/// never typed.
fn flake_hosts(flake: &str) -> Result<Vec<String>, String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--json",
            &format!("{flake}#nixosConfigurations"),
            // Only what can actually be installed. The installer images are
            // in here too, and offering them as install targets is offering
            // to install the installer - they have no disk layout, which is
            // the property that decides it rather than their name.
            "--apply",
            "cfgs: builtins.filter (n: (cfgs.${n}.config.disko.devices.disk or {}) != {}) \
             (builtins.attrNames cfgs)",
        ])
        .output()
        .map_err(|e| format!("cannot run nix: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "cannot read hosts from {flake}:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    serde_json::from_slice(&out.stdout).map_err(|e| format!("unexpected output from nix: {e}"))
}

pub struct HostPlan {
    pub name: String,
    /// The flake does not define this host yet; scaffold it after mounting.
    pub create: bool,
}

fn resolve_host(
    flake: &str,
    want: Option<String>,
    assume_yes: bool,
    new_host: bool,
    writable: bool,
) -> Result<HostPlan, String> {
    let hosts = flake_hosts(flake)?;

    if let Some(want) = want {
        if hosts.contains(&want) {
            return Ok(HostPlan { name: want, create: false });
        }
        if !writable {
            return Err(format!(
                "no such host: {want}\nAvailable: {}\n\
                 {flake} is fetched read-only, so a new host cannot be added to it.",
                hosts.join(", ")
            ));
        }
        // Creating a machine is not something to infer from a typo.
        if !new_host {
            return Err(format!(
                "no such host: {want}\nAvailable: {}\n\
                 Pass --new to scaffold hosts/{want} instead.",
                hosts.join(", ")
            ));
        }
        validate_host_name(&want)?;
        return Ok(HostPlan { name: want, create: true });
    }


    if assume_yes {
        return Err(format!(
            "--yes needs an explicit --host.\nAvailable: {}",
            hosts.join(", ")
        ));
    }

    println!("\nMachines this flake already describes:\n");
    for (i, h) in hosts.iter().enumerate() {
        println!("  {}) {h}", i + 1);
    }
    // Without this the guided installer could only reinstall a machine the
    // flake already knew about - which is not what anybody boots an installer
    // to do. Installing a new machine was reachable only by remembering
    // --host <name> --new, which the guided flow exists to avoid.
    println!("\n  n) a new machine");

    loop {
        let answer =
            prompt(&format!("\nWhich? [1-{}, or n] ", hosts.len())).map_err(|e| e.to_string())?;

        if answer.eq_ignore_ascii_case("n") {
            if !writable {
                return Err("a new machine needs somewhere to write its hardware, and this\n\
                            flake can neither be written to nor cloned. Install from a\n\
                            local checkout."
                    .into());
            }
            loop {
                let name = prompt(
                    "\nName for this machine? It becomes hosts/<name>/ and the thing you\n\
                     type in every future rebuild: ",
                )
                .map_err(|e| e.to_string())?;
                if hosts.contains(&name) {
                    println!("{name} already exists; pick it from the list, or choose another name.");
                    continue;
                }
                match validate_host_name(&name) {
                    Ok(()) => return Ok(HostPlan { name, create: true }),
                    Err(e) => println!("{e}"),
                }
            }
        }

        match answer.parse::<usize>() {
            Ok(n) if n >= 1 && n <= hosts.len() => {
                return Ok(HostPlan { name: hosts[n - 1].clone(), create: false })
            }
            _ => println!("Enter a number between 1 and {}, or n for a new machine.", hosts.len()),
        }
    }
}

/// The name becomes a directory and a flake attribute, so anything exotic
/// either breaks the path or silently produces an unreachable attribute.
fn validate_host_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "bad host name: {name}\nUse letters, digits, dashes and underscores."
        ));
    }
    Ok(())
}

/// Write `hosts/<name>/hardware.nix` by asking the machine about itself.
///
/// `--show-hardware-config` prints to stdout instead of writing into
/// /etc/nixos. That matters: the tool otherwise also drops a starter
/// `configuration.nix`, a second description of the machine competing with
/// the flake, which is exactly what this design removed.
///
/// `--no-filesystems` omits the mounts, which come from
/// the host's disk.nix instead. Generated mounts are pinned by UUID and
/// mkfs mints new ones on every reinstall, so the generated file would go
/// stale and hang boot; without them it stays correct across reformats.
fn generate_hardware(host_dir: &Path) -> Result<(), String> {
    let out = Command::new("nixos-generate-config")
        .args(["--root", "/mnt", "--show-hardware-config", "--no-filesystems"])
        .output()
        .map_err(|e| format!("nixos-generate-config: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "cannot detect hardware:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // The tool's own header says to make changes in
    // /etc/nixos/configuration.nix - the one file this design deliberately
    // does not have. Leaving it in place would point every future reader at
    // the wrong place, so replace it with where things actually live.
    let body = String::from_utf8_lossy(&out.stdout);
    let body = body
        .lines()
        .skip_while(|l| l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    let header = "\
# Hardware facts for this machine, detected by `kiwami install`.
#
# Generated with --show-hardware-config --no-filesystems, so it carries no
# UUIDs: the mounts are derived from disk.nix and survive a reformat.
#
# Do not edit by hand. `kiwami doctor` compares this against what the machine
# currently reports; regenerate with `kiwami install --regen-hardware`, or:
#   nixos-generate-config --show-hardware-config --no-filesystems
#
# Choices - hostname, users, monitors - belong in default.nix beside this.
";

    fs::create_dir_all(host_dir).map_err(|e| e.to_string())?;
    fs::write(host_dir.join("hardware.nix"), format!("{header}{body}\n"))
        .map_err(|e| e.to_string())
}

/// The choices half of a machine: the things no probe can answer.
fn scaffold_host(host_dir: &Path, name: &str, flake: &str) -> Result<(), String> {
    let body = format!(
        r#"# {name}
#
# Scaffolded by `kiwami install`. Everything here is a choice, not a fact -
# edit it freely, then `nixos-rebuild switch --flake .#{name}`.
#
# hardware.nix beside this file is the facts half, detected at install time.
# Regenerate it with `kiwami doctor` if this machine's hardware changes.
{{ ... }}:

{{
  imports = [
    # Detected at install time. Do not edit; `kiwami doctor` diffs it against
    # what this machine currently reports.
    ./hardware.nix
    # The layout you chose. disko formats from it and fileSystems is derived
    # from it, so the two cannot drift apart.
    ./disk.nix
  ];

  networking.hostName = "{name}";

  # Where this machine rebuilds itself from. It keeps no checkout, so without
  # this `kiwami update` has nowhere to build and says so.
  #
  # It is the flake this machine was installed from, which is very likely the
  # one you want. Point it at your own configuration repository if you keep
  # your hosts somewhere other than where Kiwami itself lives.
  kiwami.flake = "{flake}";

  # The account the desktop belongs to. greetd logs this user in and its home
  # carries the Hyprland and Quickshell config, so a mismatch here is quiet
  # and total: the session comes up on Hyprland's own default config with no
  # bar, because everything was installed for a different account.
  kiwami.user = "{user}";

  # The root is wiped at every boot; only what kiwami.persist declares
  # survives. disk.nix beside this makes the subvolumes that depend on.
  kiwami.ephemeralRoot = true;

  # Log the desktop user in with no password. Off deliberately: it suits a
  # throwaway VM and not a laptop, where it means whoever opens the lid is
  # you. Uncomment only if you know that is what you want.
  # kiwami.autoLogin = true;

  boot.loader.systemd-boot.enable = true;
  # Without this systemd-boot never registers an NVRAM entry and the firmware
  # boots something else, or nothing.
  boot.loader.efi.canTouchEfiVariables = true;

  # The account itself comes from modules/common.nix, which reads
  # kiwami.user above. The password is not set here: users are immutable, so
  # the hash comes from kiwami.passwordFile, which activation seeds with the
  # default and `kiwami passwd` replaces. Setting initialPassword as well is a
  # conflict Nix only warns about, and the file wins - so the line would look
  # like it set the password while doing nothing.

  # When this machine was installed. Kiwami applies the desktop user's home
  # configuration itself; this is the one part of it that belongs to the
  # machine rather than to the distro.
  home-manager.users.{user}.home.stateVersion = "26.05";

  system.stateVersion = "26.05";
}}
"#,
        name = name,
        user = "kiwami",
        flake = flake,
    );
    fs::create_dir_all(host_dir).map_err(|e| e.to_string())?;
    fs::write(host_dir.join("default.nix"), body).map_err(|e| e.to_string())
}

/// Nix builds from what git knows about, not from what is in the directory.
/// A brand new file is invisible to the flake even though `ls` shows it, and
/// the resulting "path does not exist in Git repository" names a file that is
/// plainly there. Staging is enough - no commit required.
fn git_add(repo: &Path, what: &Path) -> Result<(), String> {
    let status = Command::new("git")
        .args(["-C", &repo.to_string_lossy(), "add", "--", &what.to_string_lossy()])
        .status()
        .map_err(|e| format!("git: {e}"))?;
    if !status.success() {
        return Err(format!(
            "git add failed for {}. The flake cannot see untracked files, so \n             the install would fail on a file that is sitting right there.",
            what.display()
        ));
    }
    Ok(())
}

/// A local path we can write into, or nothing. `github:...` and friends are
/// fetched read-only into the store, so a generated file cannot be added to
/// them - that is why installing a new machine needs a clone.
fn local_checkout(flake: &str) -> Option<PathBuf> {
    let path = flake.strip_prefix("path:").unwrap_or(flake);
    if path.contains(':') {
        return None;
    }
    let p = PathBuf::from(path);
    p.join("flake.nix").exists().then_some(p)
}

/// The disks a host's disk.nix declares, resolved to kernel devices.
///
/// Once disko owns the layout, the target is whatever disk.nix names - not
/// whatever a menu selected. A confirmation naming a disk other than the one
/// about to be erased is worse than no confirmation at all.
fn host_disks(flake: &str, host: &str) -> Result<Vec<PathBuf>, String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--json",
            &format!("{flake}#nixosConfigurations.{host}.config.disko.devices.disk"),
            "--apply",
            "ds: map (d: d.device) (builtins.attrValues ds)",
        ])
        .output()
        .map_err(|e| format!("cannot run nix: {e}"))?;
    if !out.status.success() {
        // A host with no disko declaration is not an error; it just means the
        // layout is being chosen here instead.
        return Ok(Vec::new());
    }
    let declared: Vec<String> =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("unexpected output: {e}"))?;

    declared
        .into_iter()
        .map(|d| {
            // by-id paths are symlinks; the safety checks and /proc/mounts
            // both speak kernel names.
            fs::canonicalize(&d)
                .map_err(|e| format!("{host} declares {d}, absent on this machine ({e})"))
        })
        .collect()
}

/// Wipe, partition, format and mount, all from the host's disk.nix.
///
/// Built from the flake rather than fetched: the script that runs is the one
/// this flake's pinned disko produced for this exact host, so it cannot
/// disagree with the fileSystems the same declaration generates.
fn run_disko(flake: &str, host: &str) -> Result<(), String> {
    let attr = format!("{flake}#nixosConfigurations.{host}.config.system.build.diskoScript");
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "build",
            "--no-link",
            "--print-out-paths",
            &attr,
        ])
        .output()
        .map_err(|e| format!("cannot run nix: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cannot build the disk layout:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let script = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if script.is_empty() {
        return Err("disko produced no script".into());
    }
    run(&script, &[])
}

/// Are we running from installer media?
///
/// NixOS stamps VARIANT_ID=installer into /etc/os-release on its installation
/// images, which is the explicit signal. The root filesystem being a tmpfs or
/// overlay is the corroborating one - installed systems boot from a real
/// block device - and covers media that never set the variant.
fn on_installer_media() -> bool {
    let stamped = fs::read_to_string("/etc/os-release")
        .map(|s| s.lines().any(|l| l.trim() == "VARIANT_ID=installer"))
        .unwrap_or(false);

    let ephemeral_root = fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let mut f = l.split_whitespace();
            let src = f.next()?;
            let target = f.next()?;
            let fstype = f.next()?;
            (target == "/").then(|| (src.to_string(), fstype.to_string()))
        })
        .map(|(_, fstype)| fstype == "tmpfs" || fstype == "overlay")
        .unwrap_or(false);

    stamped || ephemeral_root
}


// --- disk layout wizard --------------------------------------------------

pub struct Layout {
    system: PathBuf,
    home: Option<PathBuf>,
    encrypt: bool,
    hibernate: bool,
    ephemeral: bool,
}

/// The four questions, and only these four. Each one is irreversible: every
/// other knob - filesystem, swap file, zram - is a rebuild away, and a
/// question you can answer later does not belong in an installer.
fn ask_layout(all: &[Disk], system: &Disk) -> Result<Layout, String> {
    let others: Vec<&Disk> = all.iter().filter(|d| d.path != system.path).collect();

    let home = if others.is_empty() {
        None
    } else if prompt("\nPut /home on a separate disk? [y/N] ").map_err(|e| e.to_string())?.eq_ignore_ascii_case("y") {
        println!();
        for (i, d) in others.iter().enumerate() {
            println!("  {}) {d}", i + 1);
        }
        loop {
            let a = prompt(&format!("\nWhich disk for /home? [1-{}] ", others.len()))
                .map_err(|e| e.to_string())?;
            match a.parse::<usize>() {
                Ok(n) if n >= 1 && n <= others.len() => break Some(others[n - 1].path.clone()),
                _ => println!("Enter a number between 1 and {}.", others.len()),
            }
        }
    } else {
        None
    };

    // The one choice that cannot be added later without reinstalling.
    let encrypt = prompt("\nEncrypt the disk? [y/N] ").map_err(|e| e.to_string())?.eq_ignore_ascii_case("y");

    // Hibernation is not asked about, because there is no answer to give. It
    // needs a swap area, and with an ephemeral root that area would have to
    // live inside the same btrfs the root is rolled back in - which is not
    // generated yet. Asking, and then refusing the answer, threw a person out
    // of the installer for picking the option that was offered.
    //
    // So swap is zram: compressed swap in RAM, no partition, and changeable
    // with a rebuild if hibernation ever arrives.
    let hibernate = false;

    // Not asked. Every machine this installs is ephemeral: the root is wiped
    // at each boot and only what the config declares survives. Offering the
    // alternative would mean two layouts, two sets of assumptions about where
    // state lives, and a class of bug that only appears on one of them - which
    // is exactly what was found the first time the installer wrote persisted
    // state onto a root that was about to be discarded.
    let ephemeral = true;

    Ok(Layout { system: system.path.clone(), home, encrypt, hibernate, ephemeral })
}

/// Aliases that identify a drive by number rather than by what it is.
///
/// Every drive has several by-id names and all of them are stable, so this is
/// about legibility: `nvme-PM981_NVMe_Samsung_512GB__S3ZHNA0M640707` says what
/// the disk is, `nvme-eui.335a48304d6407070025384100000001` does not.
///
/// The eui form is why this is a function. The rule used to list wwn- and
/// nvme-nvme., which are the two shapes QEMU produces - real NVMe hardware
/// emits nvme-eui., which slipped through and, being the shortest name on
/// offer, won the tiebreak outright. Found on an XPS 13, not in the VM.
fn opaque(name: &str) -> bool {
    name.starts_with("wwn-") || name.starts_with("nvme-eui.") || name.starts_with("nvme-nvme.")
}

/// A name built from the drive's own model and serial, not from the order the
/// kernel happened to find it in. /dev/sdb is a position in a queue: add a
/// disk and it can mean a different drive tomorrow, which is not something to
/// write into a file whose job is deciding what gets erased.
fn stable_device(dev: &Path) -> Result<String, String> {
    let dir = Path::new("/dev/disk/by-id");
    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| fs::canonicalize(e.path()).ok().as_deref() == Some(dev))
        .filter_map(|e| e.file_name().into_string().ok())
        // Partition aliases point at slices, not at the whole disk.
        .filter(|n| !n.contains("-part"))
        .collect();

    if names.is_empty() {
        return Err(format!(
            "{} has no /dev/disk/by-id entry, so there is no stable name to write.\n\
             Kernel names like {} can point at a different drive after adding one.",
            dev.display(),
            dev.display()
        ));
    }

    names.sort_by_key(|n| (opaque(n), n.len(), n.clone()));
    Ok(format!("/dev/disk/by-id/{}", names[0]))
}

fn ram_gib() -> Result<u64, String> {
    let meminfo = fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
    let kb: u64 = meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .ok_or("cannot read MemTotal")?;
    // Round up: a swap area smaller than RAM cannot hold the hibernation image.
    Ok(kb.div_ceil(1024 * 1024).max(1))
}

/// The filesystem sitting at the bottom of whatever nesting there is.
fn fs_content(mountpoint: &str) -> Nix {
    nix::attrs(vec![
        field("type", s("filesystem")),
        field("format", s("ext4")),
        field("mountpoint", s(mountpoint)),
    ])
}

/// Where the hibernation image gets written, so it has to be inside the
/// encrypted container when there is one - an unencrypted swap partition
/// holds a verbatim copy of everything that was in RAM.
fn swap_content() -> Nix {
    nix::attrs(vec![
        field("type", s("swap")),
        noted(
            "resumeDevice",
            "Resuming reads the image back from here, so the kernel is told\nwhich device holds it.",
            raw("true"),
        ),
    ])
}

fn luks(name: &str, inner: Nix, extra: Vec<Field>) -> Nix {
    let mut fields = vec![
        field("type", s("luks")),
        field("name", s(name)),
        field("settings.allowDiscards", raw("true")),
    ];
    fields.extend(extra);
    fields.push(field("content", inner));
    nix::attrs(fields)
}

fn gpt(partitions: Vec<Field>) -> Nix {
    nix::attrs(vec![field("type", s("gpt")), field("partitions", nix::attrs(partitions))])
}

fn esp() -> Field {
    field(
        "ESP",
        nix::attrs(vec![
            field("size", s("1G")),
            field("type", s("EF00")),
            field(
                "content",
                nix::attrs(vec![
                    field("type", s("filesystem")),
                    field("format", s("vfat")),
                    field("mountpoint", s("/boot")),
                    field("mountOptions", Nix::List(vec![s("umask=0077")])),
                ]),
            ),
        ]),
    )
}

/// The btrfs tree an ephemeral root needs: the wiped subvolume, the store, and
/// the warehouse everything declared is bound out of.
///
/// A blank snapshot of @root is taken in a postCreateHook, which is the one
/// moment @root is empty - disko has just made it and nixos-install has not
/// run. Taken later, "blank" would contain an entire system.
fn ephemeral_btrfs(inner_of: Option<&str>, swap_gib: u64) -> Nix {
    let subvol = |mount: &str| {
        nix::attrs(vec![
            field("mountpoint", s(mount)),
            field("mountOptions", Nix::List(vec![s("compress=zstd"), s("noatime")])),
        ])
    };
    let hook = match inner_of {
        // Inside LUKS the filesystem is on the mapper device, not the
        // partition, so the hook has to open it the way disko just did.
        Some(name) => format!(
            "MNTPOINT=$(mktemp -d)\n             mount \"/dev/mapper/{name}\" \"$MNTPOINT\" -o subvol=/\n             trap 'umount \"$MNTPOINT\"; rm -rf \"$MNTPOINT\"' EXIT\n             btrfs subvolume snapshot -r \"$MNTPOINT/@root\" \"$MNTPOINT/@root-blank\"\n"
        ),
        None => "MNTPOINT=$(mktemp -d)\n                 mount \"$device\" \"$MNTPOINT\" -o subvol=/\n                 trap 'umount \"$MNTPOINT\"; rm -rf \"$MNTPOINT\"' EXIT\n                 btrfs subvolume snapshot -r \"$MNTPOINT/@root\" \"$MNTPOINT/@root-blank\"\n"
            .to_string(),
    };
    nix::attrs(vec![
        field("type", s("btrfs")),
        field("extraArgs", Nix::List(vec![s("-f")])),
        field(
            "subvolumes",
            nix::attrs(vec![
                field("\"@root\"", subvol("/")),
                field("\"@nix\"", subvol("/nix")),
                noted(
                    "\"@persist\"",
                    "Everything declared is bound out of here, including the home\npaths - so this is what to snapshot or back up.",
                    subvol("/persist"),
                ),
                noted(
                    "\"@swap\"",
                    "Headroom, not hibernation - a second tier under zram for when\ncold pages stop compressing well. A file rather than a\npartition because nothing here needs a stable resume offset,\nand a file can be resized or dropped without repartitioning.\nIts own subvolume, uncompressed and never snapshotted, which\nis what a swapfile on btrfs requires.",
                    nix::attrs(vec![
                        field("mountpoint", s("/swap")),
                        field("mountOptions", Nix::List(vec![s("noatime")])),
                        field("swap.swapfile.size", s(&format!("{swap_gib}G"))),
                    ]),
                ),
            ]),
        ),
        field("postCreateHook", Nix::Raw(format!("''\n{hook}          ''"))),
    ])
}

fn render_disk_nix(l: &Layout) -> Result<String, String> {
    let system = stable_device(&l.system)?;

    // Half of RAM, within reason. Enough to be a real second tier under zram
    // without taking a bite out of the disk for a machine that will rarely
    // touch it; the file can be resized later without repartitioning.
    let swap_gib = (ram_gib()? / 2).clamp(2, 16);
    let mut devices: Vec<Field> = Vec::new();
    let mut disks: Vec<Field> = Vec::new();

    // Four shapes, from two independent choices. Composing values rather than
    // text is what keeps this from being four hand-written templates that
    // drift apart.
    let (root_partitions, lvm) = match (l.encrypt, l.hibernate) {
        // Everything inside one encrypted container, split by LVM. This is
        // the only layout where hibernation and encryption coexist safely:
        // swap is a logical volume inside the LUKS device, so the RAM image
        // is encrypted by construction rather than by a second passphrase.
        (true, true) => {
            let vg = nix::attrs(vec![
                field("type", s("lvm_vg")),
                field(
                    "lvs",
                    nix::attrs(vec![
                        field(
                            "swap",
                            nix::attrs(vec![
                                field("size", s(&format!("{}G", ram_gib()?))),
                                field("content", swap_content()),
                            ]),
                        ),
                        field(
                            "root",
                            nix::attrs(vec![
                                field("size", s("100%FREE")),
                                field("content", fs_content("/")),
                            ]),
                        ),
                    ]),
                ),
            ]);
            let pv = nix::attrs(vec![field("type", s("lvm_pv")), field("vg", s("pool"))]);
            (
                vec![field(
                    "root",
                    nix::attrs(vec![
                        field("size", s("100%")),
                        field("content", luks("cryptroot", pv, vec![])),
                    ]),
                )],
                Some(nix::attrs(vec![field("pool", vg)])),
            )
        }
        (true, false) => (
            vec![field(
                "root",
                nix::attrs(vec![
                    field("size", s("100%")),
                    field(
                        "content",
                        if l.ephemeral {
                            luks("cryptroot", ephemeral_btrfs(Some("cryptroot"), swap_gib), vec![])
                        } else {
                            luks("cryptroot", fs_content("/"), vec![])
                        },
                    ),
                ]),
            )],
            None,
        ),
        (false, true) => (
            vec![
                field(
                    "swap",
                    nix::attrs(vec![
                        field("size", s(&format!("{}G", ram_gib()?))),
                        field("content", swap_content()),
                    ]),
                ),
                field(
                    "root",
                    nix::attrs(vec![field("size", s("100%")), field("content", fs_content("/"))]),
                ),
            ],
            None,
        ),
        (false, false) => (
            vec![field(
                "root",
                nix::attrs(vec![
                    field("size", s("100%")),
                    field(
                        "content",
                        if l.ephemeral { ephemeral_btrfs(None, swap_gib) } else { fs_content("/") },
                    ),
                ]),
            )],
            None,
        ),
    };

    let mut system_partitions = vec![esp()];
    system_partitions.extend(root_partitions);
    disks.push(field(
        "system",
        nix::attrs(vec![
            field("type", s("disk")),
            field("device", s(&system)),
            field("content", gpt(system_partitions)),
        ]),
    ));

    if let Some(dev) = &l.home {
        let home_dev = stable_device(dev)?;
        let inner = fs_content("/home");
        let content = if l.encrypt {
            // A second passphrase prompt every boot is the default outcome,
            // and it is avoidable: root is already decrypted by the time this
            // is unlocked, so a key stored inside it opens this one silently.
            // The passphrase stays as a second keyslot, so losing the file is
            // recoverable.
            luks(
                "crypthome",
                inner,
                vec![
                    noted(
                        "initrdUnlock",
                        "Not needed to boot, so it is unlocked later - which is what\nmakes the key on the decrypted root reachable at all.",
                        raw("false"),
                    ),
                    field("settings.keyFile", s(KEYFILE)),
                    field("additionalKeyFiles", Nix::List(vec![s(KEYFILE)])),
                ],
            )
        } else {
            inner
        };
        disks.push(field(
            "home",
            nix::attrs(vec![
                field("type", s("disk")),
                field("device", s(&home_dev)),
                field(
                    "content",
                    gpt(vec![field(
                        "home",
                        nix::attrs(vec![field("size", s("100%")), field("content", content)]),
                    )]),
                ),
            ]),
        ));
    }

    devices.push(field("disk", nix::attrs(disks)));
    if let Some(vg) = lvm {
        devices.push(field("lvm_vg", vg));
    }

    let body = nix::attrs(vec![field("disko.devices", nix::attrs(devices))]).render(0);

    let mut notes = String::new();
    if l.encrypt && l.home.is_some() {
        notes.push_str(&format!(
            "#\n# /home unlocks from {KEYFILE}, which lives on the encrypted root, so\n\
             # there is one passphrase prompt rather than two. Create it before\n\
             # installing, or the format step has nothing to enrol.\n"
        ));
    }
    if l.encrypt && l.hibernate {
        notes.push_str(
            "#\n# Swap is a logical volume inside the LUKS container, not a partition\n\
             # beside it: hibernation writes all of RAM there, and an unencrypted\n\
             # swap area would undo the encryption for anything that was in memory.\n",
        );
    }
    if !l.hibernate {
        // What is actually generated, which is both. The note used to say
        // "no swap ... can be joined by a swapfile with a rebuild" - written
        // when the swapfile was optional, and left behind when it stopped
        // being. It then described a machine that had 8G of swapfile mounted
        // while the comment above it said there was none.
        notes.push_str(&format!(
            "#\n# Swap is zram plus a {swap_gib}G swapfile on @swap, not a partition:\n\
             # zram first - compressed swap in RAM, costing no disk - and the file\n\
             # underneath it as headroom for pages that stop compressing well.\n\
             # Neither needs a stable resume offset, so both can be resized or\n\
             # dropped with a rebuild, unlike this file.\n"
        ));
    }

    Ok(format!(
        r#"# Disk layout for this machine, written by `kiwami install`.
#
# One declaration, two uses: disko formats from it, and fileSystems is derived
# from the same tree - so what gets erased and what gets mounted cannot drift
# apart.
#
# Devices are named by id, not /dev/sdX: kernel names follow the order disks
# are found in, so adding a drive can silently repoint this at another one.
{notes}#
# Editing this after installing does not repartition anything. It describes a
# disk that already exists.
{{ ... }}:

{body}
"#
    ))
}

/// Show the generated layout and let it be edited before anything is erased.
///
/// This is what keeps the four questions from ever needing to become forty:
/// LUKS variants, btrfs subvolumes, RAID and separate /var stay documented
/// disko config that you edit, rather than menus that have to be designed,
/// tested and trusted.
fn review_layout(path: &Path, assume_yes: bool) -> Result<(), String> {
    let shown = fs::read_to_string(path).map_err(|e| e.to_string())?;
    println!("\nWrote {}:\n", path.display());
    for line in shown.lines().filter(|l| !l.trim_start().starts_with('#')) {
        println!("  {line}");
    }

    if assume_yes {
        return Ok(());
    }

    loop {
        let choice = prompt("\n[i] install with this   [e] edit first   [a] abort: ")
            .map_err(|e| e.to_string())?;
        match choice.as_str() {
            "i" | "I" => return Ok(()),
            "a" | "A" => return Err("aborted; nothing was written to any disk".into()),
            "e" | "E" => {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".into());
                let status = Command::new(&editor)
                    .arg(path)
                    .status()
                    .map_err(|e| format!("{editor}: {e}"))?;
                if !status.success() {
                    println!("    {editor} exited non-zero; file left as it was");
                }
                return review_layout(path, assume_yes);
            }
            _ => println!("Enter i, e or a."),
        }
    }
}

/// A hardware.nix that is enough to evaluate, and nothing more.
///
/// The real one is generated after mounting, but the flake has to evaluate
/// *before* that: disko's script is built from this very host, so a directory
/// containing only disk.nix cannot be formatted from. It also means an
/// install abandoned midway leaves a host that still evaluates rather than
/// breaking the whole flake.
fn placeholder_hardware(host_dir: &Path) -> Result<(), String> {
    let system = match std::env::consts::ARCH {
        "aarch64" => "aarch64-linux",
        "x86_64" => "x86_64-linux",
        other => return Err(format!("unsupported architecture: {other}")),
    };
    fs::write(
        host_dir.join("hardware.nix"),
        format!(
            "# Placeholder. `kiwami install` replaces this with detected hardware\n             # once the target is mounted; if you are reading it after a finished\n             # install, that step did not run.\n             {{ lib, ... }}:\n\n{{\n  nixpkgs.hostPlatform = lib.mkDefault \"{system}\";\n}}\n"
        ),
    )
    .map_err(|e| e.to_string())
}

/// Create the key that unlocks a second encrypted disk.
///
/// Written before disko runs, because disko enrols it into the new volume's
/// keyslot while formatting - and copied onto the installed root afterwards,
/// since that is where it has to be at every subsequent boot.
fn make_keyfile() -> Result<(), String> {
    let path = Path::new(KEYFILE);
    if path.exists() {
        return Ok(());
    }
    let dir = path.parent().ok_or("bad key path")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // 64 bytes of kernel randomness, read exactly rather than slurped.
    use std::io::Read;
    let mut buf = [0u8; 64];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    fs::write(path, buf).map_err(|e| e.to_string())?;

    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

/// Put the key on the installed system, where every later boot needs it.
fn install_keyfile() -> Result<(), String> {
    let target = target_state_path(KEYFILE);
    let dir = target.parent().ok_or("bad key path")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::copy(KEYFILE, &target).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

/// Turn a flake reference into something git can clone.
///
/// Only the shapes worth guessing at. Anything else returns None and the
/// caller says what it needs rather than inventing a URL.
fn clone_url(flake: &str) -> Option<String> {
    if let Some(rest) = flake.strip_prefix("github:") {
        let path = rest.split(['?', '#']).next()?;
        let parts: Vec<&str> = path.split('/').collect();
        // owner/repo, optionally owner/repo/ref - the ref is not part of the
        // URL, and cloning the default branch is the sane reading.
        if parts.len() >= 2 {
            return Some(format!("https://github.com/{}/{}", parts[0], parts[1]));
        }
        None
    } else if flake.starts_with("https://") || flake.starts_with("git+https://") {
        Some(flake.trim_start_matches("git+").split(['?', '#']).next()?.to_string())
    } else {
        None
    }
}

/// Clone a remote flake so there is somewhere to write this machine into.
///
/// A fetched flake lands read-only in the store, so a new host cannot be added
/// to it. That was previously an error telling you to clone it yourself, which
/// is a strange thing to be told by a program that knows the URL.
fn clone_flake(flake: &str, assume_yes: bool) -> Result<PathBuf, String> {
    let url = clone_url(flake).ok_or_else(|| {
        format!("{flake} is not a local checkout and cannot be cloned automatically.\n\
                 Clone it yourself, then install with --flake <path>.")
    })?;

    // Beside the invoking user's home when there is one, so it lands where a
    // person would look for it.
    let home = std::env::var("SUDO_USER")
        .map(|u| PathBuf::from("/home").join(u))
        .unwrap_or_else(|_| PathBuf::from("/root"));
    let dest = home.join("kiwami");

    if dest.join("flake.nix").exists() {
        println!("    using the checkout already at {}", dest.display());
        return Ok(dest);
    }

    if !assume_yes {
        let answer = prompt(&format!(
            "\nA new machine needs a writable checkout to record its hardware in.\n\
             Clone {url}\n  into {}? [Y/n] ",
            dest.display()
        ))
        .map_err(|e| e.to_string())?;
        if !(answer.is_empty() || answer.eq_ignore_ascii_case("y")) {
            return Err("no checkout to write into".into());
        }
    }

    println!("==> cloning {url}");
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    run("git", &["clone", "--depth", "1", &url, &dest.to_string_lossy()])?;
    if let Ok(user) = std::env::var("SUDO_USER") {
        let _ = run("chown", &["-R", &format!("{user}:users"), &dest.to_string_lossy()]);
    }
    Ok(dest)
}

///
/// With an ephemeral root, anything the installer writes into /mnt is on the
/// subvolume that gets wiped - and if the path is also declared as persisted,
/// the first boot deletes it and then masks it with an empty bind mount from
/// /persist. Declaring a path makes writing it to the root strictly worse
/// than not declaring it at all.
///
/// So state goes to /mnt/persist/<path> when the target has a /persist, and
/// to /mnt/<path> when it does not. Detected by looking rather than by asking
/// the config: /persist being mounted is the fact that matters.
/// Offer to put a previous machine's identity back, before the first boot.
///
/// This is the reason a reinstall is cheap. Without it a fresh machine comes
/// up as a stranger: no wifi, no tailnet, no keys, no tokens - and every one
/// of them has to be re-established by hand, from a browser, one at a time.
/// `kiwami auth` will tell you what is missing, which is not the same as not
/// having lost it.
///
/// Runs after the password seeding and the network carry-over, so what comes
/// out of the backup wins over what the installer invented a minute ago.
fn offer_restore(guided: bool, assume_yes: bool) -> Result<bool, String> {
    if !guided || assume_yes {
        return Ok(false);
    }
    if !insist("\nRestore this machine from a backup? [y/n] ")? {
        println!("\n    skipping the restore. To do it later, from the installer or");
        println!("    the booted machine:  sudo kiwami snapshot restore --identity");
        return Ok(false);
    }

    let dest = PathBuf::from("/tmp/kiwami-backup.env");
    let cred = if fetch_from_bitwarden(&dest)? {
        dest
    } else {
        match wait_for_credentials()? {
            Some(p) => p,
            None => {
                println!("    skipped - nothing arrived");
                return Ok(false);
            }
        }
    };

    // Into the target as well, so the installed machine already knows where
    // its backups live and the daily timer works from the first boot. The
    // alternative - restoring and then making you run setup again - would
    // leave a machine that has its history back but is not adding to it.
    let dest = target_state_path("var/lib/kiwami/backup/env");
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::copy(&cred, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o600));

    // /mnt, not /: the snapshot holds absolute /persist/... paths, so this
    // lands them on the disk that was just formatted rather than on the live
    // medium we are running from.
    println!("\n==> restoring identity into /mnt/persist");
    crate::snapshot::restore_with(&cred.to_string_lossy(), "/mnt", true)?;
    println!("    wifi, keys, tailnet and tokens are back");
    println!("    the rest follows after boot: sudo kiwami snapshot restore");
    Ok(true)
}

/// A yes/no question that will not accept a shrug.
///
/// The ordinary prompts treat anything that is not "y" as no, which is fine
/// for a question you can ask again. It is not fine here: an install runs for
/// minutes with no output, so a stray Enter pressed while waiting sits in the
/// terminal buffer and answers the next question the instant it appears. That
/// silently skipped the restore - the step that makes a reinstall cheap - and
/// went straight to "done", leaving a machine with no keys, no tokens and no
/// wifi, and no sign anything had been asked.
///
/// Repeating the question costs a keystroke. Losing the answer costs an hour.
fn insist(question: &str) -> Result<bool, String> {
    for _ in 0..5 {
        let answer = prompt(question).map_err(|e| e.to_string())?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            "" => println!("    (nothing typed - answer y or n)"),
            other => println!("    (did not understand {other:?} - answer y or n)"),
        }
    }
    // Five unclear answers is a terminal talking to itself, not a person.
    Ok(false)
}

/// Read the credentials out of a Bitwarden note.
///
/// The point is that nothing long gets typed. An R2 secret is 64 random
/// characters, and a person retyping it from a phone at 1am is the failure
/// mode that made every earlier version of this design bad - a memorable
/// master password is the trade, and it is a good one.
///
/// The usual objection to `bw` is blast radius: unlocking derives a session
/// key that decrypts the whole vault, so anything running as you can read
/// every secret you own. That is a real concern on a daily driver and a much
/// smaller one here - this is live media running only what we shipped, the
/// session lives in tmpfs, and the machine is minutes from being wiped. The
/// vault is locked again the moment the note has been read.
///
/// One note holds the whole environment file:
///
///     RESTIC_REPOSITORY=s3:https://...
///     RESTIC_PASSWORD=...
///     AWS_ACCESS_KEY_ID=...
///     AWS_SECRET_ACCESS_KEY=...
pub(crate) fn fetch_from_bitwarden(dest: &Path) -> Result<bool, String> {
    if !have_cmd("bw") {
        return Ok(false);
    }
    let answer = prompt("\nFetch the credentials from Bitwarden? [Y/n] ")
        .map_err(|e| e.to_string())?;
    if answer.eq_ignore_ascii_case("n") {
        return Ok(false);
    }

    // Three tries, because a mistyped note name is not a reason to lose the
    // whole route. The first real run of this used a note called
    // "kiwami-backups-xps" while the default assumed "kiwami-backup" - one
    // wrong character would have dropped straight through to copying a file
    // by hand, which is the thing all of this exists to avoid.
    for attempt in 0..3 {
        if let Some(dir) = dest.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let item = prompt("Note name [kiwami-backup]: ").map_err(|e| e.to_string())?;
        let item = if item.trim().is_empty() { "kiwami-backup".to_string() } else { item };
        if try_bitwarden_note(&item, dest)? {
            return Ok(true);
        }
        if attempt < 2 {
            println!("    no note by that name - try again, or Ctrl-C to skip");
        }
    }
    println!("    falling back");
    Ok(false)
}

/// One attempt at one note name.
fn try_bitwarden_note(item: &str, dest: &Path) -> Result<bool, String> {

    // bw prints the session key on stdout and its prompts on stderr, so the
    // command substitution captures the key while you still see what it is
    // asking - stderr is inherited, so no redirect is needed and none is used:
    // 2>/dev/tty would have added a way to fail on a console that does not
    // have one.
    //
    // unlock first: on a machine that has already logged in, `login` refuses
    // rather than unlocking.
    //
    // The empty-session guard is not defensive programming for its own sake:
    // `bw login --raw` was observed exiting 0 with no session at all when its
    // input ended early, which would otherwise sail past the `||` and reach
    // `--session ""` with a confusing error much later.
    let script = format!(
        "set -o pipefail\n\
         s=$(bw unlock --raw) || s=$(bw login --raw) || exit 1\n\
         [ -n \"$s\" ] || exit 1\n\
         bw get notes {item} --session \"$s\" > {dest} || exit 1\n\
         bw lock >/dev/null 2>&1 || true\n",
        item = shell_quote(item),
        dest = shell_quote(&dest.to_string_lossy()),
    );

    let status = Command::new("bash").arg("-c").arg(&script).status();
    let ok = matches!(status, Ok(st) if st.success())
        && dest.metadata().map(|m| m.len() > 0).unwrap_or(false);

    if ok {
        // 0600 before anything else touches it: the note is the whole
        // credential set, and /tmp on live media is world-readable.
        let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o600));
        println!("    got the credentials from Bitwarden");
        return Ok(true);
    }

    let _ = fs::remove_file(dest);
    Ok(false)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn have_cmd(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Wait for the credentials to be copied in, when Bitwarden is not the route.
///
/// One fallback, not a menu: scp from a machine that already has them, over
/// the tailnet this installer just joined. Taildrop was considered and
/// dropped - it needs enabling on the tailnet, files have to be collected
/// rather than arriving, and it only moves files, so the credentials would
/// have to already exist as a file on the phone rather than as the vault item
/// they actually live in.
fn wait_for_credentials() -> Result<Option<PathBuf>, String> {
    let dest = PathBuf::from("/tmp/kiwami-backup.env");
    let host = Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this-machine".into());

    println!("\nWaiting for the credentials. From a machine that has them:");
    println!("\n  scp backup.env {host}:/tmp/kiwami-backup.env");
    println!("\nCtrl-C to skip the restore.\n");

    // Ten minutes: long enough to go and find the other machine, short enough
    // that an abandoned install does not sit here forever.
    for i in 0..120 {
        if dest.is_file() {
            let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o600));
            println!("    got it");
            return Ok(Some(dest));
        }
        if i % 6 == 5 {
            println!("    still waiting ({}s)", (i + 1) * 5);
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    Ok(None)
}

fn target_state_path(rel: &str) -> PathBuf {
    let rel = rel.trim_start_matches('/');
    let persist = Path::new("/mnt/persist");
    if persist.is_dir() {
        persist.join(rel)
    } else {
        PathBuf::from("/mnt").join(rel)
    }
}

/// The system this medium carries, if it carries one for this host.
///
/// Two files: the host it was built for, and where its closure is. The host
/// is checked rather than assumed - a stick built for one machine must not
/// quietly install that machine onto another.
fn prebuilt_system(host: &str) -> Option<String> {
    let want = fs::read_to_string("/iso/kiwami/host").ok()?;
    if want.trim() != host {
        println!(
            "    (this medium carries {}, not {host} - installing from the flake)",
            want.trim()
        );
        return None;
    }
    let path = fs::read_to_string("/iso/kiwami/system").ok()?.trim().to_string();
    // Present in the store, not merely named: the signpost is a text file and
    // could outlive the thing it points at.
    if Path::new(&path).exists() {
        Some(path)
    } else {
        println!("    (this medium names a system it does not carry - installing from the flake)");
        None
    }
}

/// Offer to put this host's configuration on the flake, before rebooting.
///
/// The machine now has a layout, and hardware, that GitHub does not. Until
/// that is pushed the machine cannot rebuild itself - `kiwami update` would
/// build a system describing the disk this one replaced - and nothing but the
/// disk in front of you knows how it was made.
///
/// It asks rather than doing it: pushing to someone's repository is an
/// outward-facing act, and an installer has no business taking it silently.
fn offer_host_push(checkout: &Option<PathBuf>, host: &str, assume_yes: bool) -> Result<(), String> {
    let Some(repo) = checkout else { return Ok(()) };

    // Asked before it is announced.
    //
    // This used to state "hosts/<host> exists only on this machine" as a fact,
    // without checking. When the host had already been pushed - from a laptop,
    // before the install - the push that followed correctly reported nothing
    // to do, and the pair read as a failure: a confident claim, then a refusal
    // to act on it. Half an hour went into looking for a bug in the push. The
    // push was right; the sentence above it was not.
    match crate::host::differs_from_origin(repo, host) {
        Ok(false) => {
            println!("\n==> hosts/{host} already matches the flake - nothing to push");
            return Ok(());
        }
        // Unreachable network, no origin: fall through and let the push say
        // so properly rather than deciding here on a guess.
        Err(_) => {}
        Ok(true) => {}
    }

    println!("\n==> hosts/{host} exists only on this machine");
    if assume_yes {
        println!("    push it with: sudo kiwami host push");
        return Ok(());
    }
    let answer = prompt("\nPush it to the flake now? [Y/n] ").map_err(|e| e.to_string())?;
    if answer.eq_ignore_ascii_case("n") {
        println!("    later, then:  sudo kiwami host push");
        return Ok(());
    }

    // gh first, because the push needs it and finding that out afterwards
    // means doing the whole thing twice. The browser approval happens on
    // whatever device you have - nothing long gets typed here either.
    if !gh_authenticated() {
        println!("\n==> github login");
        let status = Command::new("gh").args(["auth", "login"]).status();
        if !matches!(status, Ok(s) if s.success()) && !gh_authenticated() {
            println!("    not logged in - push it after rebooting:");
            println!("      sudo kiwami host push");
            return Ok(());
        }
    }

    // host push reads KIWAMI_REPO, which here is the clone made for this
    // install rather than a checkout on the installed system - that machine
    // deliberately keeps none.
    std::env::set_var("KIWAMI_REPO", repo);
    match crate::host::push(Some(host.to_string()), false) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Not fatal. The system is installed and works; the config is
            // simply not upstream yet, and saying so beats failing an install
            // that succeeded.
            println!("    could not push: {e}");
            println!("    after rebooting:  sudo kiwami host push");
            Ok(())
        }
    }
}

fn gh_authenticated() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// What changed about the layout, in the terms the questions were asked in.
///
/// A line-by-line diff was the obvious thing and was useless: the previous
/// file may be hand-written while the new one is generated, so indentation
/// differs on nearly every line and "disko.devices = {" shows up as an
/// addition. The real change - encryption appearing - drowns in it, and a
/// diff nobody can read is a confirmation nobody really gives.
///
/// The old layout is read back from the file by looking for the few things
/// that distinguish one layout from another, which is crude and survives
/// reformatting.
fn describe_layout_change(before: &str, layout: &Layout) {
    let was_encrypted = before.contains("type = \"luks\"");
    let had_home = before.contains("home = {") || before.contains("\"home\"");

    let line = |label: &str, was: bool, now: bool| {
        if was == now {
            println!("    {label}: {}", if now { "yes" } else { "no" });
        } else {
            println!(
                "    \x1b[33m{label}: {} -> {}\x1b[0m",
                if was { "yes" } else { "no" },
                if now { "yes" } else { "no" }
            );
        }
    };

    line("encrypted", was_encrypted, layout.encrypt);
    line("separate /home disk", had_home, layout.home.is_some());
    println!("    system disk: {}", layout.system.display());
    if let Some(h) = &layout.home {
        println!("    home disk:   {}", h.display());
    }
}

/// The normal users a host declares, straight from its own configuration.
fn target_users(flake: &str, host: &str) -> Vec<String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--json",
            &format!("{flake}#nixosConfigurations.{host}.config.users.users"),
            "--apply",
            "us: builtins.filter (n: us.${n}.isNormalUser or false) (builtins.attrNames us)",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or_default(),
        _ => Vec::new(),
    }
}


/// Give the new machine a password it can be logged into.
///
/// Users are immutable, so there is no initialPassword to fall back on: with
/// no hash file the account simply has no password, and that is discovered at
/// a greeter on a machine that has just been installed.
///
/// A known default rather than a prompt, so an unattended install still
/// works. `kiwami doctor` reports it as a default until it is changed, which
/// is the honest trade - a machine you can log into and a visible nag, rather
/// than one you cannot.
const DEFAULT_PASSWORD: &str = "kiwami";

fn seed_password(flake: &str, host: &str) -> Result<(), String> {
    let dir = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--raw",
            &format!("{flake}#nixosConfigurations.{host}.config.kiwami.passwordFile"),
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "/var/lib/kiwami/passwords".to_string());

    let users = target_users(flake, host);
    for user in &users {
        let target = target_state_path(&format!("{}/{}", dir.trim_start_matches('/'), user));
        if target.exists() {
            continue;
        }
        let parent = target.parent().ok_or("bad password path")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;

        let out = Command::new("mkpasswd")
            .args(["-m", "sha-512", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.as_mut().unwrap().write_all(DEFAULT_PASSWORD.as_bytes())?;
                c.wait_with_output()
            })
            .map_err(|e| format!("mkpasswd: {e}"))?;
        if !out.status.success() {
            return Err("mkpasswd failed".into());
        }
        fs::write(&target, &out.stdout).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        println!("    {user}: default password, change it with `kiwami passwd`");
    }
    Ok(())
}



/// Put the new system first in the firmware's boot order.
///
/// The order lives in NVRAM, not on the disk, so wiping a drive leaves every
/// old entry in place - a Windows entry pointing at files that no longer
/// exist can still sit ahead of the one just created. systemd-boot appends
/// itself at the end, so straight after an install the working entry is last.
///
/// Most firmware falls through to the next entry when one fails, so this is
/// often invisible; edk2 in QEMU instead drops to an EFI shell, and there is
/// no reason to find out which kind a given machine is while standing in
/// front of it.
///
/// vm/scripts/install.sh has done this since the early days, which is exactly
/// why the gap went unnoticed: every VM install came out corrected and the
/// installer's silence never showed.
fn fix_boot_order() -> Result<(), String> {
    // Which partition is actually the ESP of the machine just installed. The
    // label is not enough: a reinstall leaves the previous install's entry in
    // NVRAM, also called "Linux Boot Manager", pointing at a partition that
    // was destroyed minutes ago. Matching by name picked that one and put a
    // dead entry first - seen on a second install onto the same disk.
    let esp = Command::new("findmnt")
        .args(["-no", "SOURCE", "/mnt/boot"])
        .output()
        .map_err(|e| format!("findmnt: {e}"))?;
    let esp = String::from_utf8_lossy(&esp.stdout).trim().to_string();
    if esp.is_empty() {
        println!("    cannot tell which partition is the ESP; leaving the boot order alone");
        return Ok(());
    }
    let uuid = Command::new("blkid")
        .args(["-s", "PARTUUID", "-o", "value", &esp])
        .output()
        .map_err(|e| format!("blkid: {e}"))?;
    let uuid = String::from_utf8_lossy(&uuid.stdout).trim().to_lowercase();
    if uuid.is_empty() {
        println!("    {esp} has no PARTUUID; leaving the boot order alone");
        return Ok(());
    }

    // efibootmgr from the installer, not from inside the target.
    //
    // This used to run through `nixos-enter`, which meant the installed system
    // had to ship efibootmgr - and only the four VM hosts happen to declare
    // it. On real hardware the command did not exist, the call failed, and
    // this printed "leaving it alone" and moved on: a first reinstall left the
    // previous install's dead entry first in the boot order, and the machine
    // booted only because the firmware fell through the entries that no longer
    // resolved.
    //
    // The installer already has the tool and the same NVRAM. Reading it here
    // depends on nothing the target chose to install.
    let out = Command::new("efibootmgr")
        .output()
        .map_err(|e| format!("efibootmgr: {e}"))?;
    if !out.status.success() {
        println!("    could not read the boot order; leaving it alone");
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);

    // The entry whose device path carries this exact partition. Unambiguous,
    // however many entries share a label.
    let ours = text.lines().find_map(|l| {
        (l.starts_with("Boot") && l.to_lowercase().contains(&uuid))
            .then(|| l.split_whitespace().next().unwrap_or(""))
            .map(|id| id.trim_start_matches("Boot").trim_end_matches('*').to_string())
    });
    let Some(entry) = ours else {
        println!("    no NVRAM entry points at {esp}; leaving the boot order alone");
        return Ok(());
    };

    // Entries left behind by previous installs onto this same machine.
    //
    // Every install writes a new "Linux Boot Manager" entry, and reformatting
    // the ESP gives it a new partition GUID, so the old entry is never
    // overwritten - it just stays in NVRAM naming a partition that no longer
    // exists. Four installs, four entries, three of them dead. The firmware
    // walks past dead entries, so this stays invisible until the day a live
    // entry sorts below one of them and the machine boots to nothing.
    //
    // Sorting the right entry first, which is all this used to do, does not
    // help: it leaves the pile to keep growing.
    prune_dead_entries(&text, &entry)?;

    // Re-read: the ids are the same, but BootOrder just lost its dead members.
    let text = Command::new("efibootmgr")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or(text.into_owned());

    let order: Vec<String> = text
        .lines()
        .find(|l| l.starts_with("BootOrder:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    if order.first().map(String::as_str) == Some(entry.as_str()) {
        println!("    already first in the boot order");
        return Ok(());
    }

    let mut new = vec![entry.clone()];
    new.extend(order.into_iter().filter(|e| e != &entry));
    let joined = new.join(",");
    run("efibootmgr", &["-o", &joined, "-t", "1"])?;
    println!("    boot order -> {joined} (Boot{entry} is this machine's ESP)");
    Ok(())
}

/// Delete boot entries whose partition is gone.
///
/// Deliberately narrow, because deleting the wrong entry is how a machine
/// stops booting. An entry is removed only when all of these hold:
///
///   - its device path names a GPT partition, so there is a GUID to check.
///     USB sticks appear as MBR or CDROM paths and are never touched;
///   - that GUID is on no block device attached right now;
///   - it is neither the entry we are about to make first, nor the one this
///     installer booted from.
///
/// If the device list cannot be read, nothing is removed. A machine with an
/// unplugged GPT boot disk would see its entry as dead - true of an external
/// system disk, which is why this only ever runs during an install, when the
/// disks that matter are by definition present.
fn prune_dead_entries(text: &str, keep: &str) -> Result<(), String> {
    let live = live_partuuids();
    if live.is_empty() {
        return Ok(());
    }
    let current = text
        .lines()
        .find(|l| l.starts_with("BootCurrent:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    for line in text.lines() {
        let Some(id) = boot_entry_id(line) else { continue };
        if id == keep || id == current {
            continue;
        }
        let Some(guid) = gpt_guid(line) else { continue };
        if live.contains(&guid) {
            continue;
        }
        let label = line.split('\t').nth(1).unwrap_or("").trim();
        println!("    removing Boot{id} ({label}) - its partition no longer exists");
        run("efibootmgr", &["-b", &id, "-B"])?;
    }
    Ok(())
}

/// The four hex digits of a `Boot0001* ...` line. None for BootOrder,
/// BootCurrent, BootNext and Timeout, whose next characters are not hex.
fn boot_entry_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Boot")?;
    let id: String = rest.chars().take(4).collect();
    (id.len() == 4 && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

/// The partition GUID out of a device path like `HD(1,GPT,<guid>,0x800,...)`.
fn gpt_guid(line: &str) -> Option<String> {
    let rest = &line[line.find("GPT,")? + 4..];
    let guid = &rest[..rest.find(',')?];
    (guid.len() == 36).then(|| guid.to_lowercase())
}

/// PARTUUIDs of every partition attached to this machine.
fn live_partuuids() -> std::collections::HashSet<String> {
    Command::new("lsblk")
        .args(["-rno", "PARTUUID"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Carry the installer's network state onto the installed system.
///
/// A wifi network joined during the install, and a tailnet joined to let
/// somebody help with it, both live on the installer's tmpfs. Without this
/// they evaporate at the first reboot: the machine comes up with no way to
/// reach a network, having been connected to one minutes earlier, and whoever
/// was helping is locked out at precisely the moment something might need
/// fixing.
///
/// Both are credentials, so both are copied with their permissions intact.
fn carry_network_state(restored: bool) -> Result<(), String> {
    // Wifi: the installer's profiles win.
    //
    // Both this and the restore write here, and for a while the restore went
    // last, which meant a network joined during the install was overwritten by
    // whatever the backup remembered about that SSID. Usually identical, so
    // nothing looked wrong - but the case where they differ is exactly the
    // case that matters: you retyped the password during the install because
    // the stored one had stopped working, and the reboot silently put the
    // broken one back.
    //
    // A profile that just carried this install is a profile proven to work on
    // this hardware, minutes ago. It outranks a remembered one. Profiles for
    // other networks are untouched, so the restore's list survives alongside.
    let profiles = Path::new("/etc/NetworkManager/system-connections");
    let saved: Vec<_> = fs::read_dir(profiles)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    if !saved.is_empty() {
        let dest = target_state_path("etc/NetworkManager/system-connections");
        let dest = dest.as_path();
        fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        for f in &saved {
            run("cp", &["-a", &f.to_string_lossy(), &dest.to_string_lossy()])?;
        }
        println!("    {} wifi network(s) carried over", saved.len());
    }

    // Tailnet identity: the restore wins.
    //
    // The opposite rule, and for the opposite reason. The installer's node was
    // registered minutes ago to let somebody help with the install; the backed
    // up one is the machine's long-lived identity - the node in your ACLs, the
    // name that already resolves, the machine your other machines trust.
    // Overwriting it would strand that node and register yet another, and the
    // tailnet would collect an xps-2, an xps-3, one per reinstall.
    //
    // Nothing is lost by yielding: the installer's session runs off its own
    // tmpfs and stays up until the reboot regardless of what is written here.
    let state = Path::new("/var/lib/tailscale/tailscaled.state");
    let dest = target_state_path("var/lib/tailscale");
    if restored && dest.join("tailscaled.state").exists() {
        println!("    tailnet identity kept from the backup - it comes back as the same node");
        return Ok(());
    }
    if state.exists() {
        let dest = dest.as_path();
        fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        run("cp", &["-a", &state.to_string_lossy(), &dest.to_string_lossy()])?;
        println!(
            "    tailnet login carried over - this machine stays reachable after reboot.\n\
             \x20   Run `tailscale logout` on it to undo that."
        );
    }
    Ok(())
}

/// Print the detected disks and exit. Used by tests and by anyone wondering
/// what the installer can see.
pub fn list_disks() -> Result<(), String> {
    let disks = disks().map_err(|e| e.to_string())?;
    if disks.is_empty() {
        println!("(no disks found)");
        return Ok(());
    }
    for d in disks {
        println!("{d}");
    }
    Ok(())
}

#[allow(dead_code)]
fn _unused(_: Stdio) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The aliases a real Samsung PM981 in an XPS 13 9380 actually offers,
    /// captured from the machine. QEMU never produced the eui form, so the
    /// preference rule was wrong for a year of testing without anything
    /// failing.
    #[test]
    fn prefers_the_readable_alias_on_real_nvme() {
        let mut names = vec![
            "nvme-eui.335a48304d6407070025384100000001".to_string(),
            "nvme-PM981_NVMe_Samsung_512GB_______S3ZHNA0M640707".to_string(),
            "nvme-PM981_NVMe_Samsung_512GB__S3ZHNA0M640707".to_string(),
            "nvme-PM981_NVMe_Samsung_512GB_______S3ZHNA0M640707_1".to_string(),
        ];
        names.sort_by_key(|n| (opaque(n), n.len(), n.clone()));
        assert_eq!(names[0], "nvme-PM981_NVMe_Samsung_512GB__S3ZHNA0M640707");
    }

    /// The T5 the installer booted from, same machine.
    #[test]
    fn prefers_the_readable_alias_on_usb() {
        let mut names = vec![
            "wwn-0x5002538e00000000".to_string(),
            "usb-Samsung_Portable_SSD_T5_1234567FA273-0:0".to_string(),
        ];
        names.sort_by_key(|n| (opaque(n), n.len(), n.clone()));
        assert_eq!(names[0], "usb-Samsung_Portable_SSD_T5_1234567FA273-0:0");
    }

    /// What the dev VM offers, so the case that did work keeps working.
    #[test]
    fn prefers_the_readable_alias_in_qemu() {
        let mut names = vec![
            "nvme-nvme.1b36-6b6977616d692d746573742d6e766d65-51454d55204e564d65204374726c-00000001".to_string(),
            "nvme-QEMU_NVMe_Ctrl_kiwami-test-nvme".to_string(),
            "nvme-QEMU_NVMe_Ctrl_kiwami-test-nvme_1".to_string(),
        ];
        names.sort_by_key(|n| (opaque(n), n.len(), n.clone()));
        assert_eq!(names[0], "nvme-QEMU_NVMe_Ctrl_kiwami-test-nvme");
    }
}

#[cfg(test)]
mod boot_entry_tests {
    use super::*;

    // Real output from the XPS, captured after four reinstalls onto the same
    // disk. Note Boot0000 and Boot0001 share a GUID: the Windows entry and the
    // first NixOS install both named the partition that install replaced.
    const XPS: &str = "\
BootCurrent: 0002
BootOrder: 0006,0005,0004,0001,0000,0002,0003
Boot0000  Windows Boot Manager\tHD(1,GPT,3bb0c7c6-67ab-4df2-bfe4-8bdb182c2e71,0x800,0x154000)/\\EFI\\Microsoft\\Boot\\bootmgfw.efi
Boot0001* Linux Boot Manager\tHD(1,GPT,3bb0c7c6-67ab-4df2-bfe4-8bdb182c2e71,0x800,0x200000)/\\EFI\\systemd\\systemd-bootx64.efi
Boot0002* UEFI: Samsung Portable SSD T5 0\tPciRoot(0x0)/Pci(0x14,0x0)/USB(12,0)/CDROM(1,0x114,0x6000)0000424f
Boot0003* UEFI: Samsung Portable SSD T5 0, Partition 2\tPciRoot(0x0)/Pci(0x14,0x0)/USB(12,0)/HD(2,MBR,0x9fb6382f,0x114,0x1800)0000424f
Boot0004* Linux Boot Manager\tHD(1,GPT,05c10375-5160-4656-9d91-9054661b4fb1,0x800,0x200000)/\\EFI\\systemd\\systemd-bootx64.efi
Boot0005* Linux Boot Manager\tHD(1,GPT,9257f17d-855a-40e5-bf51-ce11b4903ab6,0x800,0x200000)/\\EFI\\systemd\\systemd-bootx64.efi
Boot0006* Linux Boot Manager\tHD(1,GPT,5bd8c011-0071-4e4a-81e7-cb0a2186e763,0x800,0x200000)/\\EFI\\systemd\\systemd-bootx64.efi";

    #[test]
    fn ids_come_only_from_entries() {
        let ids: Vec<_> = XPS.lines().filter_map(boot_entry_id).collect();
        // BootCurrent and BootOrder must not be mistaken for entries.
        assert_eq!(ids, ["0000", "0001", "0002", "0003", "0004", "0005", "0006"]);
    }

    #[test]
    fn only_gpt_paths_yield_a_guid() {
        let guids: Vec<_> = XPS.lines().filter_map(gpt_guid).collect();
        assert_eq!(guids.len(), 5, "the two USB entries have no GPT partition");
        assert!(guids.contains(&"5bd8c011-0071-4e4a-81e7-cb0a2186e763".to_string()));
    }

    /// The bug this exists to prevent: a stick's entry deleted because its
    /// device path has no GUID to match, or a live entry deleted by accident.
    #[test]
    fn removable_and_live_entries_are_never_candidates() {
        let live = ["5bd8c011-0071-4e4a-81e7-cb0a2186e763".to_string()];
        let doomed: Vec<_> = XPS
            .lines()
            .filter_map(|l| Some((boot_entry_id(l)?, l)))
            .filter(|(id, _)| id != "0006" && id != "0002")
            .filter_map(|(id, l)| Some((id, gpt_guid(l)?)))
            .filter(|(_, g)| !live.contains(g))
            .map(|(id, _)| id)
            .collect();
        assert_eq!(doomed, ["0000", "0001", "0004", "0005"], "only the dead GPT entries");
    }
}

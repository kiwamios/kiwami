mod auth;
mod commands;
mod doctor;
mod install;
mod host;
mod net;
mod nix;
mod remote;
mod passwd;
mod snapshot;
mod wallpaper;
mod paths;
mod theme;
mod update;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kiwami", version, about = "Kiwami desktop control")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage the active theme
    Theme {
        #[command(subcommand)]
        action: ThemeCmd,
    },
    /// Install Kiwami to a disk. Intended to run from the live ISO.
    Install {
        /// Target disk, e.g. /dev/nvme0n1. Prompts if omitted.
        #[arg(long)]
        disk: Option<String>,
        /// Host attribute in the flake to install. Prompts if omitted.
        #[arg(long)]
        host: Option<String>,
        /// Flake to install from
        #[arg(long, default_value = "github:kiwamios/kiwami")]
        flake: String,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Allow running on an already-installed system
        #[arg(long)]
        force: bool,
        /// Scaffold hosts/<name>/ when the flake does not define it yet
        #[arg(long)]
        new: bool,
        /// Re-detect hardware even if hardware.nix is already committed
        #[arg(long)]
        regen_hardware: bool,
        /// Ask the layout questions again and rewrite this host's disk.nix.
        ///
        /// The way a machine changes its disk layout - adding encryption, or
        /// moving /home - since editing the file on a running system does
        /// nothing: it is a recipe for formatting, and the disk is already
        /// formatted.
        #[arg(long)]
        relayout: bool,
        /// Walk through networking and remote access first. What the
        /// installer image starts on boot.
        #[arg(long)]
        guided: bool,
    },
    /// List disks the installer can see, and exit
    Disks,
    /// Set the desktop user's password on a machine with an ephemeral root
    Passwd {
        /// Whose password. Defaults to the invoking user.
        user: Option<String>,
    },
    /// Join a tailnet so this machine can be reached during an install
    Remote {
        /// Disconnect instead of connecting
        #[arg(long)]
        down: bool,
    },
    /// Get online: reports if already connected, otherwise offers wifi
    Net {
        /// Report what is visible and exit; change nothing
        #[arg(long)]
        status: bool,
    },
    /// Report drift from the declared config, and check the desktop is healthy
    Doctor,
    /// Show which tools still need a credential
    Auth {
        #[command(subcommand)]
        action: Option<AuthCmd>,
    },
    /// Rebuild this machine from the flake on GitHub
    Update {
        /// Build an exact commit instead of the newest one - to roll back, or
        /// to put two machines on the same system.
        #[arg(long)]
        commit: Option<String>,
        /// Build it without switching
        #[arg(long)]
        dry: bool,
    },
    /// Build an installer image that already contains this machine
    Image {
        /// Which host's system to embed. Defaults to this machine.
        #[arg(long)]
        host: Option<String>,
        /// Where to leave the result symlink
        #[arg(long, default_value = "./kiwami-image")]
        out: String,
    },
    /// Back /persist up, and put it back
    /// The pictures behind everything, and which one is showing
    Wallpaper {
        #[command(subcommand)]
        action: WallpaperCmd,
    },
    Snapshot {
        #[command(subcommand)]
        action: SnapshotCmd,
    },
    /// This machine's entry in the flake
    Host {
        #[command(subcommand)]
        action: HostCmd,
    },
    /// Emit the actions the launcher should offer, as JSON
    Commands {
        /// Currently the only supported format; present so the shape is explicit.
        #[arg(long, default_value_t = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AuthCmd {
    /// Log in to whatever is missing, one at a time
    Login,
}

#[derive(Subcommand)]
enum WallpaperCmd {
    /// What is there, and which one is showing
    List,
    /// Fetch a URL or copy a file into the wallpaper directory
    Add {
        /// A URL or a path
        source: String,
        /// Show it straight away
        #[arg(long)]
        set: bool,
    },
    /// Show one now. No rebuild.
    Set {
        /// Its name, or enough of it to be unambiguous
        which: String,
    },
    /// Move to the next one, in the order they rotate
    Next,
    /// Delete one
    Remove {
        which: String,
    },
}

#[derive(Subcommand)]
enum SnapshotCmd {
    /// Point this machine at a repository, generating a passphrase if it is
    /// new, and prove a backup can be written and read back.
    Setup {
        /// The restic repository. Prompts if omitted.
        #[arg(long)]
        repository: Option<String>,
    },
    /// Take a backup now, through the same unit the timer uses
    Backup,
    /// What is in the repository
    Status,
    /// Put /persist back from the newest snapshot
    Restore {
        /// Where to write it. /persist on a running machine; /mnt/persist
        /// from the installer, before the first boot.
        #[arg(long, default_value = "/")]
        target: String,
        /// Only what the machine needs to come up as itself - wifi, keys,
        /// the tailnet node, the gh token. The rest can follow later.
        #[arg(long)]
        identity: bool,
        /// Skip the confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum HostCmd {
    /// Push this machine's hosts/<name>/ to a branch and open a pull request.
    ///
    /// The branch is built from origin/main with only that directory in it, so
    /// unrelated local commits cannot be pushed by accident.
    Push {
        /// Which host. Defaults to this machine's hostname.
        name: Option<String>,
        /// Push the branch but do not open a pull request
        #[arg(long)]
        no_pr: bool,
    },
}

#[derive(Subcommand)]
enum ThemeCmd {
    /// List available themes
    List,
    /// Print the active theme
    Current,
    /// Apply a theme (defaults to reapplying the active one)
    Set { name: Option<String> },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Theme { action } => match action {
            ThemeCmd::List => {
                let current = theme::current();
                match theme::list() {
                    Ok(names) if !names.is_empty() => {
                        for n in names {
                            let marker = if current.as_deref() == Some(n.as_str()) { "*" } else { " " };
                            println!("{marker} {n}");
                        }
                    }
                    Ok(_) => eprintln!("no themes found in any of {:?}", paths::theme_search_paths()),
                    Err(e) => {
                        eprintln!("cannot read themes: {e}");
                        return std::process::ExitCode::FAILURE;
                    }
                }
            }
            ThemeCmd::Current => match theme::current() {
                Some(n) => println!("{n}"),
                None => {
                    eprintln!("no theme applied yet");
                    return std::process::ExitCode::FAILURE;
                }
            },
            ThemeCmd::Set { name } => match theme::set(name) {
                Ok(n) => println!("theme: {n}"),
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::FAILURE;
                }
            },
        },
        Cmd::Install { disk, host, flake, yes, force, new, regen_hardware, relayout, guided } => {
            let opts = install::Options {
                disk,
                host,
                flake,
                assume_yes: yes,
                force,
                new_host: new,
                regen_hardware,
                relayout,
                guided,
            };
            if let Err(e) = install::run_install(opts) {
                eprintln!("\ninstall: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Doctor => {
            if doctor::run().is_err() {
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Auth { action } => {
            let login = matches!(action, Some(AuthCmd::Login));
            if let Err(e) = auth::run(login) {
                // An empty message is the "gaps remain" case, already printed
                // as a list. Only a real error is worth a second line.
                if !e.is_empty() {
                    eprintln!("auth: {e}");
                }
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Update { commit, dry } => {
            if let Err(e) = update::run(commit, dry) {
                eprintln!("update: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Image { host, out } => {
            if let Err(e) = update::image(host, out) {
                eprintln!("image: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Wallpaper { action } => {
            let r = match action {
                WallpaperCmd::List => wallpaper::list(),
                WallpaperCmd::Add { source, set } => wallpaper::add(source, set),
                WallpaperCmd::Set { which } => wallpaper::set(which),
                WallpaperCmd::Next => wallpaper::next(),
                WallpaperCmd::Remove { which } => wallpaper::remove(which),
            };
            if let Err(e) = r {
                eprintln!("wallpaper: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Snapshot { action } => match action {
            SnapshotCmd::Setup { repository } => {
                if let Err(e) = snapshot::setup(repository) {
                    eprintln!("snapshot setup: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
            SnapshotCmd::Backup => {
                if let Err(e) = snapshot::backup() {
                    eprintln!("snapshot backup: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
            SnapshotCmd::Status => {
                if let Err(e) = snapshot::status() {
                    eprintln!("snapshot status: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
            SnapshotCmd::Restore { target, identity, yes } => {
                if let Err(e) = snapshot::restore(target, identity, yes) {
                    eprintln!("snapshot restore: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        },
        Cmd::Host { action } => match action {
            HostCmd::Push { name, no_pr } => {
                if let Err(e) = host::push(name, no_pr) {
                    eprintln!("host push: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        },
        Cmd::Passwd { user } => {
            if let Err(e) = passwd::run(user) {
                eprintln!("passwd: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Remote { down } => {
            if let Err(e) = remote::run(down) {
                eprintln!("remote: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Net { status } => {
            let r = if status { net::status() } else { net::ensure(true) };
            match r {
                Ok(()) if !status => println!("network: online"),
                Ok(()) => {}
                Err(e) => {
                    eprintln!("net: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        Cmd::Disks => {
            if let Err(e) = install::list_disks() {
                eprintln!("{e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        Cmd::Commands { .. } => {
            let entries = commands::all();
            println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        }
    }

    std::process::ExitCode::SUCCESS
}

# Kiwami

An opinionated personal NixOS distro: Hyprland, a Quickshell shell written
from scratch, and a CLI that installs, updates and backs up a machine.

It runs on real hardware — an encrypted XPS 13 with an ephemeral root, whose
whole identity is restored from a backup during the install. It is also
developed largely by an agent driving a VM, which is why the tests boot real
images on a real console rather than asserting against mocks.

Status: early, and used daily by one person. See [TASKS.md](TASKS.md).

## The three rules

Everything here follows from three decisions. They are worth reading before
the option list, because most of the design is downstream of them.

**1. The machine keeps no copy of its own configuration.** There is no
checkout in your home directory to rebuild from. `kiwami update` builds from
the flake on GitHub and switches to it. A local copy is a second source of
truth that can be older than the disk it describes, and every bug in that area
came from having one.

**2. The root is wiped at every boot.** Only what `kiwami.persist` declares
survives. Anything you did not declare is gone in the morning, which means
configuration drift cannot accumulate quietly — it is deleted.

**3. The flake is the machine.** Configs are placed by Home Manager as
read-only symlinks into the Nix store. You change one by editing it here and
rebuilding, the same motion as changing a package. There is no user-override
layer.

Two things deliberately stay at runtime, because a rebuild is the wrong
granularity:

- **Themes** — authored as typed Nix, generated to JSON, switched with
  `kiwami theme set` and no rebuild. The shell watches the file and retints
  live.
- **The shell tree during development** — the launcher prefers
  `~/Projects/kiwami/shell` when it exists, so iterating on QML is a 0.8s
  restart rather than an 8s rebuild.

## Using it

Your machines live in **your** repository. Kiwami is an input, not a fork:

```nix
{
  inputs.kiwami.url = "github:kiwamios/kiwami";

  outputs = { self, kiwami, ... }: {
    nixosConfigurations.laptop = kiwami.lib.mkHost [ ./hosts/laptop ];
  };
}
```

`mkHost` is exported so you do not rebuild the recipe by hand — it wires
nixosSystem, the Kiwami module, disko and the flake inputs exactly the way
Kiwami's own test machines are built, so your machines and ours cannot drift.

A host is then ordinary NixOS:

```nix
# hosts/laptop/default.nix
{
  imports = [ ./hardware.nix ./disk.nix ];

  networking.hostName = "laptop";
  kiwami.user = "alice";

  # Where this machine rebuilds itself from. It keeps no checkout, so
  # without this `kiwami update` has nowhere to build.
  kiwami.flake = "github:alice/my-machines";

  kiwami.bar.position = "bottom";
  system.stateVersion = "26.05";
}
```

Kiwami's defaults use `mkDefault`, so your values win and ours fill the rest.
`nix flake update` pulls improvements without touching what you overrode.

[jimzer/kiwami-hosts](https://github.com/jimzer/kiwami-hosts) is the author's
own, and is the worked example — the same code path everyone else uses.

### Installing

Build an installer image, write it to a stick, boot it. The installer is
conversational: it asks which disk, whether to encrypt, and shows you the
layout before it writes anything.

```bash
nix build github:kiwamios/kiwami#nixosConfigurations.installer-x86_64.config.system.build.isoImage
scripts/flash-linux.sh result/iso/*.iso
```

On the machine, `kiwami install` starts by itself.

### An image that already holds the machine

For a machine that already exists, `kiwami image` builds an installer carrying
that host's **entire built system**. `nixos-install --system` then installs a
prebuilt closure instead of evaluating a flake, so the several gigabytes it
would otherwise download are already on the stick and the install needs no
network at all.

```bash
sudo kiwami image                    # about a minute, ~3 GB
```

It carries no state — no keys, tokens or wifi — so it is not a bundle of
secrets, though it names your hostname and is private rather than publishable.
It does not need to be fresh either: the image supplies the machine,
`kiwami snapshot restore` supplies everything that happened since, so a
months-old stick still boots you into a working laptop.

## Backups

`/persist` is the whole target — there is no per-directory policy to maintain,
because the root is wiped anyway and everything that matters is already
declared.

```bash
sudo kiwami snapshot setup     # pick a repository, mint a passphrase, prove a round trip
sudo kiwami snapshot backup    # or let the daily timer do it
sudo kiwami snapshot restore   # everything, or --identity for just keys and tokens
```

Restic to any S3-compatible store. `setup` generates a memorable passphrase,
writes a canary, restores it, and refuses to call itself configured until that
round trip works.

The installer offers a restore, so a reinstall gives you back the same
machine — same SSH host key, same wifi, same tailnet node — rather than a
fresh one with your name on it.

## Options

`kiwami.*` is the API. It exists so machines can differ without forking files.

| | |
|---|---|
| `flake`, `host` | where this machine rebuilds itself from |
| `user` | the account the desktop belongs to |
| `ephemeralRoot`, `rootDevice` | wipe the root at boot; where it lives |
| `persist.{directories,files,userDirectories,userFiles}` | what survives that |
| `backup.{enable,repository,credentialsFile,schedule,keep,exclude}` | restic, where to, how often, how much to keep |
| `bar.{enable,position,height,left,center,right}` | which widgets, where |
| `theme.{name,themes}` | palettes, type-checked as `#rrggbb` |
| `terminal.{settings,extraConfig}` | per-machine Ghostty overrides |
| `hyprland.extraConfig` | Lua appended after ours |
| `animations`, `autoLogin`, `passwordFile` | the obvious things |

Widgets resolve by filename: add `shell/widgets/Weather.qml`, name it in
`kiwami.bar.right`, done.

## The CLI

```bash
kiwami install                  # install to a disk, from the live ISO
kiwami install --new --host x   # scaffold a machine and install it
kiwami install --relayout       # change an existing machine's disk layout
kiwami update                   # rebuild from this machine's flake
kiwami image                    # a stick carrying this machine's whole system
kiwami snapshot setup|backup|restore|status
kiwami host push                # push this machine's config, as a PR
kiwami auth                     # what needs credentials, and log in
kiwami net                      # get online; offers a wifi menu
kiwami remote                   # be reachable over your tailnet
kiwami theme list|set|current   # switch theme, no rebuild
kiwami doctor                   # drift + health; non-zero exit on problems
kiwami disks                    # what the installer can see
kiwami commands --json          # actions the launcher offers
```

The launcher merges those with `.desktop` entries, so typing "theme" offers
the switches without opening a terminal.

## Keybinds

| | |
|---|---|
| `SUPER+RETURN` | terminal |
| `SUPER+SPACE` | launcher |
| `SUPER+SHIFT+P` | power menu |
| `SUPER+SHIFT+R` | restart the shell — bound in the compositor, so it works when the shell is gone |
| `SUPER+1..9` | workspaces |
| `XF86Audio*` / `XF86MonBrightness*` | volume and brightness, with OSD |

## Development

```bash
just vm install       # wipe + unattended install (~3 min)
just vm start         # boot headless
just vm gui           # boot in a window
just vm reload        # push + restart the shell, no rebuild
just vm rebuild       # push + nixos-rebuild switch
just vm doctor
just vm install-test  # installer matrix
just vm practice      # installer ISO in a window, throwaway disk, by hand
just check            # lint + evaluate every host and the boot test
```

The development VM is aarch64 so it runs natively under HVF on Apple silicon.
CI builds and boots x86_64, which is what real hardware uses.

`hosts/` here holds only `vm-*` — test fixtures, which belong beside the tests
that run them. Real machines live in their owners' repositories.

### Tests

`tests/` holds the desktop nixosTest, which CI runs on x86_64. Everything else
that needs a VM — the installer conversation, the ephemeral root, encryption,
relayout — is driven by `vm/`, which boots the real image on a real console,
because the failures worth catching are the ones a mock cannot have.

A rented bare-metal builder was tried and removed: the tests worth running
already run in CI or on the Mac, and the one that seemed to need it turned out
to be the one that should not have been a nixosTest at all.

## Layout

```
flake.nix     inputs, nixosModules.default, lib.mkHost, lib.installerFor
modules/      options, themes, generation, desktop, impermanence, backup
config/       hyprland.lua, ghostty config, theme palettes
shell/        Quickshell QML
cli/          kiwami: install, update, snapshot, image, theme, doctor
hosts/        vm-* only: the machines the tests boot
tests/        the desktop nixosTest, run by CI
vm/           the development VM and its harness
scripts/      flashing, host push
docs/         notes worth keeping
```

## Notes

- [docs/build-in-public.md](docs/build-in-public.md) — what actually went
  wrong, written down while it was still embarrassing.
- [docs/session-services.md](docs/session-services.md) — how the session is
  wired, and the traps that cost real time.

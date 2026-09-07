# Kiwami

A personal NixOS desktop: Hyprland, a Quickshell shell written from scratch,
and a small CLI — developed against a VM the agent can drive.

Status: early. The desktop works in a VM; it has never run on real hardware.
See [TASKS.md](TASKS.md).

## The model

**The flake is the machine.** Configs are placed by Home Manager as read-only
symlinks into the Nix store. You change one by editing it here and rebuilding
— the same motion as changing a package or a service. There is no separate
user-override layer.

Two things stay at runtime, because a rebuild is the wrong granularity for
them:

- **Themes.** Authored as typed Nix, generated to JSON, switched with
  `kiwami theme set` and no rebuild. The shell watches the file and retints
  live, which is what designing a palette needs.
- **The shell tree during development.** The launcher prefers `~/kiwami/shell`
  when it exists, so iterating on QML is a 0.8s restart rather than an 8s
  rebuild.

## Using it

Install NixOS, then:

```bash
nixos-install --flake github:kiwamios/kiwami#desktop
```

## Building on it

A machine is a small flake of its own — no fork:

```nix
{
  inputs.kiwami.url = "github:kiwamios/kiwami";

  outputs = { nixpkgs, kiwami, ... }: {
    nixosConfigurations.my-laptop = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        kiwami.nixosModules.default
        ./hardware.nix
        { kiwami.bar.position = "bottom"; }
      ];
    };
  };
}
```

Kiwami's defaults use `mkDefault`, so your values win and ours fill the rest.
`nix flake update` pulls improvements without touching what you overrode.

## Options

`kiwami.*` is the API. It exists so machines can differ without forking files.

| | |
|---|---|
| `bar.{enable,position,height,left,center,right}` | which widgets, where |
| `theme.{name,themes}` | palettes, type-checked as `#rrggbb` |
| `terminal.{settings,extraConfig}` | per-machine Ghostty overrides |
| `hyprland.extraConfig` | Lua appended after ours |

Widgets resolve by filename: add `shell/widgets/Weather.qml`, name it in
`kiwami.bar.right`, done.

## The CLI

```bash
kiwami install             # install to a disk, from the live ISO
kiwami disks               # what the installer can see
kiwami net                 # get online; offers a wifi menu if needed
kiwami install --new --host laptop   # scaffold a machine and install it
kiwami theme list / set    # switch theme, no rebuild
kiwami doctor              # drift + health; non-zero exit on problems
kiwami commands --json     # actions the launcher offers
```

The launcher merges those commands with `.desktop` entries, so typing "theme"
offers the switches without opening a terminal.

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
just vm install      # wipe + unattended install (~3 min)
just vm start        # boot headless
just vm gui          # boot in a window
just vm reload       # push + restart the shell, no rebuild
just vm rebuild      # push + nixos-rebuild switch
just vm screenshot x
just vm doctor       # or: just vm ssh 'kiwami doctor'
just vm install-test # installer matrix
just vm practice     # installer ISO in a window, throwaway disk, by hand
just check           # lint + evaluate every host and the boot test (~16s)
```

The VM is aarch64 so it runs natively under HVF on Apple silicon. CI builds
and boots x86_64, which is the architecture real hardware will use.

### A stick that already holds the machine

`nixosConfigurations.installer-xps` is the ordinary installer image plus this
host's entire built system. `nixos-install --system` then installs a prebuilt
closure instead of evaluating the flake, so the ~6 GiB it would otherwise
download is already on the stick and the install needs no network at all.

```bash
# on an x86_64 Linux machine (the laptop can build its own)
nix build .#nixosConfigurations.installer-xps.config.system.build.isoImage
scripts/flash-linux.sh result/iso/*.iso
```

It carries no state - no keys, tokens or wifi - so it is not a bundle of
secrets, though it does name your bucket and hostname and is private rather
than publishable. And it does not need to be fresh: the image supplies the
machine, `kiwami snapshot restore` supplies everything that happened since, so
a months-old stick still boots you into a working laptop.

### Tests that need a Linux machine

`tests/` holds the desktop nixosTest, which CI runs on x86_64. Everything
else that needs a VM - the installer conversation, the ephemeral root,
encryption - is driven by `vm/`, which boots the real image on a real console.

A rented bare-metal builder was tried and removed: the tests worth running
already run in CI or on the Mac, and the one that seemed to need it turned out
to be the one that should not have been a nixosTest at all.

## Layout

```
flake.nix          inputs, hosts, nixosModules.default
modules/           options, themes, generation, desktop, home-manager
config/            hyprland.lua, ghostty config, theme palettes
shell/             Quickshell QML
cli/               kiwami: install, theme, doctor, commands
hosts/             per-machine: choices + hardware facts
tests/             the desktop nixosTest, run by CI
vm/                the development VM and its harness
scripts/           flashing, host push
docs/              notes worth keeping
```

## Notes

[docs/session-services.md](docs/session-services.md) — how the session is
wired, and the traps that cost real time.

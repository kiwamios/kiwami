# My machines

Machines built from [Kiwami](https://github.com/kiwamios/kiwami).

Kiwami is the distro — Hyprland, the shell, the CLI. This repository is what
is yours about your particular computers: their names, their disks, their
hardware, where they back up to. Kiwami is an input here, never a fork, so it
can be updated without carrying your laptop with it.

## First machine

1. Rename this repository's `hosts/example` to whatever the computer is
   called, or leave it — `kiwami install` will scaffold a real one and you can
   delete the example.
2. Set `kiwami.flake` in that host to **this repository's** URL. It is how the
   machine finds its own configuration: it keeps no checkout, so `kiwami
   update` builds from here.
3. Push it.
4. Boot the Kiwami installer and point it at this repository:

   ```bash
   sudo kiwami install --flake github:you/my-machines --new --host laptop
   ```

The installer detects the hardware, designs a disk layout, shows it to you
before writing anything, and offers to push the finished host back here.

## After that

```bash
sudo kiwami update      # rebuild this machine from this repository
sudo kiwami host push   # send a machine's config here, as a pull request
sudo kiwami image       # a USB stick carrying this machine's whole system
```

`nix flake update kiwami` pulls improvements from the distro. Nothing moves
until you run it — that is the point of pinning it here.

## Adding a second machine

Boot the installer on it and give it the same `--flake`. A directory under
`hosts/` is a machine; there is no list to keep in sync.

## What goes where

| | |
|---|---|
| `hosts/<name>/default.nix` | choices: hostname, user, options |
| `hosts/<name>/hardware.nix` | facts, written by the installer |
| `hosts/<name>/disk.nix` | the layout disko formats and mounts from |

Everything else — the desktop, the shell, the CLI — comes from Kiwami. The
options it exposes are documented in its README; set them in
`hosts/<name>/default.nix` and yours win over its defaults.

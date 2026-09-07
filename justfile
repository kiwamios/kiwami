# Kiwami — top-level command interface.
#
# Subsystems expose their own justfile and are mounted here as modules, so
# `just vm <recipe>` is the stable API while vm/scripts/* can change freely.
#   just            list everything
#   just vm         list the vm subsystem
#   just vm start   run a subsystem recipe

set shell := ["bash", "-uc"]

# Dev VM: build, boot, drive, screenshot, snapshot
mod vm

# List available commands
default:
    @just --list --unsorted

# NOTE: a bare `for` loop returns only its LAST iteration's status, so a
# failure in the middle would be silently swallowed - hence the rc tracking.

# Everything CI's `evaluate` job does, locally. This is the check that would
# have caught a module being deleted while two files still imported it -
# `just lint` never will, because it does not evaluate any Nix.
eval:
    #!/usr/bin/env bash
    set -euo pipefail
    # `just` runs recipes in a non-interactive shell, which never sources the
    # profile snippet that puts nix on PATH.
    command -v nix >/dev/null || export PATH="/nix/var/nix/profiles/default/bin:$PATH"

    hosts=$(nix eval --json '.#nixosConfigurations' --apply builtins.attrNames \
              | tr -d '[]"' | tr ',' ' ')
    # An empty list here is a failure, not a pass. Silently iterating over
    # nothing is exactly how a broken check reports success forever.
    [ -n "${hosts// /}" ] || { echo "  no hosts found - is the flake broken?"; exit 1; }

    for h in $hosts "desktop-test"; do
      case "$h" in
        desktop-test) attr='.#checks.x86_64-linux.desktop.drvPath' ;;
        *)            attr=".#nixosConfigurations.$h.config.system.build.toplevel.drvPath" ;;
      esac
      printf '  %-14s ' "$h"
      # Nix warns about the dirty git tree on every call; only worth seeing
      # when something actually fails.
      if err=$(nix eval --raw "$attr" 2>&1 >/dev/null); then
        echo ok
      else
        echo FAIL; echo "$err" >&2; exit 1
      fi
    done

    # What the installer image must contain, asserted without building one.
    # Compressing the squashfs dominates an ISO build and is redone in full
    # for the smallest change, so the loop that catches mistakes should never
    # touch it.
    for iso in installer-x86_64 installer-aarch64; do
      printf '  %-14s ' "$iso"
      want='c: assert builtins.any (p: (p.pname or p.name or "") == "kiwami") c.environment.systemPackages;
            assert builtins.elem "flakes" c.nix.settings.experimental-features;
            assert c.services.getty.autologinUser != null; true'
      if err=$(nix eval --apply "$want" ".#nixosConfigurations.$iso.config" 2>&1 >/dev/null); then
        echo ok
      else
        echo FAIL; echo "$err" >&2; exit 1
      fi
    done

# Syntax of every script and justfile, plus the CLI's unit tests.
lint:
    @cargo test --manifest-path cli/Cargo.toml --quiet 2>&1 | grep -E "test result|^error" || true
    @rc=0; for f in vm/scripts/*.sh scripts/*.sh; do \
        if bash -n "$f"; then echo "ok   $f"; else echo "FAIL $f"; rc=1; fi; done; \
      for f in vm/scripts/*.py; do \
        if python3 -c "import ast,sys;ast.parse(open(sys.argv[1]).read())" "$f"; \
        then echo "ok   $f"; else echo "FAIL $f"; rc=1; fi; done; \
      for j in justfile vm/justfile; do \
        if just --justfile "$j" --summary >/dev/null 2>&1; then echo "ok   $j"; else echo "FAIL $j"; rc=1; fi; done; \
      exit $rc

# Everything that can be verified without a Linux builder.
check: lint host-push-test eval template-test

# `kiwami host push` sends the host directory and nothing else. Needs only git
# and the built CLI, so it belongs here rather than in the VM matrix.
host-push-test:
    @./scripts/host-push-test.sh

# Write an installer image to a removable drive, by name rather than by
# /dev/diskN - which moves when you replug things.
flash ISO NAME="Portable SSD T5":
    @./scripts/flash.sh "{{ISO}}" "{{NAME}}"

# Show repo layout, ignoring build artifacts
tree:
    @git ls-files

# The template a stranger starts from, instantiated and evaluated.
#
# It exists so somebody with no configuration repository has one command
# rather than a page of instructions - which only holds if it still works.
# Living in this repository is what makes that checkable: if mkHost or the
# option surface changes shape, this fails here rather than in a stranger's
# first hour.
template-test:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v nix >/dev/null || export PATH="/nix/var/nix/profiles/default/bin:$PATH"

    repo=$PWD
    dir=$(mktemp -d)
    trap 'rm -rf "$dir"' EXIT
    cd "$dir"

    printf '  %-14s ' "template"
    if ! err=$(nix flake init -t "$repo" 2>&1 >/dev/null); then
      echo FAIL; echo "$err" >&2; exit 1
    fi
    # A git tree, because nix reads flakes through git and an untracked file
    # is invisible to it - the trap that hid config/wallpaper from the build.
    git init -q . && git add -A

    # Against this checkout rather than what is on GitHub: the point is to
    # catch a change before it is pushed, not after.
    if err=$(nix eval --override-input kiwami "$repo" \
               --raw '.#nixosConfigurations.example.config.system.build.toplevel.drvPath' \
               2>&1 >/dev/null); then
      echo ok
    else
      echo FAIL; echo "$err" >&2; exit 1
    fi

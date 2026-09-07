#!/usr/bin/env bash
# Install a machine that is both encrypted and ephemeral, then prove it is
# both - across a reboot that has to be unlocked by hand.
#
# These two features were built separately and never once ran together. That
# gap hid a real defect: the rollback mounts the root filesystem in the initrd
# by device name, and inside a LUKS container that name is
# /dev/mapper/cryptroot, not the partition. Naming the partition would hand
# the rollback a LUKS blob to mount as btrfs - a failure with no shell to
# debug from, on the first boot after installing.
#
# The claim here is specifically the interaction, so the interesting assertion
# is the second boot: unlock the disk, and *then* find the root wiped and
# /persist intact. Either half passing alone proves nothing about the pair.
#
# Runs on its own disk so the dev VM everything else is driven from survives.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
VM_DIR="$(cd "$DIR/.." && pwd)"
PASS="${PASS:-kiwami-test}"

HOST=vm-luks-ephemeral \
DISK="$VM_DIR/disks/luks-ephemeral.qcow2" \
VARS="$VM_DIR/disks/luks-ephemeral-vars.fd" \
SNAPSHOT=0 \
PRE_INSTALL="printf '%s' '$PASS' > /tmp/luks.key" \
UNLOCK="$PASS" \
  "$DIR/install.sh" || { echo "install failed"; exit 1; }

pass=0; fail=0
ok() { printf '  \033[32mPASS\033[0m  %s\n' "$1"; pass=$((pass+1)); }
no() { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; fail=$((fail+1)); }
ssh_() { "$DIR/vmssh" "$@" 2>/dev/null; }

echo
echo "encrypted"
ssh_ 'findmnt -no SOURCE / | grep -q "^/dev/mapper/"' \
  && ok "root is on a mapped device" || no "root is on a mapped device"
ssh_ 'sudo cryptsetup status cryptroot | grep -q "type: *LUKS"' \
  && ok "cryptroot is a LUKS volume" || no "cryptroot is a LUKS volume"

echo
echo "and ephemeral"
ssh_ 'findmnt -no FSTYPE / | grep -q btrfs' \
  && ok "root is btrfs inside the container" || no "root is btrfs inside the container"
ssh_ 'findmnt -no OPTIONS / | grep -q "subvol=/@root"' \
  && ok "root is the @root subvolume" || no "root is the @root subvolume"
ssh_ 'findmnt -no TARGET /persist | grep -q /persist' \
  && ok "/persist is mounted" || no "/persist is mounted"
# The blank snapshot has to exist on the *decrypted* filesystem. Looking for
# it through /dev/mapper is the point: through the partition there is nothing
# but ciphertext.
ssh_ 'sudo mkdir -p /tmp/b && sudo mount -o subvol=/ /dev/mapper/cryptroot /tmp/b && sudo btrfs subvolume show /tmp/b/@root-blank >/dev/null' \
  && ok "@root-blank exists inside the container" || no "@root-blank exists inside the container"

echo
echo "the wipe, through an unlock"
ssh_ 'sudo touch /ephemeral-marker && sudo touch /persist/persistent-marker' >/dev/null \
  && ok "both markers written" || no "both markers written"

echo "  rebooting (the passphrase has to be typed again)..."
# The console buffer is seeded from serial.log, which still holds the *first*
# boot's passphrase prompt - so expect() matches it instantly, the passphrase
# goes out before the new prompt exists, and the machine sits waiting forever
# while the test reports that it asked and was answered. Truncating the log
# means the next match can only come from this boot.
: > "$VM_DIR/serial.log"
ssh_ 'sudo systemctl reboot' >/dev/null 2>&1 || true
sleep 5

# The unlock prompt exists only on the serial console: this happens before
# networking, so there is no other channel. Waiting for the prompt rather than
# sending blind - typing early goes nowhere, and a broken rollback would hang
# here rather than fail, which is precisely what this test is for.
#
# Matched on "passphrase for", the part of systemd's wording that is stable.
CONSOLE="python3 $DIR/console.py"
if TIMEOUT=240 $CONSOLE expect 'passphrase for' >/dev/null 2>&1; then
  $CONSOLE send "$PASS" >/dev/null
  ok "it asked for the passphrase again after the wipe"
else
  no "it asked for the passphrase again after the wipe"
  echo "     last console output:"
  tail -c 1200 "$VM_DIR/serial.log" | tr '\r' '\n' | tail -8 | sed 's/^/     /'
fi

for _ in $(seq 1 60); do ssh_ true && break; sleep 5; done
if ssh_ true; then
  ok "it came back after the reboot"

  # Only asked once the machine is reachable.
  #
  # `ssh_ 'test -e /ephemeral-marker' || ok "the marker is gone"` scored a
  # pass on a machine that never booted: ssh failed, the || fired, and an
  # unreachable machine reported a successful wipe. The absence of an answer
  # is not the answer - which is the whole failure mode this suite exists to
  # catch, reproduced in the suite itself.
  ssh_ 'test -e /ephemeral-marker' \
    && no "the root marker is gone - the root was wiped" \
    || ok "the root marker is gone - the root was wiped"
  ssh_ 'test -e /persist/persistent-marker' \
    && ok "the persist marker survived" || no "the persist marker survived"
  ssh_ 'findmnt -no SOURCE /var/lib/nixos | grep -q persist' \
    && ok "declared state is bound back after the wipe" || no "declared state is bound back after the wipe"

  # The password after a boot, which is not the same question as the password
  # after a rebuild.
  #
  # Activation runs in the initrd, before the bind mounts exist. The hash was
  # named by its runtime path - /var/lib/kiwami/passwords - which at that
  # moment is an empty directory on the bare root, so the seed fired every
  # boot and the install default went into /etc/shadow. The machine's password
  # was the one published in this repository until something re-ran
  # activation, and the owner's own password was refused at the greeter.
  #
  # Nothing caught it because every check ran on a machine that had just been
  # rebuilt, where activation had run with the mounts present and the hash was
  # correct. Only a plain reboot shows it - which is why this lives here.
  ssh_ 'getent shadow $(id -un) | grep -q kiwamidefault' \
    && no "a booted machine does not fall back to the install password" \
    || ok "a booted machine does not fall back to the install password"
else
  no "it came back after the reboot"
  no "the wipe could not be checked - the machine never came up"
  no "the password could not be checked - the machine never came up"
  echo "     last console output:"
  tail -c 1200 "$VM_DIR/serial.log" | tr '\r' '\n' | grep -v "^\s*$" | tail -8 | sed 's/^/     /'
fi

echo
if [[ $fail -eq 0 ]]; then
  printf '\033[1;32m==> encrypted and ephemeral work together (%d checks)\033[0m\n' "$pass"
else
  printf '\033[1;31m==> %d failed, %d passed\033[0m\n' "$fail" "$pass"
  exit 1
fi

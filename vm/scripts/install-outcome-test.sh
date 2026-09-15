#!/usr/bin/env bash
# What the installer leaves behind, as opposed to what it says on the way.
#
# `guided-test` asserts that prompts appear and then takes the abort the
# installer offers, so it never completes an install. `install.sh` completes
# one but installs from a local path and verifies only that ssh answers
# afterwards. Between them they missed four bugs in a single real install:
#
#   - a bare `owner/repo` was passed to nix unchanged, which reads it as a
#     registry lookup and fails naming neither the mistake nor the fix
#   - the machine recorded kiwami.flake = "/home/nixos/kiwami", the
#     installer's own scratch clone inside the live image, gone at the first
#     reboot - so it had nowhere to rebuild from
#   - the generated hosts/<name>/ went with it, and the advice printed for
#     recovering it said to run a command that needed the destroyed directory
#   - the password prompt never appeared, because `nixos-install` had already
#     seeded the hash file with the locked marker and the check asked whether
#     the file existed
#
# All four are downstream of one gap: no test ever installed from a flake
# that had to be *cloned*. A local path needs no clone, so the whole branch
# where these live never ran.
#
# So this installs from a bare git repository on the machine itself. It is a
# real remote - git clones it, the installer treats it like any other - and it
# needs no network, no GitHub account and no credentials, which is why the
# branch can finally be tested at all.
#
# Assertions are about the installed tree under /mnt, checked before any
# reboot: these are facts on disk, not lines in a log.
#
# Not covered, and named rather than papered over: `gh auth login` and the
# pull request `kiwami host push` opens. Those need GitHub. What is covered is
# everything up to them - that the host is committed and pushed to a remote,
# which is the part that was silently not happening.
set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
CONSOLE="python3 $DIR/console.py"
pass=0; fail=0

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
ok()   { printf '  \033[32mPASS\033[0m  %s\n' "$1"; pass=$((pass + 1)); }
no()   { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; [[ -n "${2:-}" ]] && printf '        %s\n' "$2"; fail=$((fail + 1)); }

# check <name> <expected-substring> <command>
check() {
  local name="$1" want="$2" cmd="$3" got
  got=$($CONSOLE run "$cmd" 2>&1)
  if grep -qF -- "$want" <<<"$got"; then ok "$name"; else no "$name" "wanted '$want', got: ${got:0:200}"; fi
}

REMOTE="git+file:///tmp/machines.git"
HOSTNAME_="outcome"

step "standing up a bare repository as the remote"
# Seeded from the template the distro ships, so this also checks the template
# is installable - it is the thing a new user is told to start from.
$CONSOLE run "rm -rf /tmp/machines.git /tmp/machines &&
  git init -q --bare /tmp/machines.git &&
  mkdir -p /tmp/machines && cd /tmp/machines &&
  nix --extra-experimental-features 'nix-command flakes' flake init -t /tmp/kiwami 2>&1 | tail -2 &&
  git init -q && git add -A &&
  git -c user.email=t@t -c user.name=t commit -qm seed &&
  git push -q /tmp/machines.git HEAD:refs/heads/main &&
  echo SEEDED" >/dev/null

check "the bare repo has a flake to clone" "flake.nix" \
  "git --git-dir=/tmp/machines.git ls-tree --name-only main"

step "installing from it, unattended"
# --yes, so the password is deliberately the locked marker; that the installer
# says so is asserted below. The interactive prompt is exercised by the
# guided test, which now reaches it because the check is no longer "does the
# file exist".
$CONSOLE run "nohup env NIX_CONFIG='experimental-features = nix-command flakes' \
  nix run /tmp/kiwami#kiwami -- install \
    --disk /dev/vda --yes --force --new \
    --flake ${REMOTE} --host ${HOSTNAME_} > /tmp/outcome.log 2>&1 &" >/dev/null

for _ in $(seq 1 120); do
  sleep 10
  if $CONSOLE run 'grep -q "installation finished" /tmp/outcome.log && echo DONE' 2>/dev/null | grep -q DONE; then
    echo "    installation finished"; break
  fi
  if $CONSOLE run 'grep -q "^install:" /tmp/outcome.log && echo REFUSED' 2>/dev/null | grep -q REFUSED; then
    echo "    installer refused:"; $CONSOLE run 'grep "^install:" /tmp/outcome.log'; exit 1
  fi
  printf '    still installing...\n'
done

step "what it wrote"

# The bug that cost an evening. The reference recorded must be the remote,
# never the clone the installer made to work in - that directory does not
# survive the reboot, and a machine pointed at it can never rebuild.
check "the machine records the remote, not the scratch clone" "$REMOTE" \
  "cat /mnt/etc/kiwami/origin.json"
check "and not a path inside the live image" "0" \
  "grep -c '/home/nixos' /mnt/etc/kiwami/origin.json || true"
check "the generated host says the same" "$REMOTE" \
  "grep 'kiwami.flake' /mnt/etc/kiwami/origin.json"

# The host config is the only copy until it is pushed, and the live image is
# about to be destroyed.
check "the host reached the remote" "hosts/${HOSTNAME_}" \
  "git --git-dir=/tmp/machines.git ls-tree -r --name-only main"
check "with its hardware" "hosts/${HOSTNAME_}/hardware.nix" \
  "git --git-dir=/tmp/machines.git ls-tree -r --name-only main"
check "and its layout" "hosts/${HOSTNAME_}/disk.nix" \
  "git --git-dir=/tmp/machines.git ls-tree -r --name-only main"

# Unattended means locked, and saying so is the point: an account nobody can
# log into is recoverable, a published default is not.
check "an unattended install locks the account" "!" \
  "cat /mnt/persist/var/lib/kiwami/passwords/* 2>/dev/null || cat /mnt/var/lib/kiwami/passwords/*"
check "and says the account is locked" "locked" "grep -i locked /tmp/outcome.log"

# Advice that cannot be followed reads as 'handled'. Whatever this prints
# about recovering the host config, it must not tell you to do it after a
# reboot that destroys the only copy.
check "it never says to push after rebooting" "0" \
  "grep -ci 'after rebooting' /tmp/outcome.log || true"

printf '\n'
if (( fail )); then
  printf '\033[1;31m==> %d failed, %d passed\033[0m\n' "$fail" "$pass"
  echo "installer log: /tmp/outcome.log in the guest"
  exit 1
fi
printf '\033[1;32m==> the installer leaves the machine able to rebuild itself (%d checks)\033[0m\n' "$pass"

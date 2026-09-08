#!/usr/bin/env python3
"""Drive the guided installer from a cold boot, the way a person does.

Every guided-flow bug so far was found by a human sitting at the machine
pressing keys: it started with no network and exited, the host menu offered
"n) a new machine" and then refused n, pressing n crashed it, and a question
rejected its own documented answer and quit. Each cost a real reinstall to
find, and nothing that existed could have caught them - the other VM tests
drive `kiwami install` with flags, so the conversation itself was untested.

The useful thing about those four is that all of them happened *before*
anything was written to disk. So this does not install: it walks the prompts
and takes the abort the installer already offers. Two minutes rather than
twenty, and the disk being untouched is itself asserted at the end - an
installer that writes before you confirm would be worse than any prompt bug.

Not covered, deliberately: the VM has no wifi device, so the wifi branch
cannot run here. The no-network path is exercised by cutting the link, which
proves it explains itself rather than dying silently - not that joining a
network works. That gap is the same one that produced most of these bugs, so
it is named rather than papered over.
"""

import hashlib
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

DIR = pathlib.Path(__file__).resolve().parent
VM_DIR = DIR.parent
sys.path.insert(0, str(DIR))

os.environ["SERIAL_LOG"] = str(VM_DIR / "serial.log")
os.environ["SERIAL_SOCK"] = "/tmp/kiwami-guided-serial.sock"
os.environ["QMP_SOCK"] = "/tmp/kiwami-guided-qmp.sock"

from console import Console, clean  # noqa: E402

# The shipped image, not the -test one. The -test image pre-creates the marker
# the autostart checks so that `install-test` can type commands at a shell;
# this suite's whole first claim is that nobody had to type anything, so it
# needs the image that behaves like the one people boot.
ISO = VM_DIR / "iso/kiwami-installer-guided-aarch64.iso"
DISK = VM_DIR / "disks/guided.qcow2"
VARS = VM_DIR / "disks/guided-vars.fd"

passed = failed = 0


def ok(msg):
    global passed
    passed += 1
    print(f"  \033[32mPASS\033[0m  {msg}")


def no(msg, ctx=""):
    global failed
    failed += 1
    print(f"  \033[31mFAIL\033[0m  {msg}")
    if ctx:
        # What the console last said. A guided-flow failure is almost always
        # "it asked something else", and without this the only symptom is a
        # timeout with no clue what it was waiting on.
        tail = clean(ctx).strip().splitlines()[-12:]
        for line in tail:
            print(f"\033[2m        {line}\033[0m")


def step(msg):
    print(f"\n\033[1;36m==> {msg}\033[0m")


def qmp(command, **args):
    """One QMP command, for cutting the link out from under the installer."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(os.environ["QMP_SOCK"])
    s.settimeout(5)
    s.recv(65536)
    s.sendall(b'{"execute":"qmp_capabilities"}\n')
    s.recv(65536)
    s.sendall(json.dumps({"execute": command, "arguments": args}).encode() + b"\n")
    reply = s.recv(65536).decode()
    s.close()
    return reply


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def newer_than_iso():
    """Source files changed since the image was built."""
    if not ISO.exists():
        return []
    built = ISO.stat().st_mtime
    root = VM_DIR.parent
    watched = []
    for sub in ("cli/src", "modules", "shell", "config"):
        watched += list((root / sub).rglob("*"))
    watched.append(root / "flake.nix")
    return [f for f in watched if f.is_file() and f.stat().st_mtime > built]


def boot():
    """A blank disk and a cold boot from the built image."""
    if not ISO.exists():
        print(f"no image at {ISO}\nrun: just vm build-iso-guided")
        sys.exit(1)

    # Which image this is testing, said out loud.
    #
    # An aarch64-linux ISO cannot be built on macOS, so it is built elsewhere
    # and copied in - which means the one on disk can predate the code being
    # changed, and a pass then belongs to whatever was in it rather than to
    # the working tree. Not a failure and not worth blocking on; worth seeing.
    stale = newer_than_iso()
    built = time.strftime("%Y-%m-%d %H:%M", time.localtime(ISO.stat().st_mtime))
    if stale:
        print(f"  image built {built}; {len(stale)} source file(s) are newer")
    else:
        print(f"  image built {built}")

    (VM_DIR / "disks").mkdir(exist_ok=True)
    DISK.unlink(missing_ok=True)
    subprocess.run(["qemu-img", "create", "-f", "qcow2", str(DISK), "20G"],
                   check=True, stdout=subprocess.DEVNULL)
    # Its own firmware variables: booting a CD rewrites the boot order, and a
    # shared file leaves other VMs dropping into an EFI shell afterwards.
    brew = subprocess.run(["brew", "--prefix"], capture_output=True, text=True).stdout.strip()
    subprocess.run(["cp", "-f", f"{brew}/share/qemu/edk2-arm-vars.fd", str(VARS)], check=True)

    env = dict(os.environ, ISO=str(ISO), DISK=str(DISK), VARS=str(VARS), SSH_PORT="2224")
    subprocess.run([str(DIR / "start-vm.sh"), "install", "headless"], env=env, check=True)
    return Console(path=os.environ["SERIAL_SOCK"], seed_log=False)


def reply(c, text, needle, timeout=120):
    """Answer a prompt and wait for what should come next.

    The buffer is cleared before sending because expect() scans everything
    accumulated so far - without this, a needle that already appeared once
    matches instantly and the whole sequence desynchronises.
    """
    c.buf = b""
    c.sendline(text)
    return c.expect(needle, timeout)


def conversation():
    """The whole flow, from cold boot to the abort it offers."""
    step("booting the installer image on a blank disk")
    c = boot()
    before = digest(DISK)

    step("it starts on its own")
    # Nobody types anything: boot the media and it is already working.
    #
    # Matched on "==> checking network", which only the installer prints. The
    # banner was the obvious thing to wait for and was wrong - the getty help
    # text also says "Kiwami installer", so the first version of this passed
    # against a console sitting at a shell prompt.
    if not c.expect("==> checking network", timeout=300):
        no("the guided installer starts with no keystroke", c.buf)
        return before
    ok("the guided installer starts with no keystroke")

    step("the questions")
    if not c.expect("reachable over your tailnet", timeout=180):
        no("it offers remote access", c.buf)
        return before
    ok("it offers remote access")

    if not reply(c, "n", "where are your machines described", 180):
        no("declining remote access moves on", c.buf)
        return before
    ok("declining remote access moves on")

    # The installer must not offer to put a new machine into Kiwami's own
    # repository. That was the default until the repositories were split, and
    # it produced a machine whose config had nowhere to be pushed and which
    # could therefore never update itself - discovered at the end, after the
    # hardware had been detected into a clone that was then thrown away.
    #
    # Answered with the org's worked example rather than anybody's personal
    # machines, so the test clones something it is allowed to depend on.
    # 1) a repository you already have. The other branch makes one with gh,
    # which would mean this test creating repositories on every run.
    if not reply(c, "1", "Your repository", 60):
        no("it offers to use a repository you already have", c.buf)
        return before
    ok("it offers to use a repository you already have")

    if not reply(c, "github:kiwamios/example-machines",
                 "Machines this flake already describes", 300):
        no("it asks for a repository of your own before offering a new machine", c.buf)
        return before
    ok("it asks for a repository of your own before offering a new machine")

    # The menu offered "n) a new machine" and then refused n, and later
    # crashed on it. Both are asserted separately because they broke
    # separately.
    if not c.expect("n) a new machine", timeout=30):
        no("the host menu offers a new machine", c.buf)
        return before
    ok("the host menu offers a new machine")

    if not c.expect("Which?", timeout=30):
        no("the host menu asks", c.buf)
        return before

    # Junk must re-ask. The installer is the one program where giving up
    # leaves the machine unusable - a question that rejects an answer and
    # exits cost a whole reinstall to discover.
    if reply(c, "zzz", "Which?", 30):
        ok("junk at the menu re-prompts instead of exiting")
    else:
        no("junk at the menu re-prompts instead of exiting", c.buf)
        return before

    if reply(c, "n", "Name for this machine", 60):
        ok("n is accepted and asks for a name")
    else:
        no("n is accepted and asks for a name", c.buf)
        return before

    # A new machine needs somewhere to write its hardware, so it offers to
    # clone. This is where the flow used to offer "n) a new machine" and then
    # refuse, having asked before it had anywhere to write.
    if not reply(c, "guidedtest", "Clone", 60):
        no("a new machine is offered somewhere to be recorded", c.buf)
        return before
    ok("a new machine is offered somewhere to be recorded")

    if not reply(c, "", "which disk", 900):
        no("it reaches the disk question", c.buf)
        return before
    ok("it reaches the disk question")

    # Straight to encryption, because /home on a second disk is only offered
    # when there is a second disk and this VM has one. That question belongs
    # to `just vm install-test`, which attaches scratch disks for it.
    #
    # This expected it anyway and had been failing on it - invisibly, because
    # the suite was booting an image whose installer never started, so it
    # never got this far to disagree.
    if not reply(c, "1", "Encrypt the disk", 120):
        no("it asks about encryption", c.buf)
        return before
    ok("it asks about encryption")

    step("the way out")
    # The review offers [i] install [e] edit [a] abort. Taking the abort is
    # what keeps this test cheap, and it has to work: it is also what a person
    # does when the layout is wrong.
    if not reply(c, "", "abort", 180):
        no("it shows the layout for review before writing", c.buf)
        return before
    ok("it shows the layout for review before writing")

    if reply(c, "a", "install", 120):
        ok("aborting leaves a shell that says how to start again")
    else:
        no("aborting leaves a shell that says how to start again", c.buf)

    return before


def offline():
    """The same boot with no network at all.

    This is the shape of the bug that made the guided installer useless on the
    first real machine: it found no network and exited. It cannot join a wifi
    network here - the VM has no wifi device - so the claim is narrower and
    honest: it must say what is wrong and how to continue, never just stop.
    """
    step("the same thing with the network cut")
    c = boot()
    # Down before it looks. The installer reaches its network check within
    # seconds of the console appearing, so this cannot wait for a prompt.
    time.sleep(2)
    qmp("set_link", name="n0", up=False)

    if not c.expect("Kiwami installer", timeout=300):
        no("it still starts with no network", c.buf)
        return
    ok("it still starts with no network")

    # Whatever it decides, it must explain. Silence, or a bare prompt with no
    # word about the network, is the failure being tested for.
    if c.expect("no network", timeout=240) or c.expect("wifi", timeout=5):
        ok("it says the network is the problem")
    else:
        no("it says the network is the problem", c.buf)
        return

    if c.expect("kiwami install --guided", timeout=120):
        ok("it leaves the command to run again")
    else:
        no("it leaves the command to run again", c.buf)


def main():
    before = conversation()

    step("the disk")
    # Nothing above should have touched it.
    if DISK.exists() and digest(DISK) == before:
        ok("the disk was never written")
    else:
        no("the disk was never written")

    offline()

    print()
    if failed:
        print(f"\033[1;31m==> {failed} failed, {passed} passed\033[0m")
        print(f"\nFull console log: {os.environ['SERIAL_LOG']}")
        return 1
    print(f"\033[1;32m==> the guided installer holds up a conversation ({passed} checks)\033[0m")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        subprocess.run([str(DIR / "stop-vm.sh")], stdout=subprocess.DEVNULL)

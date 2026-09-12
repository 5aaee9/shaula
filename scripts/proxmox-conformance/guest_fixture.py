"""Execute the packaged Bash bootstrap against files and command stubs in a temp dir."""

import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
TEMPLATE = REPOSITORY / "templates" / "proxmox"
JIT_CANARY = "fixture-jit-'$(never-execute)-canary"


def bash_executable():
    configured = os.environ.get("SHAULA_TEST_BASH")
    if configured:
        return configured
    if os.name == "nt":
        candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files"))
        candidate /= "Git/bin/bash.exe"
        if candidate.is_file():
            return str(candidate)
    discovered = shutil.which("bash")
    if not discovered:
        raise RuntimeError("Bash is required; set SHAULA_TEST_BASH to its executable")
    return discovered


COMMANDS = """#!/bin/bash
set -euo pipefail
command_name=${0##*/}
{
  printf '%s' "$command_name"
  printf ' <%s>' "$@"
  printf '\\n'
} >> "$GUEST_ROOT/trace"
case "$command_name" in
  chmod|chown|udevadm|id) ;;
  blkid)
    # Provider 0.5.0 emits uppercase CIDATA; label lookup is case-sensitive.
    test "$#" -eq 2 && test "$1" = -L && test "$2" = CIDATA
    test "${BLKID_STATUS:-0}" -eq 0 || exit "$BLKID_STATUS"
    printf '%s\\n' "$GUEST_ROOT/dev/sr0"
    ;;
  findmnt)
    test "${FINDMNT_STATUS:-0}" -eq 0 || exit "$FINDMNT_STATUS"
    printf '%s\\n' "$GUEST_ROOT/seed mount"
    ;;
  umount) exit "${UMOUNT_STATUS:-0}" ;;
  runuser)
    test "$#" -eq 5
    test "$1" = -u && test "$2" = runner && test "$3" = --
    shift 3
    exec "$@"
    ;;
  *) exit 90 ;;
esac
"""

LISTENER = """#!/bin/bash
set -euo pipefail
test "$#" -eq 1 && test "$1" = run
test "${ACTIONS_RUNNER_INPUT_JITCONFIG:-}" = "$(cat "$GUEST_ROOT/expected-jit")"
test ! -e "$GUEST_ROOT/var/lib/shaula/jit-config"
test ! -e "$GUEST_ROOT/var/lib/shaula/pre-start"
test -d "$GUEST_ROOT/var/lib/shaula/started"
printf 'listener-env-ok\\n' >> "$GUEST_ROOT/trace"
unset ACTIONS_RUNNER_INPUT_JITCONFIG
exit "${LISTENER_STATUS:-0}"
"""


class GuestFixture:
    def __init__(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shaula-proxmox-guest-")
        self.root = Path(self.temporary.name)
        self.bash = bash_executable()
        self.environment = os.environ.copy()
        self.environment.pop("ACTIONS_RUNNER_INPUT_JITCONFIG", None)
        if os.name == "nt":
            converted = subprocess.run(
                [self.bash, "-c", 'cygpath -u "$1"', "fixture", str(self.root)],
                check=True,
                capture_output=True,
                text=True,
                timeout=10,
            )
            shell_root = converted.stdout.strip()
        else:
            shell_root = str(self.root)
        self.environment["GUEST_ROOT"] = shell_root
        self.environment["BASH_ENV"] = str(self.root / "fixture-environment")
        self.write("var/lib/shaula/jit-config", JIT_CANARY)
        self.write("expected-jit", JIT_CANARY)
        self.write(
            "var/lib/shaula/pre-start",
            """test -z "${ACTIONS_RUNNER_INPUT_JITCONFIG:-}"
printf 'pre-start\\n' >> "$GUEST_ROOT/trace"
""",
        )
        self.write("opt/actions-runner/bin/Runner.Listener", LISTENER, executable=True)
        self.write("dev/sr0", "synthetic block device")
        for directory in ["var/lib/cloud", "run/cloud-init", "seed mount"]:
            (self.root / directory).mkdir(parents=True)
        for command in [
            "chmod",
            "chown",
            "udevadm",
            "id",
            "blkid",
            "findmnt",
            "umount",
            "runuser",
        ]:
            self.write(f"commands/{command}", COMMANDS, executable=True)

        # Retain the production control flow. Only absolute guest paths and
        # the block-device predicate are replaced; host operations are stubs.
        source = (TEMPLATE / "bootstrap.tftpl").read_text(encoding="utf-8")
        guest_paths = r"/(?:var/lib/shaula|opt/actions-runner|var/lib/cloud|run/cloud-init|_work)(?:/[\w.-]+)*"
        source = re.sub(
            guest_paths, lambda match: '"${GUEST_ROOT}' + match[0] + '"', source
        )
        self.write("bootstrap", source)
        self.write(
            "fixture-environment",
            """export PATH="$GUEST_ROOT/commands:$PATH"
test() {
  if [[ "${1:-}" == -b ]]; then
    [[ "$2" == "$GUEST_ROOT/dev/sr0" && -f "$2" ]]
  else
    builtin test "$@"
  fi
}
""",
        )

    def close(self):
        self.temporary.cleanup()

    def write(self, relative, content, executable=False):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8", newline="\n")
        if executable:
            path.chmod(0o700)

    def run(self, **environment):
        return subprocess.run(
            [self.bash, str(self.root / "bootstrap")],
            env=self.environment | environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )

    def trace(self):
        path = self.root / "trace"
        return path.read_text(encoding="utf-8") if path.exists() else ""

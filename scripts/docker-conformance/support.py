"""Protected files and subprocess transport for an administrator-run smoke test."""

import datetime
import json
import os
import re
import selectors
import signal
import stat
import subprocess
import time
import uuid
from pathlib import Path

from safety import Rejected, digest, require


def private_directory(path):
    path = Path(path)
    require(not path.is_symlink(), "directory_symlink")
    info = path.stat()
    require(
        stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid(), "directory_owner"
    )
    require(info.st_mode & 0o077 == 0, "directory_permissions_require_0700")
    return path.resolve()


def private_bytes(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(
            stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid(), "input_owner"
        )
        require(info.st_mode & 0o077 == 0, "input_permissions_require_0600")
        require(info.st_size <= 8 * 1024 * 1024, "input_size")
        return stream.read()


def private_json(path):
    return json.loads(private_bytes(path))


def save(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def save_json(path, data):
    save(path, (json.dumps(data, indent=2, sort_keys=True) + "\n").encode())


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def source_digest(workspace):
    # Only fixed generated root entries are excluded. Helpers, binaries and
    # extensionless files are source too; never silently follow a source link.
    generated_directories = {".terraform", ".docker-conformance"}
    generated_files = {
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "errored.tfstate",
        ".terraform.tfstate.lock.info",
        "crash.log",
    }
    files = []

    def unreadable(_error):
        raise Rejected("template_directory_unreadable")

    for current, directories, names in os.walk(
        workspace, followlinks=False, onerror=unreadable
    ):
        current = Path(current)
        for name in directories[:]:
            child = current / name
            require(not child.is_symlink(), "template_directory_symlink")
            if current == workspace and name in generated_directories:
                directories.remove(name)
        for name in names:
            file = current / name
            require(not file.is_symlink(), "template_file_symlink")
            require(stat.S_ISREG(file.stat().st_mode), "template_file_not_regular")
            if current == workspace and name in generated_files:
                continue
            files.append(file)
    require(files, "template_files_missing")
    content = bytearray()
    for file in sorted(files):
        require(not file.is_symlink(), "template_file_symlink")
        content.extend(file.relative_to(workspace).as_posix().encode() + b"\0")
        content.extend(str(stat.S_IMODE(file.stat().st_mode)).encode() + b"\0")
        content.extend(digest(file.read_bytes()).encode() + b"\n")
    return digest(content)


class Commands:
    OUTPUT_LIMIT = 16 * 1024 * 1024  # Per stream, both in memory and on disk.

    def __init__(self, workspace, terraform, docker, socket):
        self.workspace = workspace
        self.terraform = str(Path(terraform).resolve(strict=True))
        self.docker = str(Path(docker).resolve(strict=True))
        require(socket.startswith("unix:///"), "docker_socket_must_be_local_unix")
        require(
            stat.S_ISSOCK(os.stat(socket.removeprefix("unix://")).st_mode),
            "docker_socket_missing",
        )
        self.socket = socket
        self.env = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(workspace / ".docker-conformance"),
            "LANG": "C.UTF-8",
            "TF_IN_AUTOMATION": "1",
            "CHECKPOINT_DISABLE": "1",
        }
        for key in ("SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE"):
            if key in os.environ:
                self.env[key] = os.environ[key]

    def _drain(self, child, logs, phase, timeout):
        output = bytearray()
        counts = {child.stdout: 0, child.stderr: 0}
        deadline = time.monotonic() + timeout
        with selectors.DefaultSelector() as selector:
            for pipe in counts:
                selector.register(pipe, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                require(remaining > 0, phase + "_timeout_outcome_uncertain")
                for event, _mask in selector.select(min(remaining, 1)):
                    pipe = event.fileobj
                    chunk = os.read(pipe.fileno(), 65536)
                    if not chunk:
                        selector.unregister(pipe)
                        continue
                    retained = chunk[: self.OUTPUT_LIMIT - counts[pipe]]
                    logs[pipe].write(retained)
                    counts[pipe] += len(retained)
                    if pipe is child.stdout:
                        output.extend(retained)
                    require(
                        len(retained) == len(chunk),
                        phase + "_output_limit_outcome_uncertain",
                    )
            try:
                child.wait(timeout=max(0.001, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                raise Rejected(phase + "_timeout_outcome_uncertain") from None
        return bytes(output)

    def run(self, arguments, phase, timeout=600, accepted=(0,)):
        require(re.fullmatch(r"[a-z0-9_]{1,64}", phase), "command_phase_invalid")
        directory = self.workspace / ".docker-conformance" / "commands"
        directory.mkdir(mode=0o700, exist_ok=True)
        private_directory(directory)
        basename = phase + "-" + uuid.uuid4().hex
        # Exclusive 0600 files retain raw diagnostics without following links or
        # reusing a previous command's evidence. They may contain credentials.
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        out_fd = os.open(directory / (basename + ".stdout"), flags, 0o600)
        with os.fdopen(out_fd, "wb", buffering=0) as out_log:
            err_fd = os.open(directory / (basename + ".stderr"), flags, 0o600)
            with (
                os.fdopen(err_fd, "wb", buffering=0) as err_log,
                subprocess.Popen(
                    arguments,
                    cwd=self.workspace,
                    env=self.env,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    start_new_session=True,
                ) as child,
            ):
                try:
                    stdout = self._drain(
                        child,
                        {child.stdout: out_log, child.stderr: err_log},
                        phase,
                        timeout,
                    )
                except BaseException:
                    # Outcome stays uncertain; this is not a lifecycle
                    # fencing proof for any descendant that escaped PGID.
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    child.wait()
                    raise
                finally:
                    os.fsync(out_log.fileno())
                    os.fsync(err_log.fileno())
        require(child.returncode in accepted, phase + "_exit_" + str(child.returncode))
        return stdout

    def tf(self, *arguments, phase):
        return self.run([self.terraform, *arguments], phase)

    def tf_json(self, *arguments, phase):
        return json.loads(self.tf(*arguments, phase=phase))

    def docker_run(self, *arguments, phase, accepted=(0,)):
        return self.run(
            [self.docker, "--host", self.socket, *arguments],
            phase,
            timeout=60,
            accepted=accepted,
        )

    def docker_json(self, *arguments, phase):
        return json.loads(self.docker_run(*arguments, phase=phase))

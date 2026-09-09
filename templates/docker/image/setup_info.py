"""Bounded, optional setup-log delivery. Run network work in a killable child."""

import json
import os
import re
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

GROUP = "Terraform apply (runner provisioning)"
DESCRIPTOR_PATH = Path("/shaula/setup_info.json")
RUNNER_ROOT = Path("/home/runner")
MAX_BYTES = 1024 * 1024
MAX_LINES = 20000
MAX_DESCRIPTOR_BYTES = 8192
TEMP_NAME = ".setup_info.shaula.tmp"


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate member")
        result[key] = value
    return result


def read_regular(path, limit):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
            raise ValueError("file rejected")
        value = source.read(limit + 1)
        if len(value) > limit:
            raise ValueError("file exceeds limit")
        return value


def validate_descriptor(value):
    if value == {"status": "disabled"}:
        return value
    if not isinstance(value, dict) or set(value) != {
        "status", "url", "capability", "expires_at", "wait_seconds"
    } or value["status"] != "enabled":
        raise ValueError("descriptor rejected")
    url = value["url"]
    if not isinstance(url, str) or len(url) > 2048 or any(c.isspace() for c in url):
        raise ValueError("URL rejected")
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.query or parsed.fragment
            or parsed.port == 0
            or not re.fullmatch(r"/runner/v1/generations/[A-Za-z0-9_-]+/setup-info", parsed.path)):
        raise ValueError("URL rejected")
    if not isinstance(value["capability"], str) or not re.fullmatch(
        r"[A-Za-z0-9_-]{32,512}", value["capability"]
    ):
        raise ValueError("capability rejected")
    if type(value["wait_seconds"]) is not int or not 1 <= value["wait_seconds"] <= 300:
        raise ValueError("wait rejected")
    if type(value["expires_at"]) is not int or value["expires_at"] <= time.time():
        raise ValueError("expiry rejected")
    return value


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("redirect rejected")


def parse_entries(raw, own_only=False):
    if len(raw) > MAX_BYTES:
        raise ValueError("setup info exceeds limit")
    entries = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
    if not isinstance(entries, list) or len(entries) > 128:
        raise ValueError("setup info rejected")
    lines = 0
    for entry in entries:
        if not isinstance(entry, dict):
            raise TypeError("setup entry rejected")
        if own_only and (set(entry) != {"Group", "Detail"} or entry["Group"] != GROUP
                         or not isinstance(entry["Detail"], str)):
            raise ValueError("unexpected setup group")
        if entry.get("Group") is not None and not isinstance(entry["Group"], str):
            raise ValueError("setup group rejected")
        if entry.get("Detail") is not None and not isinstance(entry["Detail"], str):
            raise ValueError("setup detail rejected")
        lines += (entry.get("Detail") or "").count("\n") + 1
    if lines > MAX_LINES or (own_only and len(entries) != 1):
        raise ValueError("setup info exceeds bounds")
    return entries


def fetch_entries(descriptor):
    # Explicitly disable ambient proxy configuration and all redirects. HTTPSHandler
    # uses the system trust store; no configurable insecure context is accepted.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    deadline = min(time.monotonic() + descriptor["wait_seconds"],
                   time.monotonic() + descriptor["expires_at"] - time.time())
    while (remaining := deadline - time.monotonic()) > 0:
        request = urllib.request.Request(descriptor["url"], headers={
            "Authorization": "Bearer " + descriptor["capability"],
            "Accept": "application/json",
        })
        retry_after = 1.0
        try:
            with opener.open(request, timeout=min(5.0, remaining)) as response:
                if response.status == 200:
                    if response.headers.get_content_type() != "application/json":
                        raise ValueError("content type rejected")
                    return parse_entries(response.read(MAX_BYTES + 1), own_only=True)
                if response.status != 202:
                    raise ValueError("response rejected")
                retry_after = response.headers.get("Retry-After", "1")
        except urllib.error.HTTPError as error:
            if error.code != 429 and error.code < 500:
                raise ValueError("response rejected") from None
            retry_after = error.headers.get("Retry-After", "1")
            error.close()
        except (urllib.error.URLError, TimeoutError, OSError):
            pass
        try:
            delay = max(0.5, min(5.0, float(retry_after)))
        except (ValueError, TypeError):
            delay = 1.0
        remaining = deadline - time.monotonic()
        if remaining > 0:
            time.sleep(min(delay, remaining))
    raise TimeoutError("setup info deadline")


def merge_entries(runner_root, incoming):
    root_fd = os.open(runner_root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        existing = []
        try:
            existing = parse_entries(read_regular(runner_root / ".setup_info", MAX_BYTES))
        except FileNotFoundError:
            pass
        merged = [entry for entry in existing if entry.get("Group") != GROUP] + incoming
        raw = json.dumps(merged, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        parse_entries(raw)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
        descriptor = os.open(TEMP_NAME, flags, 0o600, dir_fd=root_fd)
        try:
            with os.fdopen(descriptor, "wb") as target:
                target.write(raw)
                target.flush()
                os.fsync(target.fileno())
            # Refuse a symlink introduced after the initial read as well.
            try:
                current = os.stat(".setup_info", dir_fd=root_fd, follow_symlinks=False)
                if not stat.S_ISREG(current.st_mode):
                    raise ValueError("setup destination rejected")
            except FileNotFoundError:
                pass
            os.replace(TEMP_NAME, ".setup_info", src_dir_fd=root_fd, dst_dir_fd=root_fd)
            os.fsync(root_fd)
        finally:
            try:
                os.unlink(TEMP_NAME, dir_fd=root_fd)
            except FileNotFoundError:
                pass
    finally:
        os.close(root_fd)


def prepare(descriptor_path=DESCRIPTOR_PATH, runner_root=RUNNER_ROOT):
    """All setup failures degrade; never inspect or consume JIT here."""
    try:
        try:
            raw = read_regular(descriptor_path, MAX_DESCRIPTOR_BYTES)
        finally:
            # Remove even malformed/oversize descriptors, without following symlinks.
            try:
                descriptor_path.unlink()
            except FileNotFoundError:
                pass
        descriptor = validate_descriptor(json.loads(raw, object_pairs_hook=unique_object))
        if descriptor["status"] == "disabled":
            return True
        timeout = min(descriptor["wait_seconds"], descriptor["expires_at"] - time.time())
        result = subprocess.run(
            [sys.executable, "-I", str(Path(__file__).resolve()), "--worker", str(runner_root)],
            input=json.dumps(descriptor).encode(), stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=max(0.001, timeout), check=False,
            env={key: value for key, value in os.environ.items()
                 if not key.upper().startswith(("ACTIONS_RUNNER_INPUT_", "SHAULA_SETUP_"))},
        )
        return result.returncode == 0
    except FileNotFoundError:
        return True
    except (OSError, ValueError, TypeError, RecursionError, subprocess.TimeoutExpired):
        return False


def worker():
    try:
        if len(sys.argv) != 3 or sys.argv[1] != "--worker":
            return 1
        descriptor = validate_descriptor(json.loads(sys.stdin.buffer.read(MAX_DESCRIPTOR_BYTES + 1)))
        merge_entries(Path(sys.argv[2]), fetch_entries(descriptor))
        return 0
    except (OSError, ValueError, TypeError, RecursionError):
        return 1


if __name__ == "__main__":
    sys.exit(worker())

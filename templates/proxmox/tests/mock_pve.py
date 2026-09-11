"""Credential-free PVE fixture for the real provider, never a guest simulator."""

import json
from email.parser import BytesParser
from email.policy import default
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlsplit


class MockPve(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.inventory = [self.template(9000)]
        self.vms = {}
        self.isos = {}
        self.events = []
        self.failures = []

    @staticmethod
    def template(vmid):
        return {
            "vmid": vmid,
            "name": "GitHub-Runner",
            "node": "pve-test",
            "template": 1,
            "type": "qemu",
            "status": "stopped",
        }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, data, status=200):
        payload = json.dumps({"data": data}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        self.dispatch("GET")

    def do_POST(self):
        self.dispatch("POST")

    def do_PUT(self):
        self.dispatch("PUT")

    def do_DELETE(self):
        self.dispatch("DELETE")

    def task(self, event):
        self.server.events.append(event)
        self.reply(f"UPID:pve-test:task-{len(self.server.events)}")

    def dispatch(self, method):
        try:
            self.handle_request(method)
        except (AssertionError, IndexError, KeyError, TypeError, ValueError) as error:
            self.server.failures.append(str(error))
            self.reply(None, 500)

    def handle_request(self, method):
        server = self.server
        assert (
            self.headers.get("Authorization")
            == "PVEAPIToken=test@pve!shaula=synthetic=secret"
        ), "API token was not split on the first equals sign"
        url = urlsplit(self.path)
        path = unquote(url.path).removeprefix("/api2/json")
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        form = (
            parse_qs(body.decode())
            if "multipart/" not in self.headers.get("Content-Type", "")
            else {}
        )
        if method == "GET" and path == "/cluster/resources":
            assert parse_qs(url.query) == {"type": ["vm"]}
            return self.reply(server.inventory)
        if method == "GET" and path == "/cluster/nextid":
            candidate = int(parse_qs(url.query)["vmid"][0])
            while candidate in server.vms:
                candidate += 1
            return self.reply(candidate)
        if method == "GET" and path == "/nodes/pve-test/storage":
            return self.reply(
                [
                    {
                        "storage": "local",
                        "type": "dir",
                        "content": "iso",
                        "enabled": 1,
                        "active": 1,
                    }
                ]
            )
        if method == "GET" and path == "/nodes/pve-test/storage/local/content":
            return self.reply(
                [
                    {
                        "volid": f"local:iso/{name}",
                        "format": "iso",
                        "content": "iso",
                        "size": len(data),
                    }
                    for name, data in server.isos.items()
                ]
            )
        if method == "POST" and path == "/nodes/pve-test/storage/local/upload":
            message = BytesParser(policy=default).parsebytes(
                f"Content-Type: {self.headers['Content-Type']}\r\nMIME-Version: 1.0\r\n\r\n".encode()
                + body
            )
            files = [part for part in message.iter_parts() if part.get_filename()]
            assert len(files) == 1
            name = files[0].get_filename()
            assert name not in server.isos, "Existing ISO must never be overwritten"
            server.isos[name] = files[0].get_payload(decode=True)
            return self.task("upload")
        if (
            method == "GET"
            and path.startswith("/nodes/pve-test/tasks/")
            and path.endswith("/status")
        ):
            return self.reply({"status": "stopped", "exitstatus": "OK"})
        if method == "POST" and path == "/nodes/pve-test/qemu/9000/clone":
            vmid = int(form["newid"][0])
            assert vmid not in server.vms and form["full"] == ["1"]
            server.vms[vmid] = {
                "config": {
                    "name": form["name"][0],
                    "scsi0": f"local-lvm:vm-{vmid}-disk-0,size=8G",
                    "net0": "virtio=BC:24:11:00:00:01,bridge=vmbr0",
                    "ide2": f"local:vm-{vmid}-cloudinit,media=cdrom",
                },
                "running": False,
            }
            return self.task("clone")
        if path.startswith("/nodes/pve-test/qemu/"):
            segments = path.split("/")
            vmid = int(segments[4])
            vm = server.vms.get(vmid)
            if vm is None:
                return self.reply(None, 404)
            if method == "GET" and path.endswith("/config"):
                return self.reply(vm["config"])
            if method == "PUT" and path.endswith("/config"):
                assert not {"scsi0", "net0", "cores", "memory"}.intersection(form), (
                    "Inherited hardware must not be managed"
                )
                for key, value in form.items():
                    vm["config"][key] = (
                        int(value[0]) if key in {"onboot", "protection"} else value[0]
                    )
                assert vm["config"]["ide2"].startswith("local:iso/")
                server.events.append("attach")
                return self.reply(None)
            if method == "GET" and path.endswith("/status/current"):
                return self.reply({"status": "running" if vm["running"] else "stopped"})
            if method == "POST" and path.endswith("/status/start"):
                vm["running"] = True
                return self.task("start")
            if method == "POST" and path.endswith("/status/stop"):
                vm["running"] = False
                return self.task("stop")
            if method == "DELETE" and len(segments) == 5:
                assert not vm["running"], "VM deletion must follow stop"
                del server.vms[vmid]
                return self.task("vm-delete")
        if method == "DELETE" and path.startswith(
            "/nodes/pve-test/storage/local/content/local:iso/"
        ):
            assert not server.vms, "ISO deletion must follow VM deletion"
            name = path.rsplit("/", 1)[1]
            assert name in server.isos, "Only the owned ISO can be deleted"
            del server.isos[name]
            return self.task("iso-delete")
        raise AssertionError(f"Unexpected mock request: {method} {path}")

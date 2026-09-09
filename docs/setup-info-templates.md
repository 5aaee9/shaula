# Prepare a Template with Setup Info delivery

[文档索引](README.md) · [项目首页](../README.md)

Run the commands below from the repository root.

The bundled `docker` and `kubernetes` sources retain their existing v1 image and
input pins. Editing the bootstrap source does not change those immutable images.
The current workstation has no Docker or Podman engine; no new image digest or
passing platform conformance is claimed by this guide.

1. Build `templates/docker/image/Dockerfile` in a trusted Linux build environment,
   retaining the reviewed official Runner base digest documented in that image
   directory. The Dockerfile installs both `bootstrap-shim` and `setup_info.py`.
2. Run `python3 -m unittest discover -s templates/docker/image -p 'test_*.py'`.
3. Publish/import the resulting image through the deployment's normal image
   distribution process and obtain its actual repository manifest digest.
   A local image configuration ID is not a repository manifest digest.
4. Generate a separate source, using that new image pin:

   ```sh
   python3 templates/setup-info/prepare.py --platform docker \
     --image 'registry.example/shaula-runner:setup-v2@sha256:<actual-64-hex-digest>' \
     --output /new/template-source/docker-setup-info
   ```

   Use `--platform kubernetes` for the matching Kubernetes source. The generator
   refuses existing output directories and any existing v1 image digest. It
   keeps resource cardinality and provider lock files, updates image aliases,
   adds the descriptor upload/Secret staging, and declares the v2 input and
   `shaula.setup-info/v1` manifest contracts. It cannot prove image contents or
   turn the staged Kubernetes provider/image evidence into passing conformance.

5. Configure the daemon's optional `setup_info` section with a distinct loopback
   listener and a Runner-reachable HTTPS `advertised_origin`. A trusted reverse
   proxy must route only `/runner/v1/generations/*/setup-info` to this listener;
   it must preserve Authorization without logging it and present a certificate
   trusted by the Runner image. Management OIDC and state/control routes stay
   on their existing listeners. Proxy reachability/certificate validation needs
   a real Runner smoke test; binding loopback alone does not prove it.
6. Import/publish the new source through ordinary Template admission. Existing
   Template revisions and live Generation inputs remain unchanged. A v2
   Generation receives `{"status":"disabled"}` when delivery is not configured.

The helper removes its staged descriptor before network access, enforces a hard
total deadline around its network/file worker, and writes `.setup_info` before
the Listener starts. Failure preserves existing image setup data and continues
the JIT startup path. Kubernetes waits only in the main container; waiting in
an init container would block the provider's Pod Running gate.

Validate the first real GitHub job's **Set up job** group, slow apply, delivery
timeout, invalid TLS/redirect rejection, old setup entries, and deletion/restart
history through [spec 0019](specs/0019-workflow-jobs-and-operation-logs.md).

"""Generate a separate v2 Template source from a newly built, verified image pin."""
import argparse
import hashlib
import json
import re
import shutil
from pathlib import Path


def prepare(platform, image, output):
    if platform not in {"docker", "kubernetes"}:
        raise ValueError("platform rejected")
    if not re.fullmatch(r"[A-Za-z0-9:/._-]+@sha256:[0-9a-f]{64}", image):
        raise ValueError("image must be an immutable repository@sha256 pin")
    source = Path(__file__).resolve().parents[1] / platform
    profile = (source / "profile.yaml").read_text()
    digest = image.rsplit("@sha256:", 1)[1]
    if digest in re.findall(r"@sha256:([0-9a-f]{64})", profile):
        raise ValueError("existing v1 image pins cannot claim the new bootstrap contract")
    if output.exists():
        raise ValueError("output must be a new directory; existing revisions are never overwritten")
    alias = image.split("@", 1)[0]
    main = (source / "main.tf").read_text()
    main, count = re.subn(r"(?m)^(\s*)jit_config(\s*)= string$",
                         r"\1jit_config\2= string\n\1setup_info = any", main)
    if count != 1:
        raise ValueError("source input declaration changed; review the generator")
    main, count = re.subn(r'(runner_image\s*= optional\(string, ")[^"]+("\))',
                         lambda match: match[1] + alias + match[2], main)
    if count != 1:
        raise ValueError("source image declaration changed; review the generator")
    if platform == "docker":
        anchor = '  command = ["/usr/local/bin/bootstrap-shim"]'
        replacement = '''  upload {
    file    = "/shaula/setup_info.json"
    content = jsonencode(var.shaula.setup_info)
  }

''' + anchor
        main = main.replace(anchor, replacement)
        if main.count('file    = "/shaula/setup_info.json"') != 1:
            raise ValueError("Docker upload anchor changed")
    else:
        anchor = '    "jit_config" = var.shaula.jit_config'
        main = main.replace(anchor, anchor + '\n    "setup_info.json" = jsonencode(var.shaula.setup_info)')
        anchor = "cp /in/jit_config /stage/jit_config"
        main = main.replace(anchor, "cp /in/setup_info.json /stage/setup_info.json && chmod 400 /stage/setup_info.json && " + anchor)
        if main.count('"setup_info.json" = jsonencode') != 1 or main.count("cp /in/setup_info.json") != 1:
            raise ValueError("Kubernetes bootstrap staging anchor changed")
    policy = ("# Setup Info bootstrap runtime policy\n\n"
              "Uses the reviewed image/bootstrap-shim and setup_info.py source.\n"
              "The v2 system descriptor is staged at /shaula/setup_info.json.\n"
              "Only the main runner shim waits; init containers never wait for apply.\n"
              "HTTPS-only, no redirects, bounded requests and a hard process deadline.\n"
              "Only approved Create apply text enters /home/runner/.setup_info.\n"
              "Read/merge failures degrade; JIT failures remain fatal.\n"
              "The original platform JIT, trust and Create/Destroy constraints continue.\n"
              "A new image digest is required; conformance remains independently verified.\n")
    profile, count = re.subn(r"runner_image_digests:\n(?:  - .*\n)+", "runner_image_digests:\n  - " + image + "\n", profile)
    if count != 1:
        raise ValueError("source image manifest changed")
    profile = re.sub(r"(?m)^runtime_policy_digest:.*$", "runtime_policy_digest: sha256:" + hashlib.sha256(policy.encode()).hexdigest(), profile)
    profile += "input_contract_version: 2\nsetup_info_contract: shaula.setup-info/v1\n"
    parameters = json.loads((source / "schemas/parameters.schema.json").read_text())
    parameters["properties"]["runner_image"]["enum"] = [alias]
    output.mkdir(parents=True)
    (output / "schemas").mkdir()
    (output / "main.tf").write_text(main, encoding="utf-8", newline="\n")
    (output / "profile.yaml").write_text(profile, encoding="utf-8", newline="\n")
    (output / "runtime-policy.md").write_text(policy, encoding="utf-8", newline="\n")
    (output / "schemas/parameters.schema.json").write_text(json.dumps(parameters, indent=2) + "\n", encoding="utf-8")
    shutil.copyfile(source / "schemas/bindings.schema.json", output / "schemas/bindings.schema.json")
    shutil.copyfile(source / ".terraform.lock.hcl", output / ".terraform.lock.hcl")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["docker", "kubernetes"], required=True)
    parser.add_argument("--image", required=True, help="Actual newly built image repository@sha256 digest")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    prepare(args.platform, args.image, args.output)
    print(args.output)

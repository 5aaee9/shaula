# Docker template

The default target is `unix:///var/run/docker.sock`. To use Docker on a remote
host, publish a new Template Revision with the following **bindings** (not Fleet
inputs). The existing bindings JSON editor accepts these fields.

## SSH password

```json
{
  "docker_host": "ssh://runner@docker.example.com:2222",
  "ssh_password": "<SSH login password>",
  "ssh_known_hosts": "[docker.example.com]:2222 ssh-ed25519 <verified host public key>\n"
}
```

## SSH private key

```json
{
  "docker_host": "ssh://runner@docker.example.com:2222",
  "ssh_private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\n<key contents>\n-----END OPENSSH PRIVATE KEY-----\n",
  "ssh_private_key_passphrase": "<optional key passphrase>",
  "ssh_known_hosts": "[docker.example.com]:2222 ssh-ed25519 <verified host public key>\n"
}
```

- Supply exactly one of `ssh_password` and `ssh_private_key`. The private key is
  its full text, **not a path**. Omit `ssh_private_key_passphrase` for an
  unencrypted key. Passwords/passphrases must be nonempty single-line strings.
- The SSH port is optional and defaults to 22. IPv6 targets use brackets, e.g.
  `ssh://runner@[2001:db8::1]:2222`. Do not embed passwords in the URL.
- OpenSSH and Docker CLI must be installed on the Shaula host. The remote user
  must be able to run `docker system dial-stdio` without sudo and access the
  remote Docker daemon. Preload the pinned Runner image on that daemon.
- Host verification is always enabled. `ssh_known_hosts` contains host **public**
  keys, not the client's private key. Verify the fingerprint through a trusted
  channel; `ssh-keyscan` output alone is not proof of identity. If omitted, seed
  the Shaula service user's known_hosts file. Ambient SSH configuration and
  ssh-agent are not used.
- Passwords, private keys and passphrases are write-only sensitive bindings.
  Terraform create/destroy and Docker inspect/copy/start use the same credentials;
  no credential is mounted or copied into the Runner. See [runtime-policy.md](runtime-policy.md)
  for retained-state and temporary-file security requirements.
- Existing Generations retain their original artifact and bindings. Publish the
  updated artifact as a new Template Version/Revision to enable SSH for new ones.

Local Unix/named-pipe targets remain supported. SSH credential fields are only
valid for SSH targets; unauthenticated TCP targets remain rejected.

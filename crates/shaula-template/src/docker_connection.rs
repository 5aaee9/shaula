//! Invocation-scoped OpenSSH authentication shared by Terraform and Docker CLI.
//! Only paths, never credential bytes, enter the child environment or argv.

use serde_json::{Map, Value};
use shaula_core::template::ProfileManifest;
use std::{io::Write, path::Path};

use crate::ssh_helper::{Config, CONFIG_ENV};

const SSH_FIELDS: &[&str] = &[
    "ssh_password",
    "ssh_private_key",
    "ssh_private_key_passphrase",
    "ssh_known_hosts",
];
const ENVIRONMENT_KEYS: &[&str] = &[
    "PATH",
    CONFIG_ENV,
    "SSH_ASKPASS",
    "SSH_ASKPASS_REQUIRE",
    "DISPLAY",
];
pub(crate) const SSH_HOST_PATTERN: &str = r"^ssh://[a-zA-Z0-9_][a-zA-Z0-9_.-]*@([a-zA-Z0-9][a-zA-Z0-9.-]*|\[[0-9a-fA-F:]+\])(:[0-9]{1,5})?$";

pub(crate) fn local_host(host: &str) -> bool {
    host.starts_with("unix:///")
        || host.strip_prefix("npipe:////./pipe/").is_some_and(|name| {
            !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
        })
}

pub(crate) fn ssh_host(host: &str) -> bool {
    regex::Regex::new(SSH_HOST_PATTERN).is_ok_and(|pattern| pattern.is_match(host))
        && url::Url::parse(host).is_ok_and(|url| url.port() != Some(0))
}

fn invalid() -> std::io::Error {
    std::io::Error::other("invalid Docker SSH configuration")
}

fn field<'a>(bindings: &'a Map<String, Value>, name: &str) -> std::io::Result<Option<&'a str>> {
    bindings
        .get(name)
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty() && !value.contains('\0'))
                .ok_or_else(invalid)
        })
        .transpose()
}

/// Kept alive through apply/bootstrap or Destroy; Drop removes all transient
/// files. Durable credentials remain only in the original protected bindings.
pub(crate) struct DockerSsh {
    _directory: tempfile::TempDir,
    environment: Vec<(String, String)>,
}

impl DockerSsh {
    pub(crate) fn prepare(
        manifest: &ProfileManifest,
        bindings: &Map<String, Value>,
    ) -> std::io::Result<Option<Self>> {
        if manifest.platform != "docker" {
            return Ok(None);
        }
        let host = field(bindings, "docker_host")?.unwrap_or("unix:///var/run/docker.sock");
        if local_host(host) {
            return if SSH_FIELDS.iter().any(|key| bindings.contains_key(*key)) {
                Err(invalid())
            } else {
                Ok(None)
            };
        }
        if !ssh_host(host) {
            return Err(invalid());
        }
        let password = field(bindings, "ssh_password")?;
        let key = field(bindings, "ssh_private_key")?;
        let passphrase = field(bindings, "ssh_private_key_passphrase")?;
        if password.is_some() == key.is_some() || passphrase.is_some() && key.is_none() {
            return Err(invalid());
        }
        let secret = password.or(passphrase);
        if secret.is_some_and(|secret| secret.contains(['\r', '\n'])) {
            return Err(invalid());
        }
        let executable = resolve("ssh")?;
        let helper =
            crate::engine::engine_supervisor::supervisor_executable().map_err(|_| invalid())?;
        Self::materialize(bindings, key, secret, &executable, &helper).map(Some)
    }

    fn materialize(
        bindings: &Map<String, Value>,
        key: Option<&str>,
        secret: Option<&str>,
        executable: &Path,
        helper: &Path,
    ) -> std::io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("shaula-docker-ssh-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = builder.tempdir()?;
        let root = directory.path();
        link_helper(helper, &root.join(executable_name("ssh")))?;
        let askpass = root.join(executable_name("shaula-ssh-askpass"));
        link_helper(helper, &askpass)?;
        let empty_config = write_private(root, "config", b"")?;
        let mut arguments = vec!["-F".into(), path_text(&empty_config)?, "-T".into()];
        for option in [
            "StrictHostKeyChecking=yes",
            "IdentityAgent=none",
            "IdentitiesOnly=yes",
            "ForwardAgent=no",
            "ClearAllForwardings=yes",
            "ControlMaster=no",
            "ControlPath=none",
            "ConnectTimeout=15",
            "ConnectionAttempts=1",
            "NumberOfPasswordPrompts=1",
            "KbdInteractiveAuthentication=no",
        ] {
            arguments.extend(["-o".into(), option.into()]);
        }
        arguments.extend([
            "-o".into(),
            format!("BatchMode={}", if secret.is_some() { "no" } else { "yes" }),
            "-o".into(),
            format!(
                "PreferredAuthentications={}",
                if key.is_some() {
                    "publickey"
                } else {
                    "password"
                }
            ),
        ]);
        if let Some(key) = key {
            // OpenSSH requires the final newline; accept pasted CRLF keys too.
            let mut key = zeroize::Zeroizing::new(key.replace("\r\n", "\n"));
            if !key.ends_with('\n') {
                key.push('\n');
            }
            let path = write_private(root, "identity", key.as_bytes())?;
            arguments.extend(["-i".into(), path_text(&path)?]);
        }
        if let Some(hosts) = field(bindings, "ssh_known_hosts")? {
            let path = write_private(root, "known_hosts", hosts.as_bytes())?;
            arguments.extend([
                "-o".into(),
                format!("UserKnownHostsFile=\"{}\"", path_text(&path)?),
                "-o".into(),
                "GlobalKnownHostsFile=none".into(),
            ]);
        }
        let secret_file = secret
            .map(|value| write_private(root, "credential", value.as_bytes()))
            .transpose()?;
        let config = Config {
            executable: executable.to_path_buf(),
            arguments,
            secret_file,
        };
        let config_path = write_private(root, "connection.json", &serde_json::to_vec(&config)?)?;
        let environment = vec![
            // Only our helper is discoverable; do not forward the daemon PATH.
            ("PATH".into(), path_text(root)?),
            (CONFIG_ENV.into(), path_text(&config_path)?),
            ("SSH_ASKPASS".into(), path_text(&askpass)?),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            ("DISPLAY".into(), "shaula:0".into()),
        ];
        Ok(Self {
            _directory: directory,
            environment,
        })
    }

    pub(crate) fn configure(&self, environment: &mut Vec<(String, String)>) {
        environment.retain(|(name, _)| {
            !ENVIRONMENT_KEYS
                .iter()
                .any(|key| name.eq_ignore_ascii_case(key))
                && !name.eq_ignore_ascii_case("SSH_AUTH_SOCK")
                && !name.eq_ignore_ascii_case("DOCKER_SSH_OPTS")
        });
        environment.extend(self.environment.iter().cloned());
    }
}

/// Bootstrap must not inherit Terraform HTTP-backend credentials.
pub(crate) fn bootstrap_environment(
    environment: &[(String, String)],
) -> std::io::Result<Vec<(String, String)>> {
    ENVIRONMENT_KEYS
        .iter()
        .map(|key| {
            environment
                .iter()
                .find(|(name, _)| name == key)
                .cloned()
                .ok_or_else(invalid)
        })
        .collect()
}

fn executable_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn resolve(name: &str) -> std::io::Result<std::path::PathBuf> {
    let paths = std::env::var_os("PATH").ok_or_else(invalid)?;
    std::env::split_paths(&paths)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(executable_name(name)))
        .find(|path| path.is_file())
        .ok_or_else(invalid)?
        .canonicalize()
}

fn path_text(path: &Path) -> std::io::Result<String> {
    let value = path.to_str().ok_or_else(invalid)?;
    // OpenSSH expands percent tokens in identity/known-hosts paths. Fail closed
    // for unusual temporary roots instead of allowing path reinterpretation.
    if value.contains(['\r', '\n', '"', '%']) {
        return Err(invalid());
    }
    #[cfg(windows)]
    let value = value.replace('\\', "/");
    #[cfg(not(windows))]
    if value.contains('\\') {
        return Err(invalid());
    }
    Ok(value.to_string())
}

fn write_private(root: &Path, name: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    let path = root.join(name);
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(&path).map_err(|error| error.error)?;
    Ok(path)
}

fn link_helper(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination)
    }
    #[cfg(not(unix))]
    {
        std::fs::hard_link(source, destination)
            .or_else(|_| std::fs::copy(source, destination).map(|_| ()))
    }
}

#[cfg(test)]
#[path = "docker_connection_tests.rs"]
mod tests;

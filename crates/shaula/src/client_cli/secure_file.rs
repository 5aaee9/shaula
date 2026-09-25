//! Exclusive creation, no symlink traversal, private permissions before secret writes.
use shaula_client::Error;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
pub(super) fn safe_path(path: &Path) -> Result<(), Error> {
    let absolute =
        std::path::absolute(path).map_err(|_| Error::Invalid("invalid credential path"))?;
    for ancestor in absolute.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(Error::Invalid("credential path contains a symlink"));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(Error::Invalid("credential path contains a reparse point"));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Invalid("cannot inspect credential path")),
        }
    }
    Ok(())
}
pub(super) fn private_directory(path: &Path) -> Result<(), Error> {
    safe_path(path)?;
    if !path.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(path)
            .map_err(|_| Error::Invalid("cannot create credential directory"))?;
        #[cfg(windows)]
        acl(path, true)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|_| Error::Invalid("cannot inspect directory"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(Error::Invalid(
                "credential directory permissions must be 0700",
            ));
        }
    }
    #[cfg(windows)]
    acl(path, false)?;
    Ok(())
}
#[cfg(windows)]
fn acl(path: &Path, create: bool) -> Result<(), Error> {
    // Script is constant. The path is an environment value, never shell source.
    let script = if create {
        "$ErrorActionPreference='Stop'; $p=$env:SHAULA_CREDENTIAL_PATH; $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User; $a=Get-Acl -LiteralPath $p; $a.SetAccessRuleProtection($true,$false); foreach($r in @($a.Access)){$a.RemoveAccessRuleSpecific($r)}; $a.SetOwner($sid); $r=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','Allow'); $a.AddAccessRule($r); Set-Acl -LiteralPath $p -AclObject $a"
    } else {
        "$ErrorActionPreference='Stop'; $a=Get-Acl -LiteralPath $env:SHAULA_CREDENTIAL_PATH; $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; if($a.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $sid){exit 1}; foreach($r in $a.Access){if($r.AccessControlType -eq 'Allow' -and $r.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -ne $sid){exit 1}}"
    };
    let root = std::env::var_os("SystemRoot").ok_or(Error::Invalid("SystemRoot missing"))?;
    let status = std::process::Command::new(
        Path::new(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args(["-NoProfile", "-NonInteractive", "-Command", script])
    .env_clear()
    .env("SystemRoot", root)
    .env("SHAULA_CREDENTIAL_PATH", path)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .status()
    .map_err(|_| Error::Invalid("cannot establish private credential ACL"))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(
            "credential ACL must grant access only to the current owner",
        ))
    }
}
pub fn create(path: &Path) -> Result<File, Error> {
    safe_path(path)?;
    let absolute =
        std::path::absolute(path).map_err(|_| Error::Invalid("invalid credential path"))?;
    let parent = absolute
        .parent()
        .ok_or(Error::Invalid("invalid credential directory"))?;
    if !parent.is_dir() {
        return Err(Error::Invalid(
            "secret output requires an existing private directory",
        ));
    }
    // An owner-only parent prevents another user from opening the initially
    // empty Windows file before its explicit file ACL is installed.
    private_directory(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| {
        Error::Invalid("secret output must be a new file in an existing secure directory")
    })?;
    #[cfg(windows)]
    if let Err(e) = acl(path, true) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(file)
}
pub fn write(file: &mut File, secret: &str) -> Result<(), Error> {
    file.write_all(secret.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| Error::Invalid("could not safely save the issued secret"))
}
pub fn read(path: &Path) -> Result<String, Error> {
    safe_path(path)?;
    #[cfg(windows)]
    acl(path, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|_| Error::Invalid("cannot inspect credential file"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(Error::Invalid("credential file permissions must be 0600"));
        }
    }
    let mut value = String::new();
    File::open(path)
        .map_err(|_| Error::Invalid("cannot open credential file"))?
        .take(65_537)
        .read_to_string(&mut value)
        .map_err(|_| Error::Invalid("cannot read credential file"))?;
    Ok(value)
}

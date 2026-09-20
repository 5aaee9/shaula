use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

pub const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const GENERATION: &str = "50a2dd2d-e48e-4b23-88de-b4bca9cf3b90";
pub const HOST: &str = "ssh://runner@host:2222";
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub fn artifact(root: &Path) -> TestResult<shaula_template::artifact::PublishedArtifact> {
    let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/docker");
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for name in [
        "main.tf",
        "profile.yaml",
        ".terraform.lock.hcl",
        "runtime-policy.md",
        "schemas/bindings.schema.json",
        "schemas/parameters.schema.json",
    ] {
        let bytes = std::fs::read(template.join(name))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, name, bytes.as_slice())?;
    }
    let bytes = builder.into_inner()?.finish()?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    Ok(shaula_template::ArtifactStore::new(root.join("artifacts")).publish(&bytes, &digest)?)
}

pub fn responses(root: &Path, image: &str) -> TestResult {
    let short: String = GENERATION.chars().take(24).collect();
    let after = json!({"start":false,"must_run":false,"rm":false,"restart":"no","wait":false,
        "name":format!("shaula-fleet-{short}"),"command":["/home/runner/bin/Runner.Listener","run"],
        "env":["ACTIONS_RUNNER_INPUT_JITCONFIG=frozen-jit"],"image":image,
        "privileged":false,"upload":[],"mounts":[],"volumes":[],"devices":[],"capabilities":[],
        "labels":[{"label":"shaula.fleet","value":"fleet"},{"label":"shaula.generation","value":GENERATION}]});
    let create = json!({"format_version":"1.2","terraform_version":"1.9.8","applyable":true,"complete":true,"errored":false,
        "configuration":{"root_module":{"resources":[]}},
        "resource_changes":[{"address":"docker_container.runner","mode":"managed","type":"docker_container","name":"runner",
            "provider_name":"registry.terraform.io/kreuzwerker/docker","change":{"actions":["create"],"after":after,"after_unknown":{"id":true}}}]});
    let mut destroy = create.clone();
    destroy["resource_changes"][0]["change"] = json!({"actions":["delete"]});
    let state = json!({"version":4,"lineage":"original","serial":1,"resources":[
        {"mode":"managed","type":"docker_container","name":"runner","instances":[{"attributes":{"id":ID}}]}]});
    let empty = json!({"version":4,"lineage":"original","serial":2,"resources":[]});
    let output = json!({"shaula_result":{"sensitive":true,"value":{"contract_version":1,
        "generation_id":GENERATION,"bindings_digest":"commitment","resources":[{"role":"runner","id":ID}]}}});
    let inspect = json!([{"Id":ID,"Name":format!("/shaula-fleet-{short}"),"Image":"image-id","Mounts":[],
        "Config":{"Cmd":["/home/runner/bin/Runner.Listener","run"],"Entrypoint":null,"User":"runner",
            "Env":["ACTIONS_RUNNER_INPUT_JITCONFIG=frozen-jit"],"Labels":{"shaula.fleet":"fleet","shaula.generation":GENERATION}},
        "State":{"Status":"created","Running":false,"Restarting":false,"Dead":false},
        "HostConfig":{"RestartPolicy":{"Name":"no"},"AutoRemove":false,"Privileged":false}}]);
    for (name, value) in [
        ("create", create),
        ("destroy", destroy),
        ("state-create", state),
        ("state-destroy", empty),
        ("output", output),
        ("inspect", inspect),
        ("image", json!([{"Id":"image-id"}])),
    ] {
        let mut file = std::fs::File::create(root.join(format!("{name}.json")))?;
        writeln!(file, "{value}")?;
    }
    Ok(())
}

pub fn script(root: &Path, name: &str, body: &str) -> TestResult<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    let search = std::env::var_os("PATH").ok_or("PATH missing")?;
    let shell = std::env::split_paths(&search)
        .map(|path| path.join("sh"))
        .find(|path| path.is_file())
        .ok_or("shell missing")?
        .canonicalize()?;
    let root = root
        .to_str()
        .ok_or("non UTF-8 path")?
        .replace('\'', "'\"'\"'");
    std::fs::write(&path, format!("#!{}\nset -eu\nROOT='{root}'\nemit() {{ while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; done < \"$1\"; }}\n{body}\n", shell.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

pub const ENGINE: &str = r#"
export SHAULA_TEST_PHASE="$1"
case "$1" in
version) printf 'Terraform v1.9.8\n';;
init) exit 0;;
plan)
  ssh -l runner -p 2222 -- host docker system dial-stdio
  op=create
  for arg in "$@"; do [ "$arg" != '-destroy' ] || op=destroy; done
  printf '%s\n' "$op" > tfplan;;
show) IFS= read -r op < tfplan; emit "$ROOT/$op.json";;
apply)
  ssh -l runner -p 2222 -- host docker system dial-stdio
  IFS= read -r op < tfplan; emit "$ROOT/state-$op.json" > terraform.tfstate;;
output) emit "$ROOT/output.json";;
state)
  case "$2" in
  list) if [ -f terraform.tfstate ]; then printf 'docker_container.runner\n'; fi;;
  pull) emit terraform.tfstate;;
  *) exit 1;; esac;;
*) exit 1;; esac
"#;

pub const DOCKER: &str = r#"
[ "$1" = '--host' ] && [ "$2" = 'ssh://runner@host:2222' ]
shift 2
export SHAULA_TEST_PHASE="bootstrap-$1-$2"
ssh -l runner -p 2222 -- host docker system dial-stdio
case "$1 $2" in
'image inspect') emit "$ROOT/image.json";;
'container inspect') emit "$ROOT/inspect.json";;
'container start') printf 'started\n';;
'cp --') exit 0;;
*) exit 1;; esac
"#;

pub const SSH: &str = r#"
[ "${SSH_AUTH_SOCK-unset}" = unset ]
[ "$SSH_ASKPASS_REQUIRE" = force ]
method=; batch=; identity=; strict=no
while [ "$#" -gt 0 ]; do
  case "$1" in
  -F) [ -f "$2" ]; shift;;
  -i) identity="$2"; shift;;
  -o) case "$2" in
    PreferredAuthentications=*) method="${2#*=}";;
    BatchMode=*) batch="${2#*=}";;
    StrictHostKeyChecking=yes) strict=yes;;
    esac; shift;;
  esac
  shift
done
[ "$strict" = yes ]
if [ "$method" = publickey ]; then
  IFS= read -r key < "$identity"
  [ "$key" = private-key ];
fi
if [ "$batch" = no ]; then
  credential="$("$SSH_ASKPASS" "authentication prompt")"
  case "$credential" in password-secret|passphrase-secret) :;; *) exit 1;; esac
fi
printf '%s|%s\n' "$SHAULA_TEST_PHASE" "$SHAULA_DOCKER_SSH_CONFIG" >> "$ROOT/calls"
"#;

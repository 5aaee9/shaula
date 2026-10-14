//! The real Terraform artifact a fixture Generation is created from.
use super::fixture::TestResult;
use shaula_core::template::ManagedResourceRole;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Plain,
    /// The runner provisioner leaves a process outside the worker.
    Orphan,
    /// The runner is created, then its provisioner fails: Terraform records
    /// it tainted, so the failed state holds the whole managed shape.
    FailsTainted,
    /// Two runners are declared; the second fails at apply on an input unknown
    /// at plan (a check would be refused), so the failed state holds only part
    /// of the shape.
    FailsPartial,
}

impl Template {
    pub fn shape(self) -> Vec<ManagedResourceRole> {
        vec![ManagedResourceRole {
            role: "runner".into(),
            terraform_type: "terraform_data".into(),
            exact_count: if self == Self::FailsPartial { 2 } else { 1 },
        }]
    }

    fn runner(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Orphan => {
                r#"provisioner "local-exec" {
    command = "setsid sh -c 'echo $$ > orphan.pid; exec sleep 300' </dev/null >/dev/null 2>&1 &"
  }"#
            }
            Self::FailsTainted => r#"provisioner "local-exec" { command = "exit 1" }"#,
            Self::FailsPartial => "",
        }
    }

    fn profile(self) -> String {
        format!(
            r#"api_version: shaula.io/template-profile/v1
kind: RunnerTemplateProfile
platform: aws
runtime:
  protocol: terraform-cli/v1
  engine: terraform
  root_module: .
  required_version: ">= 1.9, < 2.0"
bindings_contract: shaula.bindings.aws/v1
vm_image_contract: shaula.aws-ami/v1
schemas:
  bindings: schemas/bindings.schema.json
  parameters: schemas/parameters.schema.json
managed_resource_shape:
  - role: runner
    terraform_type: terraform_data
    exact_count: {}
runner_image_digests: []
runtime_policy_digest: sha256:fixture
"#,
            self.shape()[0].exact_count
        )
    }

    fn main(self) -> String {
        let second = if self == Self::FailsPartial {
            r#"resource "terraform_data" "second" { input = tonumber(terraform_data.runner.id) }
"#
        } else {
            ""
        };
        format!(
            r#"terraform {{ required_version = "= 1.9.8" }}
variable "shaula" {{ type = any }}
resource "terraform_data" "runner" {{
  input = var.shaula.generation.id
  {}
}}
{second}output "shaula_result" {{
  value = {{
    contract_version = 1
    generation_id = var.shaula.generation.id
    bindings_digest = var.shaula.bindings_digest
    resources = [{{ role = "runner", id = terraform_data.runner.id }}]
  }}
}}
"#,
            self.runner()
        )
    }

    /// Writes the artifact under `path`, packs it beside it and returns
    /// `(digest, manifest)`; the manifest is what activation would store.
    pub fn write(self, path: &std::path::Path) -> TestResult<(String, String)> {
        let profile = self.profile();
        std::fs::write(path.join("profile.yaml"), &profile)?;
        std::fs::write(path.join(".terraform.lock.hcl"), "")?;
        std::fs::write(path.join("main.tf"), self.main())?;
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for name in ["profile.yaml", ".terraform.lock.hcl", "main.tf"] {
            archive.append_path_with_name(path.join(name), name)?;
        }
        let bytes = archive.into_inner()?.finish()?;
        std::fs::write(path.with_extension("tar.gz"), &bytes)?;
        use sha2::{Digest, Sha256};
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
        Ok((digest, profile))
    }
}

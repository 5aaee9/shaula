//! Backend capabilities belong to the artifact; selection belongs to protected
//! publisher bindings, frozen on each Template Revision (never Fleet inputs).

use serde_json::{Map, Value};

use super::{container::official_runner_image, CoreError, CoreResult, ProfileManifest, ReasonCode};
use crate::fleet::FleetProviderKind;

fn invalid(message: &str) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, message)
}

impl ProfileManifest {
    pub fn supports_runner_backend(&self, backend: &str) -> bool {
        if self.runner_backends.is_empty() {
            self.runner_backend == backend
        } else {
            self.runner_backends.iter().any(|item| item == backend)
        }
    }

    pub(super) fn validate_runner_backends(&self) -> CoreResult<()> {
        if !matches!(self.runner_backend.as_str(), "github" | "forgejo") {
            return Err(invalid("runner_backend must be github or forgejo"));
        }
        if self.runner_backends.is_empty() {
            return Ok(());
        }
        let container = matches!(self.platform.as_str(), "docker" | "kubernetes")
            && self.container_bootstrap_contract.as_deref()
                == Some(super::CONTAINER_BOOTSTRAP_CONTRACT);
        let vm = self.forgejo_vm_bootstrap_contract.as_deref()
            == Some(super::FORGEJO_VM_BOOTSTRAP_CONTRACT);
        if (!container && !vm)
            || self.runner_backends.len() != 2
            || !self.runner_backends.iter().any(|b| b == "github")
            || !self.runner_backends.iter().any(|b| b == "forgejo")
            || self.runner_backend != "github"
        {
            return Err(invalid("selectable runner_backends requires github and forgejo, a github default and an admitted bootstrap contract"));
        }
        if !container {
            return Ok(());
        }
        for backend in &self.runner_backends {
            if !self
                .runner_image_digests
                .iter()
                .any(|image| official_runner_image(backend, image))
            {
                return Err(invalid(
                    "each selectable runner backend requires an official pinned image",
                ));
            }
        }
        Ok(())
    }

    /// Historical single-backend manifests ignore the new binding. Only an
    /// explicit capability declaration opts a source into publisher selection.
    pub fn runner_backend_for_bindings<'a>(
        &'a self,
        bindings: &'a Map<String, Value>,
    ) -> CoreResult<&'a str> {
        let selected = if self.runner_backends.is_empty() {
            self.runner_backend.as_str()
        } else {
            match bindings.get("runner_backend") {
                Some(Value::String(backend)) => backend,
                None => self.runner_backend.as_str(),
                _ => return Err(invalid("runner_backend binding must be github or forgejo")),
            }
        };
        if !self.supports_runner_backend(selected) {
            return Err(invalid(
                "runner_backend binding is not supported by this template",
            ));
        }
        Ok(selected)
    }

    /// Checks capability only. Destroy keeps the original Revision/material;
    /// admission and Create additionally verify its immutable binding selection.
    pub fn validate_for_provider(&self, provider: FleetProviderKind) -> CoreResult<()> {
        self.validate()?;
        if !self.supports_runner_backend(backend_name(provider)) {
            return Err(invalid(
                "template does not support the Fleet runner backend",
            ));
        }
        Ok(())
    }

    pub fn validate_bindings_for_provider(
        &self,
        bindings: &Map<String, Value>,
        provider: FleetProviderKind,
    ) -> CoreResult<()> {
        self.validate_for_provider(provider)?;
        if self.runner_backend_for_bindings(bindings)? != backend_name(provider) {
            return Err(invalid(
                "Template Revision runner_backend does not match the Fleet provider",
            ));
        }
        Ok(())
    }

    /// `auto` picks the sole official image for the selected backend. Explicit
    /// aliases remain finite and cannot cross the publisher's backend choice.
    pub fn selected_runner_image(
        &self,
        bindings: &Map<String, Value>,
        parameters: &Map<String, Value>,
    ) -> CoreResult<String> {
        let backend = self.runner_backend_for_bindings(bindings)?;
        let alias = match parameters.get("runner_image") {
            None => None,
            Some(Value::String(alias)) if alias == "auto" => None,
            Some(Value::String(alias)) => Some(alias.as_str()),
            _ => return Err(invalid("runner_image must be an admitted alias or auto")),
        };
        let mut images = self.runner_image_digests.iter().filter(|image| {
            official_runner_image(backend, image)
                && alias.is_none_or(|alias| image.split('@').next() == Some(alias))
        });
        let image = images
            .next()
            .ok_or_else(|| invalid("runner image does not match the selected backend"))?;
        if images.next().is_some() {
            return Err(invalid(
                "runner image selection is ambiguous; select an exact alias",
            ));
        }
        Ok(image.clone())
    }
}

#[cfg(test)]
#[path = "template_backend_tests.rs"]
mod tests;

fn backend_name(provider: FleetProviderKind) -> &'static str {
    match provider {
        FleetProviderKind::Github => "github",
        FleetProviderKind::Forgejo => "forgejo",
    }
}

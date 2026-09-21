# Shaula bundled Kubernetes Runner Template Profile.
#
# One Runner Generation = one bootstrap Secret + one Pod
# (restartPolicy Never, automountServiceAccountToken false), both in the
# pre-provisioned namespace selected by the publisher-owned binding.
# The namespace is never created, imported or managed by this Profile.
#
# The publisher selects GitHub (default) or Forgejo on this same template.
# Shaula releases the required Secret key gate after apply: GitHub Setup Info
# or a Forgejo token file. Tokens never appear in Forgejo argv or environment.

terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "2.33.0"
    }
  }
}

provider "kubernetes" {
  config_path = var.shaula.bindings.kubeconfig
}

# Ordinary provider data source verifies the pre-created namespace exists.
# The blocking lifecycle.precondition fails Create planning when the
# namespace is missing; no namespace is ever created or defaulted.
# A standalone `check` block is advisory and is deliberately not the gate.
data "kubernetes_namespace" "target" {
  metadata {
    name = var.shaula.bindings.namespace
  }
}

locals {
  forgejo       = var.shaula.bindings.runner_backend == "forgejo"
  image_prefix  = local.forgejo ? "code.forgejo.org/forgejo/runner:" : "ghcr.io/actions/actions-runner:"
  runner_images = { for image in yamldecode(file("${path.module}/profile.yaml")).runner_image_digests : split("@", image)[0] => image if startswith(image, local.image_prefix) }
  runner_image  = var.shaula.parameters.runner_image == "auto" ? one(values(local.runner_images)) : local.runner_images[var.shaula.parameters.runner_image]
  forgejo_args = local.forgejo ? concat(
    ["one-job", "--url", var.shaula.forgejo.instance_url, "--uuid", var.shaula.forgejo.uuid, "--token-url", "file:///data/.forgejo-token"],
    flatten([for label in var.shaula.forgejo.labels : ["--label", label]]),
    ["--wait"]
  ) : []
  bootstrap_key = local.forgejo ? "token" : ".setup_info"
  # Exact generation-scoped metadata.name persisted by Shaula before any
  # external effect; the Secret and the Pod share it, kind separates the
  # Resource Keys. Not a Kubernetes UID; never reconstructed after state
  # loss. The short display runner_name must never be used here.
  generation_name = var.shaula.generation.generation_name
}

resource "kubernetes_secret_v1" "bootstrap" {
  metadata {
    name      = local.generation_name
    namespace = var.shaula.bindings.namespace

    labels = {
      "shaula.io/fleet"         = var.shaula.generation.fleet_key
      "shaula.io/generation-id" = var.shaula.generation.id
    }
  }

  type = "Opaque"

  # A nonsecret marker keeps data known in Kubernetes 2.33.0 plans; the
  # required token key stays absent until the host-owned bootstrap patch.
  data = local.forgejo ? { runner_backend = "forgejo" } : {
    # GitHub Listener input; Forgejo's token never enters Terraform inputs.
    "jit_config" = var.shaula.jit_config
  }

  # Deliberately omit the bootstrap key (.setup_info or token): its required
  # volume prevents startup until Shaula finishes apply and adds that key
  # together with immutable=true in a single identity-checked API patch.
  immutable = false

  lifecycle {
    precondition {
      condition     = data.kubernetes_namespace.target.metadata[0].name == var.shaula.bindings.namespace
      error_message = "The configured namespace must exist and match exactly before the bootstrap Secret can be planned."
    }
  }
}

resource "kubernetes_pod_v1" "runner" {
  # Provider 2.33.0 normally waits for Running. Pending is the successful
  # provisioning state here because bootstrap is released only after apply.
  target_state = ["Pending"]

  metadata {
    name      = local.generation_name
    namespace = var.shaula.bindings.namespace

    labels = {
      "shaula.io/fleet"         = var.shaula.generation.fleet_key
      "shaula.io/generation-id" = var.shaula.generation.id
    }
  }

  spec {
    # No API credential reaches the Runner; Kubernetes may still surface
    # serviceAccountName "default", the invariant is the absent token.
    automount_service_account_token = false

    restart_policy = "Never"

    container {
      name  = "runner"
      image = local.runner_image

      command = local.forgejo ? ["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"] : ["/home/runner/bin/Runner.Listener", "run"]
      args    = local.forgejo_args

      dynamic "env" {
        for_each = local.forgejo ? [] : [1]
        content {
          name = "ACTIONS_RUNNER_INPUT_JITCONFIG"
          value_from {
            secret_key_ref {
              name     = kubernetes_secret_v1.bootstrap.metadata[0].name
              key      = "jit_config"
              optional = false
            }
          }
        }
      }

      dynamic "security_context" {
        for_each = local.forgejo ? [1] : []
        content {
          run_as_non_root            = true
          run_as_user                = 1000
          allow_privilege_escalation = false
          capabilities {
            drop = ["ALL"]
          }
        }
      }

      resources {
        requests = {
          cpu    = var.shaula.parameters.cpu_request
          memory = var.shaula.parameters.memory_request
        }
      }

      # Mount only the backend's required bootstrap file. The subPath is
      # fixed before start and Shaula freezes the Secret when releasing it.
      volume_mount {
        name       = "setup-info"
        mount_path = local.forgejo ? "/data/.forgejo-token" : "/home/runner/.setup_info"
        sub_path   = local.bootstrap_key
        read_only  = true
      }
    }

    volume {
      name = "setup-info"
      secret {
        secret_name = kubernetes_secret_v1.bootstrap.metadata[0].name
        optional    = false
        items {
          key  = local.bootstrap_key
          path = local.bootstrap_key
        }
      }
    }
  }

  # Terraform owns only Create and Destroy. The declared bootstrap contract
  # gives Shaula the one-time, identity-checked Secret patch described above.
  depends_on = [kubernetes_secret_v1.bootstrap]
}

# Fixed provider-neutral output envelope; Shaula validates contract
# version, echoed generation id, bindings_digest and role cardinality, then
# stores the body as protected opaque evidence.
output "shaula_result" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    bindings_digest  = var.shaula.bindings_digest
    resources = [
      {
        role        = "bootstrap"
        id          = "${kubernetes_secret_v1.bootstrap.metadata[0].namespace}/${kubernetes_secret_v1.bootstrap.metadata[0].name}"
        incarnation = kubernetes_secret_v1.bootstrap.metadata[0].uid
      },
      {
        role        = "runner"
        id          = "${kubernetes_pod_v1.runner.metadata[0].namespace}/${kubernetes_pod_v1.runner.metadata[0].name}"
        incarnation = kubernetes_pod_v1.runner.metadata[0].uid
      }
    ]
  }
  sensitive = true
}

variable "shaula" {
  description = "Fixed Shaula system input envelope; the whole variable is sensitive and the Profile cannot rename or extend it."
  type = object({
    contract_version = number
    generation       = any
    jit_config       = string
    forgejo          = optional(any)
    bindings_digest  = string
    bindings = object({
      runner_backend = optional(string, "github")
      namespace      = string
      kubeconfig     = string
    })
    parameters = object({
      runner_image   = optional(string, "auto")
      cpu_request    = optional(string, "500m")
      memory_request = optional(string, "2Gi")
    })
  })
  sensitive = true
}

# Shaula bundled Kubernetes Runner Template Profile.
#
# One Runner Generation = one bootstrap Secret + one Pod
# (restartPolicy Never, automountServiceAccountToken false), both in the
# pre-provisioned namespace selected by the publisher-owned binding.
# The namespace is never created, imported or managed by this Profile.
#
# The publisher selects GitHub (default), Forgejo or GitLab on this same
# template. Shaula releases the required Secret key gate after apply: GitHub
# Setup Info, a Forgejo token file or a GitLab config.toml. Pool tokens never
# appear in argv or environment.

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
  backend       = var.shaula.bindings.runner_backend
  forgejo       = local.backend == "forgejo"
  gitlab        = local.backend == "gitlab"
  image_prefix  = { github = "ghcr.io/actions/actions-runner:", forgejo = "code.forgejo.org/forgejo/runner:", gitlab = "gitlab/gitlab-runner:" }[local.backend]
  runner_images = { for image in yamldecode(file("${path.module}/profile.yaml")).runner_image_digests : split("@", image)[0] => image if startswith(image, local.image_prefix) }
  runner_image  = var.shaula.parameters.runner_image == "auto" ? one(values(local.runner_images)) : local.runner_images[var.shaula.parameters.runner_image]
  forgejo_args = local.forgejo ? concat(
    ["one-job", "--url", var.shaula.forgejo.instance_url, "--uuid", var.shaula.forgejo.uuid, "--token-url", "file:///data/.forgejo-token"],
    flatten([for label in var.shaula.forgejo.labels : ["--label", label]]),
    ["--wait"]
  ) : []
  # The registration token is never a Terraform input: Shaula patches the
  # protected config.toml into the Secret. 600 s equals the Pool idle
  # deadline (RUNNER_WAIT_TIMEOUT_SECS).
  gitlab_args = local.gitlab ? [
    "run-single", "--config", "/home/gitlab-runner/config.toml",
    "--runner", var.shaula.gitlab.runner_name, "--max-builds", "1", "--wait-timeout", "600"
  ] : []
  command = {
    github  = ["/home/runner/bin/Runner.Listener", "run"]
    forgejo = ["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"]
    gitlab  = ["/usr/bin/dumb-init", "--", "/usr/bin/gitlab-runner"]
  }[local.backend]
  bootstrap_key  = { github = ".setup_info", forgejo = "token", gitlab = "config.toml" }[local.backend]
  bootstrap_path = { github = "/home/runner/.setup_info", forgejo = "/data/.forgejo-token", gitlab = "/home/gitlab-runner/config.toml" }[local.backend]
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
  # required token or config.toml key stays absent until the host-owned
  # bootstrap patch.
  data = local.forgejo || local.gitlab ? { runner_backend = local.backend } : {
    # GitHub Listener input; Pool tokens never enter Terraform inputs.
    "jit_config" = var.shaula.jit_config
  }

  # Deliberately omit the bootstrap key (.setup_info, token or config.toml):
  # its required volume prevents startup until Shaula finishes apply and adds
  # that key together with immutable=true in a single identity-checked patch.
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

    # A restarted gitlab-runner would claim a second job with the same token.
    restart_policy = "Never"

    container {
      name    = "runner"
      image   = local.runner_image
      command = local.command
      args    = local.gitlab ? local.gitlab_args : local.forgejo_args
      # The official GitLab image declares no working directory; its shell
      # executor builds below this Generation-owned emptyDir instead of /.
      working_dir = local.gitlab ? "/home/gitlab-runner" : null

      dynamic "env" {
        for_each = local.forgejo || local.gitlab ? [] : [1]
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
        for_each = local.forgejo || local.gitlab ? [1] : []
        content {
          run_as_non_root = true
          # The GitLab image defaults to root; 999 is its packaged
          # gitlab-runner user. The Forgejo image's runner user is 1000.
          run_as_user                = local.gitlab ? 999 : 1000
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
        mount_path = local.bootstrap_path
        sub_path   = local.bootstrap_key
        read_only  = true
      }

      # gitlab-runner writes .runner_system_id next to its config and builds
      # below its working directory. Both live in this writable, Generation-
      # scoped emptyDir; the read-only config.toml file mount sits inside it.
      dynamic "volume_mount" {
        for_each = local.gitlab ? [1] : []
        content {
          name       = "home"
          mount_path = "/home/gitlab-runner"
        }
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

    dynamic "volume" {
      for_each = local.gitlab ? [1] : []
      content {
        name = "home"
        empty_dir {}
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
    gitlab           = optional(any)
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

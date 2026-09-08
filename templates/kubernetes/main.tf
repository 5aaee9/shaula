# Shaula bundled Kubernetes Runner Template Profile.
#
# One Runner Generation = one immutable Secret (JIT bootstrap) + one Pod
# (restartPolicy Never, automountServiceAccountToken false), both in the
# pre-provisioned namespace selected by the publisher-owned binding.
# The namespace is never created, imported or managed by this Profile.
#
# IMAGE REQUIREMENT (staged gate, spec 0003 §5 / ARD-0004): the runner
# image MUST be built with the reviewed bootstrap-shim installed at
# /usr/local/bin/bootstrap-shim (reads the staged JIT once, unlinks the
# staged copy, then execs the Actions listener). The stock
# actions-runner image does NOT contain it; conformance images are
# delivered with the phase-3 conformance harness.

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
  runner_images = { for image in yamldecode(file("${path.module}/profile.yaml")).runner_image_digests : split("@", image)[0] => image }
  runner_image  = local.runner_images[var.shaula.parameters.runner_image]
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

  data = {
    # One-time JIT bootstrap payload; consumed only through the Secret
    # volume mounted read-only by the init container.
    "jit_config" = var.shaula.jit_config
  }

  immutable = true

  lifecycle {
    precondition {
      condition     = data.kubernetes_namespace.target.metadata[0].name == var.shaula.bindings.namespace
      error_message = "The configured namespace must exist and match exactly before the bootstrap Secret can be planned."
    }
  }
}

resource "kubernetes_pod_v1" "runner" {
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

    # Stage 1: init container copies the JIT file from the read-only Secret
    # volume into the generation-scoped memory volume.
    init_container {
      name  = "jit-stage"
      image = local.runner_image

      // Stage the JIT payload AND the pinned bootstrap shim into the
      // memory volume. The shim is copied from the pinned image so the
      // runner’s mount of the memory volume cannot shadow it away
      // (spec 0003 section 5: reviewed shim, read-once JIT, unlink).
      command = [
        "/bin/sh",
        "-c",
        "cp /in/jit_config /stage/jit_config && chmod 400 /stage/jit_config && cp /usr/local/bin/bootstrap-shim /stage/bootstrap-shim && chmod 500 /stage/bootstrap-shim",
      ]

      volume_mount {
        name       = "jit-source"
        mount_path = "/in"
        read_only  = true
      }

      volume_mount {
        name       = "jit-stage-volume"
        mount_path = "/stage"
      }
    }

    container {
      name  = "runner"
      image = local.runner_image

      # The pinned shim reads and unlinks the staged JIT file, sets only
      # ACTIONS_RUNNER_INPUT_JITCONFIG for Runner.Listener, and never puts
      # the secret in argv.
      // Executed from the memory volume (staged by init), not from a
      // shadowed image path.
      command = ["/shaula/bootstrap-shim"]

      resources {
        requests = {
          cpu    = var.shaula.parameters.cpu_request
          memory = var.shaula.parameters.memory_request
        }
      }

      # Only the memory volume is mounted; the Secret volume itself is
      # never mounted into the runner container.
      volume_mount {
        name       = "jit-stage-volume"
        mount_path = "/shaula"
      }
    }

    volume {
      name = "jit-source"
      secret {
        secret_name = kubernetes_secret_v1.bootstrap.metadata[0].name
      }
    }

    volume {
      name = "jit-stage-volume"
      empty_dir {
        medium = "Memory"
      }
    }
  }

  lifecycle {
    precondition {
      condition     = kubernetes_secret_v1.bootstrap.immutable
      error_message = "The bootstrap Secret must be immutable before the Pod is planned."
    }
  }

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
        incarnation = kubernetes_secret_v1.bootstrap.metadata[0].resource_version
      },
      {
        role        = "runner"
        id          = "${kubernetes_pod_v1.runner.metadata[0].namespace}/${kubernetes_pod_v1.runner.metadata[0].name}"
        incarnation = kubernetes_pod_v1.runner.metadata[0].resource_version
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
    bindings_digest  = string
    bindings = object({
      namespace  = string
      kubeconfig = string
    })
    parameters = object({
      runner_image   = optional(string, "ghcr.io/actions/actions-runner:2.323.0")
      cpu_request    = optional(string, "500m")
      memory_request = optional(string, "2Gi")
    })
  })
  sensitive = true
}

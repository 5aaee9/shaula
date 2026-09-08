# Shaula bundled Docker Runner Template Profile.
#
# One Runner Generation = one generation-scoped docker_container. The image
# is pre-pulled shared data (not owned by the state), the container never
# restarts or auto-removes, and the default Profile never mounts
# /var/run/docker.sock into the Runner.
#
# Build the runner image with image/Dockerfile and pin its final digest in
# profile.yaml. The stock actions-runner image does not contain the shim.
# Image construction alone does not satisfy the activation attestation gate.

terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    docker = {
      source  = "kreuzwerker/docker"
      version = "3.0.2"
    }
  }
}

provider "docker" {
  # Trusted Profile binding; never a Fleet input. Plaintext TCP is
  # rejected by the bindings schema.
  host = var.shaula.bindings.docker_host
}

locals {
  runner_images = { for image in yamldecode(file("${path.module}/profile.yaml")).runner_image_digests : split("@", image)[0] => image }
  runner_image  = local.runner_images[var.shaula.parameters.runner_image]
  # Deterministic container name derived from the already-persisted
  # Generation identity; never randomized per attempt.
  container_name = "shaula-${var.shaula.generation.fleet_key}-${substr(var.shaula.generation.id, 0, 24)}"
}

# Docker image resolved from a finite alias to a pinned digest. The image
# is pre-pulled or treated as shared read-only data, not a managed object.
data "docker_image" "runner" {
  name = local.runner_image
}

resource "docker_container" "runner" {
  name    = local.container_name
  image   = data.docker_image.runner.id
  runtime = "runc"

  # One ephemeral job: no restart, no daemon-side removal behind the
  # lifecycle ledger, no persistence claim.
  restart  = "no"
  must_run = false
  rm       = false
  memory   = 4096
  cpu_set  = "0-1"

  labels {
    label = "shaula.fleet"
    value = var.shaula.generation.fleet_key
  }
  labels {
    label = "shaula.generation"
    value = var.shaula.generation.id
  }

  # JIT handoff: the provider uploads the protected value to a fixed file
  # before container start (never a host source path, never argv). The
  # pinned shim reads and unlinks it, then spawns Runner.Listener with only
  # ACTIONS_RUNNER_INPUT_JITCONFIG set.
  upload {
    # Provider 3.0.2 uploads a root-owned 0644 file. The image's runner-owned
    # 0700 directory restricts access and lets runner read and unlink it.
    file    = "/shaula/jit_config"
    content = var.shaula.jit_config
  }

  // Pinned shim (bundled in the pinned runner image) performs the
  // read-once / unlink / env-only handoff.
  command = ["/usr/local/bin/bootstrap-shim"]

  # JIT arrives through the upload above, not through declarative env.
  env = []

  lifecycle {
    precondition {
      condition     = !startswith(var.shaula.bindings.docker_host, "tcp://")
      error_message = "Plaintext TCP Docker endpoints are rejected by the v1 contract."
    }
  }
}

# Fixed provider-neutral output envelope validated and stored opaquely by
# the core.
output "shaula_result" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    bindings_digest  = var.shaula.bindings_digest
    resources = [
      {
        role = "runner"
        id   = docker_container.runner.id
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
      docker_host   = optional(string, "unix:///var/run/docker.sock")
      registry_auth = optional(string)
    })
    parameters = object({
      runner_image = optional(string, "localhost:5001/shaula-runner:2.337.0-bootstrap-v1")
    })
  })
  sensitive = true
}

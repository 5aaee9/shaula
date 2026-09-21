# Shaula bundled Docker Runner Template Profile.
#
# One Runner Generation = one generation-scoped docker_container. The image
# is pre-pulled shared data (not owned by the state), the container never
# restarts or auto-removes, and the default Profile never mounts
# /var/run/docker.sock into the Runner.
#
# The publisher selects GitHub (default) or Forgejo on this same template.
# Shaula bootstraps the stopped official container after apply: GitHub Setup
# Info or the Forgejo token file, then starts the corresponding one-job runner.

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
  # Trusted Profile binding; never a Fleet input. Shaula supplies the same
  # invocation-scoped SSH authentication to this provider and Docker bootstrap.
  # Passwords and private keys are never passed as provider SSH options.
  host = var.shaula.bindings.docker_host
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
  start    = false
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

  # GitHub's Listener captures and removes this supported input variable
  # before workflow execution. Docker metadata and Terraform state remain
  # credential-grade; no JIT value is placed in argv or a shell script.
  command        = local.forgejo ? concat(["/bin/forgejo-runner"], local.forgejo_args) : ["/home/runner/bin/Runner.Listener", "run"]
  entrypoint     = local.forgejo ? ["/usr/bin/dumb-init"] : null
  working_dir    = local.forgejo ? "/data" : null
  remove_volumes = true
  # A fixed nonsecret marker avoids Docker 3.0.2 treating an empty env set
  # as computed/unknown in the saved plan. Unknown launch controls fail closed.
  env = local.forgejo ? ["SHAULA_RUNNER_BACKEND=forgejo"] : ["ACTIONS_RUNNER_INPUT_JITCONFIG=${var.shaula.jit_config}"]
  # Forgejo's token is copied into the official image's anonymous /data
  # volume by Shaula, never Terraform. Destroy removes that volume too.

  # Do not start during apply: its completed, sanitized log projection is
  # prepared outside the container before Shaula releases this start gate.
  # Terraform remains the sole container create/destroy owner.

  lifecycle {
    precondition {
      condition     = can(regex("^(unix|npipe|ssh)://", var.shaula.bindings.docker_host))
      error_message = "Docker endpoints must use a local socket or authenticated SSH; plaintext TCP is rejected."
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
    forgejo          = optional(any)
    bindings_digest  = string
    bindings = object({
      runner_backend             = optional(string, "github")
      docker_host                = optional(string, "unix:///var/run/docker.sock")
      registry_auth              = optional(string)
      ssh_password               = optional(string)
      ssh_private_key            = optional(string)
      ssh_private_key_passphrase = optional(string)
      ssh_known_hosts            = optional(string)
    })
    parameters = object({
      runner_image = optional(string, "auto")
    })
  })
  sensitive = true
}

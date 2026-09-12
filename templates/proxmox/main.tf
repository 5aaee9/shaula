# One Runner Generation owns one clone and one dedicated NoCloud ISO.
# The selected template VM, storage, network and node are shared publisher data.
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    proxmox = {
      source  = "indexyz/proxmox"
      version = "0.5.0"
    }
  }
}

provider "proxmox" {
  endpoint         = var.shaula.bindings.proxmox_host
  api_token_id     = split("=", var.shaula.bindings.proxmox_token)[0]
  api_token_secret = join("=", slice(split("=", var.shaula.bindings.proxmox_token), 1, length(split("=", var.shaula.bindings.proxmox_token))))
  insecure         = var.shaula.bindings.proxmox_insecure
}

data "proxmox_qemu_vms" "template" {
  name     = var.shaula.bindings.proxmox_template_name
  template = true
}

locals {
  # A normal destroy refresh still validates required configuration fields.
  # Keep them syntactically valid if the source template disappeared; the
  # exact-one precondition prevents these sentinels from creating anything,
  # while provider Read/Delete continue using the original bound state.
  source          = try(one(data.proxmox_qemu_vms.template.vms), null)
  source_node     = try(local.source.node, "shaula-source-unavailable")
  source_vm_id    = try(local.source.vm_id, 100)
  generation_name = var.shaula.generation.generation_name
}

resource "proxmox_nocloud_iso" "bootstrap" {
  node     = local.source_node
  storage  = var.shaula.bindings.proxmox_iso_storage
  filename = "${local.generation_name}.iso"

  meta_data = yamlencode({
    instance-id    = local.generation_name
    local-hostname = local.generation_name
  })
  user_data = templatefile("${path.module}/user-data.tftpl", {
    jit_config = var.shaula.jit_config
    pre_start  = var.shaula.bindings.proxmox_cloud_init_cmd
    bootstrap  = file("${path.module}/bootstrap.tftpl")
    service    = file("${path.module}/runner-service.tftpl")
  })
  # A clean Linux template supplies one Ethernet NIC (eth* or en*). Its
  # bridge/MAC/model are inherited. No Proxmox ipconfig or Fleet override.
  network_config = yamlencode({
    version = 2
    ethernets = {
      runner = {
        match = { name = "e*" }
        dhcp4 = true
      }
    }
  })

  lifecycle {
    create_before_destroy = true
    precondition {
      condition     = length(data.proxmox_qemu_vms.template.vms) == 1
      error_message = "The selected name must identify exactly one visible Proxmox template VM."
    }
  }
}

resource "proxmox_qemu_vm" "runner" {
  node        = local.source_node
  name        = local.generation_name
  vm_id_start = var.shaula.bindings.proxmox_vmid_begin
  clone = {
    source_node = local.source_node
    source_vmid = local.source_vm_id
    full        = var.shaula.bindings.proxmox_full_clone
  }

  # Keep inherited root disks and NICs outside the managed map. CPU and memory
  # are Fleet parameters so each Fleet can choose an approved VM size.
  # The provider refuses a foreign disk/ISO or a second inherited seed.
  cores             = var.shaula.parameters.cpu_cores
  memory            = var.shaula.parameters.memory_mb
  nocloud_cdrom_slot = "ide2"
  disk = {
    ide2 = {
      media  = "cdrom"
      volume = proxmox_nocloud_iso.bootstrap.volume_id
    }
  }
  onboot          = false
  protection      = false
  start_on_create = true
  stop_on_destroy = true

  lifecycle {
    replace_triggered_by = [proxmox_nocloud_iso.bootstrap]
  }
}

output "shaula_result" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    bindings_digest  = var.shaula.bindings_digest
    resources = [
      { role = "bootstrap", id = proxmox_nocloud_iso.bootstrap.id },
      { role = "runner", id = proxmox_qemu_vm.runner.id }
    ]
  }
  sensitive = true
}

variable "shaula" {
  description = "Fixed Shaula system input envelope; publisher bindings never become Fleet inputs."
  type = object({
    contract_version = number
    generation       = any
    jit_config       = string
    bindings_digest  = string
    bindings = object({
      proxmox_host           = string
      proxmox_token          = string
      proxmox_insecure       = optional(bool, true)
      proxmox_template_name  = optional(string, "GitHub-Runner")
      proxmox_vmid_begin     = optional(number, 100)
      proxmox_iso_storage    = optional(string, "local")
      proxmox_full_clone     = optional(bool, false)
      proxmox_cloud_init_cmd = optional(string, "")
    })
    parameters = object({
      cpu_cores = optional(number, 2)
      memory_mb = optional(number, 4096)
    })
  })
  sensitive = true

  validation {
    condition     = can(regex("^https://[^/?#@\\s]+/?$", var.shaula.bindings.proxmox_host))
    error_message = "proxmox_host must be an HTTPS API origin without credentials, path, query or fragment."
  }
  validation {
    condition     = can(regex("^[^\\s!=@]+@[^\\s!=@]+![^\\s!=]+=[^\\s]+$", var.shaula.bindings.proxmox_token))
    error_message = "proxmox_token must use user@realm!tokenid=secret format."
  }
  validation {
    condition     = var.shaula.bindings.proxmox_vmid_begin >= 100 && var.shaula.bindings.proxmox_vmid_begin <= 999999999 && floor(var.shaula.bindings.proxmox_vmid_begin) == var.shaula.bindings.proxmox_vmid_begin
    error_message = "proxmox_vmid_begin must be an integer from 100 through 999999999."
  }
  validation {
    condition     = length(trimspace(var.shaula.bindings.proxmox_template_name)) > 0
    error_message = "proxmox_template_name must not be blank."
  }
}

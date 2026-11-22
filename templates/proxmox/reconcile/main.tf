# Read-only failed-Create discovery module (ARD-0044).
# Lists remote Proxmox ISO and QEMU VM objects owned by this Generation without
# mutating infrastructure or Terraform state.
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    proxmox = {
      source  = "indexyz/proxmox"
      version = "0.7.1"
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
  source          = one(data.proxmox_qemu_vms.template.vms)
  source_node     = local.source.node
  generation_name = var.shaula.generation.generation_name
  iso_volid       = "${var.shaula.bindings.proxmox_iso_storage}:iso/${local.generation_name}.iso"
}

data "proxmox_storage_contents" "iso" {
  node    = local.source_node
  storage = var.shaula.bindings.proxmox_iso_storage
  content = "iso"
}

data "proxmox_qemu_vms" "runner" {
  name     = local.generation_name
  template = false
}

output "shaula_discover" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    roles = [
      {
        role = "bootstrap"
        resources = [
          for v in data.proxmox_storage_contents.iso.volumes : {
            address   = "proxmox_nocloud_iso.bootstrap"
            import_id = null
          }
          if v.volid == local.iso_volid
        ]
      },
      {
        role = "runner"
        resources = [
          for vm in data.proxmox_qemu_vms.runner.vms : {
            address   = "proxmox_qemu_vm.runner"
            import_id = "${vm.node}/${vm.vm_id}"
          }
        ]
      }
    ]
  }
  sensitive = true
}

variable "shaula" {
  type      = any
  sensitive = true
}

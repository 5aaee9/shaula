# Each Generation owns one ECS instance. Images, vSwitches and security groups
# are publisher-selected shared resources, never managed here.
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    alicloud = {
      source  = "aliyun/alicloud"
      version = "1.292.0"
    }
  }
}

provider "alicloud" {
  region     = var.shaula.bindings.alicloud_region
  access_key = var.shaula.bindings.alicloud_access_key_id
  secret_key = var.shaula.bindings.alicloud_access_key_secret
}

locals {
  user_data = base64encode(templatefile("${path.module}/user-data.tftpl", {
    jit_config = var.shaula.jit_config
    pre_start  = var.shaula.bindings.alicloud_cloud_init_cmd
    bootstrap  = file("${path.module}/bootstrap.tftpl")
    service    = file("${path.module}/runner-service.tftpl")
  }))
}

resource "alicloud_instance" "runner" {
  instance_name = var.shaula.generation.generation_name
  image_id      = var.shaula.bindings.alicloud_image_id
  instance_type = var.shaula.bindings.alicloud_instance_type

  vswitch_id      = var.shaula.bindings.alicloud_vswitch_id
  security_groups = var.shaula.bindings.alicloud_security_group_ids

  instance_charge_type       = "PostPaid"
  spot_strategy              = "NoSpot"
  internet_charge_type       = "PayByTraffic"
  internet_max_bandwidth_out = var.shaula.bindings.alicloud_internet_max_bandwidth_out
  system_disk_category       = "cloud_essd"
  system_disk_size           = var.shaula.bindings.alicloud_system_disk_size
  system_disk_encrypted      = true

  # Generation inputs are immutable; never reboot a consumed JIT seed to reuse it.
  user_data                     = local.user_data
  deletion_protection           = false
  password_inherit              = false
  security_enhancement_strategy = "Deactive"
  http_endpoint                 = "enabled"
  http_tokens                   = "required"

  tags = {
    "shaula.managed"    = "true"
    "shaula.fleet"      = var.shaula.generation.fleet_key
    "shaula.generation" = var.shaula.generation.id
  }

  lifecycle {
    precondition {
      # A conservative encoded-size bound also keeps decoded data below the
      # ECS 16 KiB limit, including multibyte publisher preparation scripts.
      condition     = length(local.user_data) <= 16384
      error_message = "ECS user-data exceeds the template's 16 KiB base64 limit; shorten the preparation script."
    }
  }
}

output "shaula_result" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    bindings_digest  = var.shaula.bindings_digest
    resources = [
      {
        role = "runner"
        id   = alicloud_instance.runner.id
      }
    ]
  }
  sensitive = true
}

variable "shaula" {
  description = "Fixed Shaula system input envelope; all platform configuration belongs to publisher bindings."
  type = object({
    contract_version = number
    generation       = any
    jit_config       = string
    bindings_digest  = string
    bindings = object({
      alicloud_region                     = string
      alicloud_access_key_id              = string
      alicloud_access_key_secret          = string
      alicloud_image_id                   = string
      alicloud_vswitch_id                 = string
      alicloud_security_group_ids         = list(string)
      alicloud_instance_type              = optional(string, "ecs.g6.large")
      alicloud_system_disk_size           = optional(number, 40)
      alicloud_internet_max_bandwidth_out = optional(number, 0)
      alicloud_cloud_init_cmd             = optional(string, "")
    })
    parameters = object({})
  })
  sensitive = true
}

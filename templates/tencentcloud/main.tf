# Each Generation owns one CVM instance. Images, VPCs, subnets and security
# groups are publisher-selected shared resources, never managed here.
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    tencentcloud = {
      source  = "tencentcloudstack/tencentcloud"
      version = "1.83.31"
    }
  }
}

provider "tencentcloud" {
  region     = var.shaula.bindings.tencentcloud_region
  secret_id  = var.shaula.bindings.tencentcloud_secret_id
  secret_key = var.shaula.bindings.tencentcloud_secret_key
}

locals {
  forgejo = var.shaula.bindings.runner_backend == "forgejo"
  user_data = base64encode(templatefile("${path.module}/${local.forgejo ? "user-data-forgejo" : "user-data"}.tftpl", {
    jit_config = var.shaula.jit_config
    forgejo    = var.shaula.forgejo
    forgejo_vm = var.shaula.forgejo_vm
    pre_start  = var.shaula.bindings.tencentcloud_cloud_init_cmd
    bootstrap  = file("${path.module}/${local.forgejo ? "bootstrap-forgejo" : "bootstrap"}.tftpl")
    service    = file("${path.module}/${local.forgejo ? "runner-service-forgejo" : "runner-service"}.tftpl")
  }))
}

resource "tencentcloud_instance" "runner" {
  instance_name     = var.shaula.generation.generation_name
  availability_zone = var.shaula.bindings.tencentcloud_availability_zone
  image_id          = var.shaula.bindings.tencentcloud_image_id
  instance_type     = var.shaula.bindings.tencentcloud_instance_type

  vpc_id                  = var.shaula.bindings.tencentcloud_vpc_id
  subnet_id               = var.shaula.bindings.tencentcloud_subnet_id
  orderly_security_groups = var.shaula.bindings.tencentcloud_security_group_ids

  instance_charge_type       = "POSTPAID_BY_HOUR"
  allocate_public_ip         = var.shaula.bindings.tencentcloud_internet_max_bandwidth_out > 0
  internet_charge_type       = "TRAFFIC_POSTPAID_BY_HOUR"
  internet_max_bandwidth_out = var.shaula.bindings.tencentcloud_internet_max_bandwidth_out
  system_disk_type           = "CLOUD_SSD"
  system_disk_size           = var.shaula.bindings.tencentcloud_system_disk_size
  system_disk_encrypt        = true

  user_data                   = local.user_data
  user_data_replace_on_change = true
  disable_api_termination     = false
  disable_monitor_service     = true
  disable_security_service    = true
  disable_automation_service  = true
  keep_image_login            = false

  tags = {
    "shaula.managed"    = "true"
    "shaula.fleet"      = var.shaula.generation.fleet_key
    "shaula.generation" = var.shaula.generation.id
  }

  lifecycle {
    precondition {
      condition     = length(local.user_data) <= 16384
      error_message = "CVM user-data exceeds the 16 KiB base64 limit; shorten the preparation script."
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
        id   = tencentcloud_instance.runner.id
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
    forgejo          = optional(any)
    forgejo_vm       = optional(any)
    bindings_digest  = string
    bindings = object({
      runner_backend                          = optional(string, "github")
      tencentcloud_region                     = string
      tencentcloud_secret_id                  = string
      tencentcloud_secret_key                 = string
      tencentcloud_availability_zone          = string
      tencentcloud_image_id                   = string
      tencentcloud_vpc_id                     = string
      tencentcloud_subnet_id                  = string
      tencentcloud_security_group_ids         = list(string)
      tencentcloud_instance_type              = optional(string, "S5.MEDIUM4")
      tencentcloud_system_disk_size           = optional(number, 50)
      tencentcloud_internet_max_bandwidth_out = optional(number, 0)
      tencentcloud_cloud_init_cmd             = optional(string, "")
    })
    parameters = object({})
  })
  sensitive = true
}

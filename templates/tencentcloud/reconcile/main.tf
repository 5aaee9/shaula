# Read-only failed-Create discovery module (ARD-0044).
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

data "tencentcloud_instances" "runner" {
  tags = {
    "shaula.generation" = var.shaula.generation.id
  }
}

output "shaula_discover" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    roles = [
      {
        role = "runner"
        resources = [
          for inst in data.tencentcloud_instances.runner.instance_lists : {
            address   = "tencentcloud_instance.runner"
            import_id = inst.instance_id
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

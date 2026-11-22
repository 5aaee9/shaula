# Read-only failed-Create discovery module (ARD-0044).
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

data "alicloud_instances" "runner" {
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
          for id in data.alicloud_instances.runner.ids : {
            address   = "alicloud_instance.runner"
            import_id = id
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

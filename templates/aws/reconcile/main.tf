# Read-only failed-Create discovery module (ARD-0044).
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "5.67.0"
    }
  }
}

provider "aws" {
  region                      = var.shaula.bindings.aws_region
  access_key                  = var.shaula.bindings.aws_access_key_id
  secret_key                  = var.shaula.bindings.aws_secret_access_key
  skip_credentials_validation = true
  skip_requesting_account_id  = true
}

data "aws_instances" "runner" {
  instance_tags = {
    "shaula.generation" = var.shaula.generation.id
  }
  instance_state_names = ["pending", "running", "stopping", "stopped"]
}

output "shaula_discover" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    roles = [
      {
        role = "runner"
        resources = [
          for id in data.aws_instances.runner.ids : {
            address   = "aws_instance.runner"
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

# One Runner Generation owns one EC2 instance. The Canonical Ubuntu AMI,
# subnet, security groups and IAM credential are shared publisher data.
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
  region     = var.shaula.bindings.aws_region
  access_key = var.shaula.bindings.aws_access_key_id
  secret_key = var.shaula.bindings.aws_secret_access_key
  # Synthetic bindings must still produce a valid plan for validation and
  # admission review; credential checks belong to the real apply, not plan.
  skip_credentials_validation = true
  skip_requesting_account_id  = true
  default_tags {
    tags = {
      "shaula.managed" = "true"
    }
  }
}

# Canonical's official Ubuntu publications under the dedicated owner account.
# most_recent is a mutable release channel; publishers pin a dated serial in
# aws_ami_name for an immutable image. Destroy refresh does not require the
# AMI to still exist: only the instance itself is managed.
data "aws_ami" "ubuntu" {
  owners      = ["099720109477"]
  most_recent = true

  filter {
    name   = "name"
    values = [var.shaula.bindings.aws_ami_name]
  }
  filter {
    name   = "architecture"
    values = ["x86_64"]
  }
  filter {
    name   = "virtualization-type"
    values = ["hvm"]
  }
  filter {
    name   = "root-device-type"
    values = ["ebs"]
  }
  filter {
    name   = "state"
    values = ["available"]
  }
}

locals {
  generation_name = var.shaula.generation.generation_name
  security_groups = length(var.shaula.bindings.aws_security_group_ids) > 0 ? var.shaula.bindings.aws_security_group_ids : null
  subnet          = var.shaula.bindings.aws_subnet_id != "" ? var.shaula.bindings.aws_subnet_id : null
}

resource "aws_instance" "runner" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = var.shaula.bindings.aws_instance_type
  subnet_id     = local.subnet

  vpc_security_group_ids      = local.security_groups
  associate_public_ip_address = local.subnet == null ? true : null

  user_data = templatefile("${path.module}/user-data.tftpl", {
    jit_config = var.shaula.jit_config
    pre_start  = var.shaula.bindings.aws_cloud_init_cmd
    bootstrap  = file("${path.module}/bootstrap.tftpl")
    service    = file("${path.module}/runner-service.tftpl")
  })
  # Guard against accidental in-place user-data replacement semantics: a new
  # Generation is always a new instance, never a reboot of a consumed seed.
  user_data_replace_on_change = true

  metadata_options {
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
    instance_metadata_tags      = "disabled"
  }

  root_block_device {
    volume_type           = "gp3"
    volume_size           = var.shaula.bindings.aws_root_volume_gb
    encrypted             = true
    delete_on_termination = true
  }

  instance_initiated_shutdown_behavior = "terminate"
  disable_api_termination              = false
  monitoring                           = false

  tags = {
    Name                = local.generation_name
    "shaula.fleet"      = var.shaula.generation.fleet_key
    "shaula.generation" = var.shaula.generation.id
  }

  lifecycle {
    precondition {
      condition     = data.aws_ami.ubuntu.id != ""
      error_message = "The bound name pattern must resolve to exactly one available Canonical Ubuntu AMI in this region."
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
        id   = aws_instance.runner.id
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
      aws_region             = string
      aws_access_key_id      = string
      aws_secret_access_key  = string
      aws_ami_name           = optional(string, "ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-*")
      aws_instance_type      = optional(string, "t3.large")
      aws_subnet_id          = optional(string, "")
      aws_security_group_ids = optional(list(string), [])
      aws_root_volume_gb     = optional(number, 30)
      aws_cloud_init_cmd     = optional(string, "")
    })
    parameters = object({})
  })
  sensitive = true
}

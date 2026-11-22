# Read-only failed-Create discovery module (ARD-0044).
terraform {
  required_version = ">= 1.9, < 2.0"
  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "2.33.0"
    }
  }
}

provider "kubernetes" {
  config_path = var.shaula.bindings.kubeconfig
}

data "kubernetes_resources" "bootstrap" {
  api_version    = "v1"
  kind           = "Secret"
  namespace      = var.shaula.bindings.namespace
  label_selector = "shaula.io/generation-id=${var.shaula.generation.id}"
}

data "kubernetes_resources" "runner" {
  api_version    = "v1"
  kind           = "Pod"
  namespace      = var.shaula.bindings.namespace
  label_selector = "shaula.io/generation-id=${var.shaula.generation.id}"
}

output "shaula_discover" {
  value = {
    contract_version = 1
    generation_id    = var.shaula.generation.id
    roles = [
      {
        role = "bootstrap"
        resources = [
          for item in data.kubernetes_resources.bootstrap.objects : {
            address   = "kubernetes_secret_v1.bootstrap"
            import_id = "${var.shaula.bindings.namespace}/${item.metadata.name}"
          }
        ]
      },
      {
        role = "runner"
        resources = [
          for item in data.kubernetes_resources.runner.objects : {
            address   = "kubernetes_pod_v1.runner"
            import_id = "${var.shaula.bindings.namespace}/${item.metadata.name}"
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

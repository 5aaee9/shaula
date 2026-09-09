---
status: accepted
date: 2026-09-09
amends: [0004, 0019]
---

# Provision Proxmox runners with generation-owned NoCloud media

使用 `indexyz/proxmox` 0.4.0 的 template discovery、full clone、NoCloud ISO 和生命周期
开关，将 Proxmox 纳入现有 Terraform Template seam。详细协议由
[spec 0022](../specs/0022-proxmox-runner-template.md) 维护。

一个 Generation 拥有一个 VM 和一个 ISO，Terraform 的依赖图保证 seed 在启动前可用、
在 VM 删除后清理。沿用 worker、GitHub JIT、安全移除和 HTTP state backend，避免在 Rust
内新增另一个 Proxmox API 客户端和第二套资源恢复逻辑。provider 的 NoCloud 资源通过 API
上传，不要求宿主 SSH、snippet 路径或外部 ISO 构建工具。

用 Terraform 原生 templatefile/yamlencode 渲染项目固定 bootstrap，guest 启动预装的
Runner.Listener。可选 publisher 初始化脚本在固定启动前执行，不开放 Fleet 脚本输入。
固定 DHCP、继承基础 VM 配置缩小第一版范围；源模板必须唯一，节点随源模板确定。

Proxmox VM template 不具有 OCI content pin。增加显式 `vm_image_contract` 并允许其精确
空 image set，把操作员管理基础镜像的信任条件放入 artifact 和 runtime policy。
拒绝伪造 digest，也不放宽历史/容器的 image pin 规则。代价是同名 VM 内容可变，旧 conformance
无法证明后续 clone 内容一致；操作员需冻结基础模板并在更新后重新验证。

JIT 经受保护的 cloud-init 文件交付并进入 Listener 支持的输入环境，不使用长期注册 token
或自动重启服务。ISO、cloud-init 缓存和 Terraform state 仍可能保留 JIT，属于凭据材料。
此路径在 apply 内启动 VM，因此本次不把容器专用的 post-apply Setup Info 门槛推广到 VM。

provider 的 task 等待时限、节点本地 upload 和实际 guest 行为需要平台验收。静态激活及
Terraform mock 通过只证明对应边界，不替代 clone、DHCP、真实 GitHub job 和清理的完整验证。

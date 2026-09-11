---
status: proposed
date: 2026-09-11
amends: [0004]
---

# Provision AWS runners with EC2 user-data and official Ubuntu images

使用 `hashicorp/aws` provider 的 AMI 解析、单实例生命周期与 user-data cloud-init，
将 AWS EC2 纳入现有 Terraform Template seam，路径与 Proxmox（ARD-0026）同构。
详细协议由 [spec 0027](../specs/0027-aws-runner-template.md) 维护。

Provider lock 随 artifact 冻结前必须覆盖真实 Terraform 宿主平台，按 spec 0004 §3
从受信来源准备目标平台校验和，并在目标平台 readonly init 后运行 validate 和
不执行 apply 的 plan。缺少 Linux `h1:` 等校验材料的修复通过新 artifact/Revision
与显式 Fleet 采用交付，不改历史材料、不关闭 checksum 校验，也不把静态 Active
冒充运行兼容证明。

## Context

Proxmox 模板证明了 VM 类平台不需要新 Executor Driver 或 Rust SDK：Terraform 拥有
资源生命周期，JIT 经 cloud-init 交付，guest 内固定 bootstrap 启动一次性 Listener。
AWS 的区别在于：NoCloud ISO 由 EC2 原生 user-data 机制替代（单一 `aws_instance`
资源即可，不需要第二个"bootstrap"资源），且不存在操作员自建的"模板 VM"——
可信来源是 Canonical 在 owner account `099720109477` 下发布的官方 Ubuntu AMI。

选官方 Ubuntu 镜像并在 guest 内安装 runner，而不是要求操作员预烘焙 AMI，理由：
第一版不引入镜像构建管线（Packer/EBS snapshot 流程）与发布者侧的 AMI 分发负担；
runner 版本随 artifact 以"版本 + sha256"常量固定，升级走不可变 Revision 语义；
代价是每次启动增加一次受校验的下载，且 guest 需要到 GitHub/对象存储的出站。

## Decision

- 一个 Generation 拥有一个 `aws_instance`；`aws_ami` data source 按 Canonical
  owner + name pattern + `most_recent` 解析镜像。模板不创建 IAM role、key pair、
  EIP、额外卷或安全组资源。
- manifest 使用 `vm_image_contract: shaula.aws-ami/v1` 与精确空
  `runner_image_digests`。AMI 是外部发布通道而非 OCI 内容钉：`most_recent`
  的解析结果可变，信任条件写入 artifact 与 runtime policy，publisher 可用
  dated serial pattern 换取不可变镜像。拒绝伪造 digest，也不放宽容器镜像 pin 规则。
- runner tarball 版本与 sha256 固定在模板材料中，guest 先校验后解压到
  `/opt/actions-runner`；不作为 binding，避免把软件供应链决定交给发布配置。
  不提供 guest 内 `latest` 解析：releases API 无可信 checksum 通道，且可变下载
  违背不可变材料语义；注册后的服务端自更新已覆盖“运行最新版”的需求。
- JIT 经 cloud-init `write_files` 以受限权限落盘，一次性 systemd 服务消费并删除
  后交给 `Runner.Listener run`；不使用长期注册 token、config.sh 或自动重启服务。
- 实例强制 IMDSv2、EBS 加密与 `delete_on_termination`；tags 携带 fleet/generation
  身份但不携带任何凭据。

## Consequences

沿用 worker、GitHub JIT、安全移除和 HTTP state backend，不在 Rust 内新增 AWS
客户端或第二套资源恢复逻辑。user-data 路径在 apply 内启动实例，因此不把容器专用的
post-apply Setup Info 门槛推广到 VM（同 ARD-0026 的取舍）。

已知代价与边界：user-data、cloud-init 缓存、IMDS、Terraform state 与 EC2 console
output 均属凭据材料，不声称对拥有 root 的 workflow 隔离 JIT；`most_recent` AMI
是可变输入，同名 pattern 之后的解析结果不受旧 conformance 证明；实例冷启动 +
runner 下载显著慢于容器路径，容量与超时预算需实测；静态凭据 binding 是第一版
认证模式，instance role/OIDC 等环境另行验收。

provider 的等待时限、AMI 解析行为与真实 guest 引导需要平台验收。静态激活及
mock 测试通过只证明对应边界，不替代实例启动、runner 注册、真实 GitHub job
和清理的完整验证。

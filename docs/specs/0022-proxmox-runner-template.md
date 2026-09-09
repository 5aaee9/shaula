# Proxmox Runner Template

- Status: Accepted
- Date: 2026-09-09
- Decision: [ARD-0026](../ard/0026-provision-proxmox-runners-with-nocloud.md)
- Extends: [spec 0004](0004-template-profile-runtime.md),
  [spec 0005](0005-profile-http-control-plane.md),
  [spec 0015](0015-template-library-and-variable-discovery.md) and
  [spec 0021](0021-default-template-updates.md).

## 1. Outcome and ownership

新增稳定的默认 Template Source `proxmox`。沿用数据库 archive、不可变 Profile Revision、
Terraform `exec` 生命周期和变量发现；无需新增 Executor Driver、Rust Proxmox SDK 或宿主 SSH。
一个 Generation 管理一个 `proxmox_qemu_vm`（`runner`）和一个 `proxmox_nocloud_iso`
（`bootstrap`）。固定 provider `indexyz/proxmox` `0.4.0`，提交真实 dependency lock。

Terraform 是 VM 和 ISO 的唯一创建/销毁者。Shaula 仍负责 JIT、GitHub occupancy、安全移除、
worker fencing、state 和恢复；平台启动成功不等于 Runner 在线或 workflow 成功。
静态校验后的自动激活沿用 spec 0017，真实平台 conformance 独立记录。

## 2. Inputs

只声明一个 sensitive 的 `variable "shaula"`，使用 input contract v1。
下列配置属于可信 publisher 的 bindings，类型和默认值定义在 Terraform，schema 只补充
描述、边界和敏感性。Fleet parameters 为空对象，不暴露脚本、平台凭据或网络配置。

| Binding | Type | Default | Meaning |
| --- | --- | --- | --- |
| `proxmox_host` | string | required | HTTPS Proxmox API origin，例如 `https://pve.example:8006` |
| `proxmox_token` | sensitive string | required | `user@realm!tokenid=secret`，仅按第一个 `=` 拆分 |
| `proxmox_insecure` | bool | `true` | 是否跳过服务器证书验证；由 publisher 明确选择 |
| `proxmox_template_name` | string | `GitHub-Runner` | 精确匹配的基础 VM template 名称 |
| `proxmox_vmid_begin` | integer | `100` | provider `vm_id_start`，不是预先保留的 VMID |
| `proxmox_iso_storage` | string | `local` | VM 所在节点可读写、支持 ISO 的 storage |
| `proxmox_cloud_init_cmd` | string | empty | 可选的 root 初始化脚本，在固定 JIT 启动前运行 |

`proxmox_cloud_init_cmd` 是受信任的发布代码，不是凭据容器，不支持插入平台 token 或自行替换
JIT 注册逻辑。以独立文件传入，不把用户脚本文本再次解释为 Terraform template 表达式。
失败阻止 Listener 启动；成功后始终执行项目固定的 JIT handoff。它不能成为 Fleet input。

默认网络为 DHCP IPv4；不提供 bridge、VLAN、静态地址、DNS 或 NIC 配置项。
继承基础 VM 的磁盘、CPU、内存和 NIC。第一版要求 Linux、systemd、cloud-init、DHCP 可达网络、
预装 `/opt/actions-runner` 及其依赖和 `runner` 用户；基础镜像不能携带已注册 runner 状态或
自动运行的旧 runner 服务。支持的网卡命名及软件前提由随 artifact 固定的 runtime policy 声明。

## 3. Managed image contract

manifest 增加可选 `vm_image_contract: shaula.proxmox-template/v1`。只接受以下组合：
`platform: proxmox`、`bindings_contract: shaula.bindings.proxmox/v1`、input v1、
没有 setup-info/container-bootstrap contract，且 `runner_image_digests: []`。
未知 contract、其他平台或混用 OCI pins 均拒绝。

未声明此 contract 的历史/容器 manifest 保持原有 1–8 个不可变 image digest 校验。
本声明明确接受基础 VM 由操作员维护、按名字在 Create 时解析，不能据此声称整个 VM 镜像可复现。
不以虚构 image digest、脚本 digest 或 VMID 代替磁盘内容证明。runtime policy 必须记录这一边界。
变更基础 VM 内容需要重新进行平台验收，即使 Profile bindings 字节没有变化。

attestation subject 仍使用现有格式，精确比较空 image set；artifact digest 绑定该显式 contract，
runtime policy 和 binding commitment 绑定信任条件。这样的 evidence 只描述被测试的基础 VM
环境，不证明后来同名模板仍有相同内容。Proxmox 成为有界 `platform=proxmox` metric label。

## 4. Create and bootstrap

1. 用 `proxmox_qemu_vms` 精确筛选名称和 `template=true`；结果必须恰好一个，零个或多个拒绝。
   在该 template 的节点执行 full clone；VMID 从 `proxmox_vmid_begin` 开始由 provider 申请。
   竞争失败不能采用别人的 VM，也不能把 unknown outcome 当作安全重试。
2. Generation 已持久化的 `generation_name` 派生 VM 名、ISO 文件名和 NoCloud `instance-id`，
   重试不能重新随机命名。ISO 的 user-data、meta-data 和 network-config 由 Terraform
   `templatefile` / `yamlencode` 生成；不引入另一个模板引擎。
3. 先建立独占 ISO，再 full clone、配置 `ide2` CD-ROM（明确 `media=cdrom` 和 ISO volume），
   开启 provider `nocloud_cdrom_slot=ide2` 校验后启动。该槽只能为空或可安全替换的原生
   PVE cloud-init 介质；其他 seed、占用槽或目标节点不可见的 ISO 拒绝。
4. `start_on_create=true`、`stop_on_destroy=true`、`onboot=false`、`protection=false`。
   不管理基础 VM/root disk，也不新建额外 Terraform provisioner、远程执行资源或持久服务。
5. cloud-init 交付权限受限的 JIT 文件及固定 bootstrap，启动一个不重启的 systemd 服务。
   固定 bootstrap 消费并删除临时 JIT 文件，将 `ACTIONS_RUNNER_INPUT_JITCONFIG` 交给
   `runner` 用户的 `/opt/actions-runner/bin/Runner.Listener run`，不用 config.sh 注册 token
   或 svc.sh 常驻循环。JIT 不能写入 shell/Listener argv、命令日志或 Terraform公开输出。
   服务不 enable，不在 reboot 或失败后重新注册同一 Generation。

API token 只用于 provider 配置，绝不能进入 ISO、guest 文件或启动环境。cloud-init、ISO、
Terraform state、provider 临时目录仍是 credential-grade 材料；删除一个 guest JIT 文件不等于
擦除 datasource 缓存或 ISO，不能声称对 VM root/workflow 隔离 JIT。运行日志遵守现有敏感输出
处理。此版本不实现 spec 0020 的宿主 post-apply `.setup_info` 交付，因为 VM 已在 apply 中启动。

## 5. Destroy, recovery and deployment

`shaula_result` 回传 Generation ID、binding commitment 和两项不透明资源 ID。
先通过现有 GitHub safe-removal gate，再由 provider 停机、删除 VM，最后删除依赖的 ISO。
ISO 替换必须带动 VM 替换，不能把正在使用的 seed 原地覆盖。
等待 task 失败、state 写入失败或 Create outcome 未知时，保留原材料并沿用恢复/隔离流程；
不能丢弃 state 或用一个新的 Generation 冒充重试成功。

Destroy 使用现有 refresh-enabled saved plan，不能依赖源模板仍存在或仍唯一。查询无法
解析时只用受精确唯一性 precondition 保护的占位值满足 Terraform 必填配置检查；Create
计划必须失败且无 mutation，Read/Delete 始终使用原 state 中的 VM/ISO 身份。这些占位值
不是备用节点或可用的 clone 来源。唯一性条件放在受管 ISO 上，使 Destroy 跳过该条件时
仍符合原有严格的 deleted-resource check 规则，不放宽通用 plan admission。

默认导入的允许列表增加根目录 `.tftpl`，仍不采集任意脚本、链接、缓存或额外子目录。
Nix 包提供 `share/shaula/templates/proxmox` 及其 `.tf`、`.tftpl`、manifest、schemas、lock、
runtime policy；启动同步进入数据库 source catalog，旧 archive/Revision 不受更新影响。

第一版使用直接到目标节点的 API origin 和该节点 ISO storage；多节点上传代理/共享存储
必须独立验收。provider 0.4.0 的单项 PVE task 等待有 10 分钟上限；较大的 full clone、启动
延迟和 JIT 有效性须实测，不能用增大单个 HTTP timeout 宣称解决。

## 6. Acceptance

- Rust tests：显式 VM image contract 与不合法混用、旧 manifest 兼容、变量默认值/敏感性/
  空 Fleet inputs、真实 archive 导入保留 `.tftpl` 和拒绝不安全目录项。
- Terraform：真实锁定 provider 的 init/validate；测试唯一/缺失/歧义 template、token 分割、
  DHCP、generation identity、两资源依赖、启动/销毁开关和 JIT 编码后的 cloud-init 内容。
  synthetic/mock 测试结果必须标明，不冒充真实 PVE。
- 真实平台：从未注册基础 VM full clone，经 DHCP/cloud-init 在目标 repository 注册 JIT runner，
  完成一个 job，Busy 时不得强删，正常退役后 VM/ISO 和 GitHub runner 均清理。
  补充创建/启动失败恢复、并发 VMID、service 重启、token/seed 不泄漏和原始 Revision 恢复。
  未提供 PVE 环境时，明确保留这些待验收项，不给出生产 Ready 结论。

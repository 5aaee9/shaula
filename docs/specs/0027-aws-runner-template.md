# AWS Runner Template

- Status: Draft
- Date: 2026-09-11
- Decision: [ARD-0034](../ard/0034-provision-aws-runners-with-ec2-user-data.md)
- Extends: [spec 0004](0004-template-profile-runtime.md),
  [spec 0005](0005-profile-http-control-plane.md),
  [spec 0015](0015-template-library-and-variable-discovery.md),
  [spec 0021](0021-default-template-updates.md) and
  [spec 0022](0022-proxmox-runner-template.md).

## 1. Outcome and ownership

新增稳定的默认 Template Source `aws`。沿用数据库 archive、不可变 Profile Revision、
Terraform `exec` 生命周期和变量发现；无需新增 Executor Driver、Rust AWS SDK、
SSM Session、宿主 SSH 或 key pair。一个 Generation 管理一个 `aws_instance`（`runner`），
JIT 经 EC2 user-data 由 cloud-init 交付。固定 provider `hashicorp/aws`，版本与真实
dependency lock 随 artifact 冻结。

该 lock 必须覆盖实际 Terraform 执行端；Linux 部署需要冻结经来源验证的
`linux_amd64` provider checksum（包括安装路径所需的 `h1:`），不能只在开发机生成 lock
后凭 `zh:` 列表假定 validate/plan 可用。验证与不可变发布规则归 spec 0004 §3，
同 spec 0022 §1 的 Proxmox lock 教训。

Terraform 是实例的唯一创建/销毁者。Shaula 仍负责 JIT、GitHub occupancy、安全移除、
worker fencing、state 和恢复；实例到达 `running` 不等于 Runner 在线或 workflow 成功。
静态校验后的自动激活沿用 spec 0017，真实平台 conformance 独立记录。

## 2. Inputs

只声明一个 sensitive 的 `variable "shaula"`，使用 input contract v1。
下列配置属于可信 publisher 的 bindings，类型和默认值定义在 Terraform，schema 只补充
描述、边界和敏感性。Fleet parameters 为空对象，不暴露脚本、平台凭据、AMI ID 或网络配置。

| Binding | Type | Default | Meaning |
| --- | --- | --- | --- |
| `aws_region` | string | required | EC2 region，例如 `us-east-1`；约束为合法 region 形态 |
| `aws_access_key_id` | sensitive string | required | 静态 IAM access key，仅传给 provider |
| `aws_secret_access_key` | sensitive string | required | 对应 secret，仅传给 provider |
| `aws_ami_name` | string | `ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-*` | Canonical 官方 Ubuntu AMI name pattern，按 `most_recent` 解析 |
| `aws_instance_type` | string | `t3.large` | 实例规格，schema 约束非空标识符 |
| `aws_subnet_id` | string | optional empty | 目标 subnet；空时使用 region 默认 VPC 的默认 subnet |
| `aws_security_group_ids` | list(string) | `[]` | 附加 security group；空时使用 subnet 所在 VPC 默认 SG |
| `aws_root_volume_gb` | integer | `30` | gp3 root volume 大小（GiB），schema 限定 8–200 |
| `aws_cloud_init_cmd` | string | empty | 可选的 root 初始化脚本，在 runner 安装与固定 JIT handoff 之前运行 |

`aws_cloud_init_cmd` 是受信任的发布代码，不是凭据容器，不支持插入平台 token、AWS 凭据
或自行替换 JIT 注册逻辑。以独立 `.tftpl` 文件传入，不把脚本文本再次解释为 Terraform
template 表达式。失败阻止 Listener 启动；成功后始终执行项目固定的安装与 JIT handoff。
它不能成为 Fleet input。

网络语义：默认依赖目标 subnet 的公网/NAT 出站与 DHCP；不提供 Elastic IP、
静态地址、DNS、IPv6 或多 ENI 配置项。Runner 只需要出站到 GitHub 与对象存储，
模板不创建 inbound 规则、不绑定 SSH key，也不创建 IAM role/instance profile
（第一版实例无 IAM identity；需要 instance profile 的环境另行验收）。

## 3. Managed image contract

manifest 增加可选 `vm_image_contract: shaula.aws-ami/v1`。只接受以下组合：
`platform: aws`、`bindings_contract: shaula.bindings.aws/v1`、input v1、
没有 setup-info/container-bootstrap contract，且 `runner_image_digests: []`。
未知 contract、其他平台或混用 OCI pins 均拒绝。实现可将现有
`shaula.proxmox-template/v1` 校验泛化为按 platform 分派的 VM image contract 表，
但每种 platform 的允许组合必须显式列出，不接受泛化通配。

未声明此 contract 的历史/容器 manifest 保持原有 1–8 个不可变 image digest 校验。
本声明明确接受 guest 镜像为 Canonical 发布的官方 Ubuntu AMI：按 owner account
`099720109477` 与 name pattern 在 Create 时解析，不能据此声称磁盘内容可复现。
不以虚构 OCI digest、AMI ID 或快照 ID 代替磁盘内容证明。runtime policy 必须记录
这一边界：name pattern 是可变的发布通道，`most_recent` 解析结果随 Canonical
上新而变化；锁定 pattern 到某一 dated serial（如 `ubuntu-jammy-22.04-amd64-server-20240911`）
可获得不可变镜像，是否固定由 publisher 在 bindings 中选择。

Runner 软件不由镜像携带：固定 bootstrap 在 guest 内下载
`actions-runner-linux-x64-<version>.tar.gz`，版本与 sha256 以常量形式固定在模板材料中，
下载后先校验 digest 再解压到 `/opt/actions-runner`。这是 artifact 固定材料，
不是 binding 或 Fleet input；升级 runner 版本按新 artifact/Revision 发布。
不支持在 guest 内解析 `latest`：可变下载破坏 artifact 的材料确定性，且 GitHub
releases 不提供可机器信任的 asset checksum 通道。固定安装版本不限制最终运行版本——
`Runner.Listener` 注册后按服务端要求自更新（除非显式 `--disableupdate`，本模板不使用），
实际执行 job 的 runner 版本始终由 GitHub 服务侧决定。
attestation subject 仍使用现有格式，精确比较空 image set；conformance evidence
只描述被测 AMI 解析结果与 runner 版本，不证明后续 `most_recent` 解析到同一 AMI。
AWS 成为有界 `platform=aws` metric label。

## 4. Create and bootstrap

1. 用 `aws_ami` data source 以 `owners = ["099720109477"]`、`most_recent = true`、
   name pattern、architecture `x86_64`、virtualization `hvm`、root-device `ebs` 过滤；
   结果必须恰好一个，零个拒绝。`aws_instance` 的 `ami` 引用该解析结果；
   Destroy 的 refresh 不要求 AMI 仍存在——受管资源只有实例本身，AMI 是 data source。
2. Generation 已持久化的 `generation_name` 写入 `Name` tag 与
   `shaula.fleet`/`shaula.generation` tags，重试不能重新随机命名。
   user-data 由 Terraform `templatefile`/`yamlencode` 生成 cloud-config；
   不引入另一个模板引擎。
3. `user_data` 内含三段固定材料：受限权限的 JIT 文件、runner 安装/启动脚本、
   一次性 systemd 服务。cloud-init 写入后由服务消费并删除临时 JIT 文件，
   将 `ACTIONS_RUNNER_INPUT_JITCONFIG` 交给 `runner` 用户的
   `/opt/actions-runner/bin/Runner.Listener run`；不用 config.sh 注册 token、
   svc.sh 常驻循环或 `--once` 之外的注册路径。服务不 enable，不在 reboot 或失败后
   重新注册同一 Generation。
4. `metadata_options` 固定 `http_tokens = "required"`（IMDSv2）、
   `http_put_response_hop_limit = 1`；`disable_api_termination = false`、
   `instance_initiated_shutdown_behavior = "terminate"`。
   不管理 IAM、key pair、EIP、额外 EBS 或 ENI，也不新建 Terraform provisioner、
   remote-exec 或持久资源。
5. root volume 固定 `gp3`、大小取 `aws_root_volume_gb`、`encrypted = true`
   （使用 account 默认 EBS encryption key 或 AWS managed key，不新建 KMS 资源）、
   `delete_on_termination = true`。

AWS 凭据只用于 provider 配置，绝不能进入 user-data、guest 文件、tags 或启动环境。
user-data、cloud-init 缓存、Terraform state、EC2 console output 和 IMDS 仍是
credential-grade 材料：IMDSv2 只限制获取方式，不阻止 guest 内 root/sudo workflow
读取 user-data 或 JIT；删除 guest 临时 JIT 文件不等于擦除 datasource 缓存，
不能声称对 workflow 隔离 JIT。运行日志遵守现有敏感输出处理。此版本不实现
spec 0020 的宿主 post-apply `.setup_info` 交付，因为实例在 apply 中自行启动。

## 5. Destroy, recovery and deployment

`shaula_result` 回传 Generation ID、binding commitment 和一项不透明资源 ID
（`aws_instance` id）。先通过现有 GitHub safe-removal gate，再由 provider 终止实例；
root volume 随 `delete_on_termination` 回收。实例 `running`/`stopped` 状态不作为
安全移除依据。

等待 apply 超时、state 写入失败或 Create outcome 未知时，保留原材料并沿用
恢复/隔离流程；不能丢弃 state、用新 Generation 冒充重试成功，或按 tag 名称
手动回收实例来代替 Destroy。Destroy 使用现有 refresh-enabled saved plan；
`aws_ami` data source 在 refresh 中重新解析失败不阻止实例终止——
data 结果只服务 Create 计划，plan admission 记录的是当时解析的 AMI ID。

默认导入的允许列表沿用 `.tf`/`.tf.json`/`.tftpl`/schemas/lock/runtime-policy，
不采集任意脚本、链接、缓存或额外子目录。Nix 包提供
`share/shaula/templates/aws`；启动同步进入数据库 source catalog，
旧 archive/Revision 不受更新影响。

修复 provider lock、runner 版本或 AMI pattern 默认值时发布新 artifact/Template
Revision，并显式更新 Fleet 的 exact pin；不原地修改已发布 artifact、历史 Revision
或已有 Generation 的冻结材料。不能用关闭 checksum 校验、运行时改写 lock 或
重新 apply 原 Generation 绕过该错误。

实例创建到 runner 上线的时长包含 AMI 首次启动、runner tarball 下载与解压，
明显长于容器路径；apply/引导超时的具体预算以真实平台验收量测为准，
不能仅凭 API 返回时间宣称。AWS API 限流、容量不足（`InsufficientInstanceCapacity`）
和 subnet 无可用 IP 是平台失败，不冒充 Shaula 侧取消。

## 6. Acceptance

- Rust tests：`shaula.aws-ami/v1` contract 的显式接受与不合法混用（其他 platform、
  非空 `runner_image_digests`、input v2、setup-info/container-bootstrap 共存）、
  旧 manifest 兼容、变量默认值/敏感性/空 Fleet inputs、`platform=aws` metric label、
  真实 archive 导入保留 `.tftpl`。
- Terraform：在真实执行平台以冻结 lock 运行 `init -lockfile=readonly`，随后实际执行
  validate 和无 apply 的 plan，确认 provider 可加载且 lock 未变。测试 AMI 过滤
  零结果拒绝、IMDSv2/encryption/termination 开关、tags 与 generation identity、
  user-data 渲染中 JIT 的编码与权限、synthetic 凭据不进入 guest 材料。
  synthetic/mock 测试结果必须标明，不冒充真实 AWS。
- 真实平台：在独立 subnet/SG 启动实例，经 cloud-init 下载校验并安装 runner、
  注册 JIT runner、完成一个 job；Busy 时不得强删，正常退役后实例、root volume
  与 GitHub runner 均清理。补充创建中断恢复、并发命名、服务重启不重注册、
  token/user-data 不泄漏到 tags/console output、IMDSv2 强制生效和原始 Revision 恢复。
  未提供 AWS 环境时，明确保留这些待验收项，不给出生产 Ready 结论。

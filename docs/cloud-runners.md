# 腾讯云与阿里云 Runner

[文档索引](README.md) · [项目首页](../README.md)

项目内置两个云主机 Template Source：

- `tencentcloud`：为每个 Generation 创建一台腾讯云 CVM；
- `alicloud`：为每个 Generation 创建一台阿里云 ECS。

二者均使用 Terraform 的标准 `exec` 生命周期和现有 GitHub safe-removal gate，
不引入云厂商 SDK、SSH、远程 provisioner 或常驻管理代理。实例通过 cloud-init
一次性安装、校验并启动 GitHub Actions Runner；Generation 清退时由 Terraform
销毁实例及其系统盘。

## 发布前准备

在 Templates 页面选择 `tencentcloud` 或 `alicloud` source，填写 publisher bindings
后发布 Revision，再将 Fleet 绑定到该 Revision。平台设置均属于 publisher bindings，
Fleet 没有可覆盖的 parameters。

共同要求：

1. 选择 Ubuntu Server 22.04 LTS x86_64 镜像。镜像必须包含 systemd 和适用于对应
   云平台的 cloud-init datasource；阿里云镜像还须支持 IMDSv2。
2. 使用已有 VPC 网络和已有安全组。Runner 不需要入站端口，但必须能出站访问
   GitHub Actions、GitHub Release 下载地址和 Ubuntu 软件源。
3. 默认公网带宽为 `0`，实例不分配公网 IP，因此子网/vSwitch 需要 NAT 出站。
   将带宽改为正数会为实例分配按流量计费的公网 IP。
4. 使用最小权限 CAM/RAM 凭据，只授予实例生命周期、镜像与网络查询、标签和
   加密系统盘所需权限。凭据仅进入 Terraform provider，不会写入云主机。
5. 确认账号已经开通系统盘加密所需 KMS 能力，并确认目标可用区支持默认实例
   规格和磁盘类型。

腾讯云另外需要显式填写 region、availability zone、VPC、subnet 和至少一个
security group。阿里云需要 region、vSwitch 和至少一个 security group；vSwitch
决定实例所在可用区。

## 镜像和安全边界

两个模板都要求显式填写镜像 ID。镜像 ID 是 publisher 审核后的选择，不是磁盘内容
摘要；`shaula.tencentcloud-image/v1` 和 `shaula.alicloud-image/v1` 仅明确记录这一
信任边界，不能据此声称实例磁盘可复现。请在发布前固定并验收具体镜像版本。

JIT config 会包含在 cloud user-data 中。模板将 guest 内文件设为 root-only、使用
IMDSv2（阿里云）并在 Listener 启动前删除临时 JIT 文件，但云平台 metadata、
cloud-init 缓存、Terraform plan/state 和诊断日志在实例生命周期内仍属于敏感材料。
能获取 root 权限或访问 metadata 的 workflow 可能读取 JIT；不要将这类 Runner
视为针对恶意 workflow 的隔离边界。

## 验收

仓库测试只覆盖 artifact、manifest、变量、provider schema 和静态 Terraform 校验。
投入生产前仍须在目标账号执行真实 conformance：创建实例、等待 cloud-init、完成
一次 job、验证 busy 时不强删，以及确认正常清退后实例和系统盘均已释放。容量不足、
API 限流、网络中断、镜像不可用和 KMS/权限错误会作为 Terraform 平台错误呈现。

更细的不可变材料、凭据和恢复边界见各 artifact 内的 `runtime-policy.md`。

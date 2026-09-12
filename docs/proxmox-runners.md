# Proxmox Runner

内置 `proxmox` 模板通过 `indexyz/proxmox` 0.5.0 创建一个链接克隆 VM 和专属 NoCloud ISO。
模板随包启动同步到数据库，进入 **Templates → New template → Default template** 后选择
`proxmox`；发布时配置平台 bindings，Fleet 选择发布后的 Template，无需 Fleet inputs。
协议见 [spec 0022](specs/0022-proxmox-runner-template.md)。

## 准备基础 VM

默认名称为 `GitHub-Runner`，在 API 可见范围内必须唯一且标记为 template。
准备 Linux、systemd、cloud-init、udev、bash、util-linux（runuser/blkid/findmnt/umount）、
DHCP 网络，以及拥有 `/opt/actions-runner` 的 `runner` 用户和已安装依赖的 GitHub Actions runner。
不要预先注册 runner，也不要启用旧 runner 服务。cloud-init 状态须按镜像制作流程清理，
让每次 clone 的新 instance-id 触发初始化。

模板继承原 VM 的 CPU、内存、磁盘和 NIC，使用单张名称以 `en` 或 `eth` 开头的 Ethernet NIC；
NoCloud network-config 固定 DHCP IPv4。当前没有 bridge、VLAN 或静态地址选项。
`ide2` 必须为空或原生 PVE cloud-init 介质，不能留另一份 seed 或安装光盘。
基础 VM 内容由你管理：同名模板被修改后，后续 runner 会使用新内容，需要重新验收。

## 发布配置

必填 `proxmox_host`（例如 `https://pve.example.com:8006`）和 `proxmox_token`
（`user@realm!tokenid=secret`）。token 只传给 provider，不能写进下面的初始化脚本。
使用直接指向基础 VM 节点的 API 地址；该节点须具有可用、支持 ISO 上传的 storage。
多节点入口/共享存储需要另行验证，不能仅凭同名 storage 假定文件可见。

其余配置已有 Terraform 默认值，页面自动发现并显示：

| 配置 | 默认值 |
| --- | --- |
| `proxmox_insecure` | `true`；使用可信证书时设置 `false` |
| `proxmox_template_name` | `GitHub-Runner` |
| `proxmox_vmid_begin` | `100` |
| `proxmox_iso_storage` | `local` |
| `proxmox_full_clone` | `false`；链接克隆共享基础 VM 磁盘，其存储须支持链接克隆，不支持时设置 `true` |
| `proxmox_cloud_init_cmd` | 空，不执行额外初始化 |

可选初始化脚本以 root 执行，成功后运行固定 JIT bootstrap；失败则不启动 Listener。
它适合设置工作目录等镜像初始化步骤，不要包含平台凭据、注册 token 或无限等待的命令。
不支持 `${token}` 等替换语法；JIT 由固定模板安全编码、交付，脚本不需要处理它。

固定 bootstrap 将 JIT 交给 `Runner.Listener run`，systemd 服务不 enable、失败不自动重启。
任务结束后 Shaula 按原安全移除流程停机并删除 VM，再删除 ISO。
此版本不提供容器路径的 `.setup_info` 文件，Apply/Destroy 日志仍可在 Shaula 查看。

## 验证与恢复

发布后的 **Active** 只表示静态校验通过。先使用测试 Fleet，确认 clone、DHCP、cloud-init、
GitHub runner 在线、一个实际 job 成功，以及退役后 VM、ISO 和 GitHub runner 都被清理。
并发分配 VMID、错误存储、模糊模板名和创建中断也应在生产启用前验证。

provider 的每项 PVE task 等待上限为 10 分钟；大型 full clone 应先量测。
错误时保留 Shaula 的 state/Generation 恢复材料，不手动清空 state 或复用 VM/ISO 名称。
ISO、cloud-init 缓存、Terraform state 和 provider 临时目录可能保存 JIT；它们是凭据材料。
guest 临时 JIT 文件清除不代表对拥有 root/sudo 的 workflow 提供隔离。

上游接口与已知边界见 [provider NoCloud 指南](https://github.com/Indexyz/terraform-provider-proxmox/blob/v0.5.0/docs/guides/nocloud-runner-vm.md)。

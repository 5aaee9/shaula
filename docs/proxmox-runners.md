# Proxmox Runner

内置 `proxmox` 模板通过 `indexyz/proxmox` 0.7.1 创建一个链接克隆 VM 和专属 NoCloud ISO。
模板随包启动同步到数据库，进入 **Templates → New template → Default template** 后选择
`proxmox`；发布时配置平台 bindings，Fleet 选择发布后的 Template，并可为每个 Fleet 选择 CPU
核心数和内存大小。
协议见 [spec 0022](specs/0022-proxmox-runner-template.md)。

## 准备基础 VM

默认名称为 `GitHub-Runner`，在 API 可见范围内必须唯一且标记为 template。
准备 Linux、systemd、cloud-init、udev、bash、util-linux（runuser/blkid/findmnt/umount）、
DHCP 网络，以及拥有 `/opt/actions-runner` 的 `runner` 用户和已安装依赖的 GitHub Actions runner。
不要预先注册 runner，也不要启用旧 runner 服务。
选择 Forgejo 时，基础 VM 可预装 `/opt/forgejo-runner/forgejo-runner`：仅当其 SHA-256 与模板固定版本一致时直接使用，
否则仍下载并校验；已有 curl、python3、git 时跳过 apt。
选择 GitLab 时同理：可预装官方 `/opt/gitlab-runner/gitlab-runner` 19.4.1 裸二进制（不要装 `.deb`，
它会启用常驻 `gitlab-runner run` 服务）；SHA-256 不符时下载 `gitlab-runner-linux-amd64` / `-arm64`
并校验，已有 curl、git 时跳过 apt。见下文 GitLab 小节。cloud-init 状态须按镜像制作流程清理，
让每次 clone 的新 instance-id 触发初始化。

模板继承原 VM 的磁盘和 NIC；Fleet 可在批准的选项中选择 CPU 核心数和内存大小。使用单张名称以 `en` 或 `eth` 开头的 Ethernet NIC；
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

Fleet 参数由模板发布者批准后，在 Fleet 创建页选择：

| 参数 | 默认值 | 可选值 |
| --- | --- | --- |
| `cpu_cores` | `2` | `1`, `2`, `4`, `8` |
| `memory_mb` | `4096` MiB | `2048`, `4096`, `8192`, `16384` MiB |

可选初始化脚本以 root 执行，成功后运行固定 JIT bootstrap；失败则不启动 Listener。
它适合设置工作目录等镜像初始化步骤，不要包含平台凭据、注册 token 或无限等待的命令。
不支持 `${token}` 等替换语法；JIT 由固定模板安全编码、交付，脚本不需要处理它。

固定 bootstrap 将 JIT 交给 `Runner.Listener run`，systemd 服务不 enable、失败不自动重启。
任务结束后 Shaula 按原安全移除流程停机并删除 VM，再删除 ISO。
此版本不提供容器路径的 `.setup_info` 文件，Apply/Destroy 日志仍可在 Shaula 查看。

## GitLab（Proxmox / AWS / TencentCloud / AliCloud 通用）

在 provider bindings 中加入 `"runner_backend": "gitlab"`，Fleet 选择 `gitlab` kind 即可；
四个 VM 模板都声明 `gitlab_vm_bootstrap_contract: shaula.gitlab-vm-cloud-init/v1`。

- 只支持 `shell` executor：job 直接以非 root `runner` 用户在这台一次性 VM 内执行，
  忽略 `image:` / `services:`。VM 内 `docker` executor 不在本切片。
- Shaula 服务端先注册一个 GitLab Runner，再把 URL、Runner id/name、**单注册 token** 与
  `executor = "shell"` 渲染成唯一的 `config.toml`（与 Docker 模板同一实现）。Auth Profile
  的访问令牌、云 provider 凭据不会进入 VM；非规范的 `config.toml`（其他 executor、额外键、
  非 `glrt-` token）在 Shaula 侧即被拒绝。
- 引导以 root 领取一次性 marker、校验官方 gitlab-runner 19.4.1（amd64
  `5a159b36c5ff54fda9248e00bb0cdfb62c1112ab3741f5282c04a41e94da1306`、arm64
  `9763a63f7bc2319119f80de41d1d414465b5268d072bcd55266020b989e93ee9`，来自上游
  `release.sha256`），保护 cloud-init 缓存（Proxmox 另保护 CIDATA），把配置移到
  `/run/shaula-gitlab/config.toml`（目录 `0700`、文件 `0600`，属 `runner`），然后执行
  `gitlab-runner run-single --config … --runner <name> --max-builds 1 --wait-timeout 600`。
  token 不进入 argv/env；不会在 guest 内 `register`。
- systemd unit `Restart=no`、不 enable、`NoNewPrivileges=true`：重启会以同一 token
  领取第二个 job。runner 的 journal 输出包含 short token，属于凭据邻接材料。
- **token 会进入受保护的 Terraform 输入、plan/state、云 user-data 或 Proxmox ISO**，
  边界与 Forgejo VM 例外相同；不适合要求 token 完全不落 Terraform 的部署。

## 验证与恢复

发布后的 **Active** 只表示静态校验通过。先使用测试 Fleet，确认 clone、DHCP、cloud-init、
GitHub runner 在线、一个实际 job 成功，以及退役后 VM、ISO 和 GitHub runner 都被清理。
并发分配 VMID、错误存储、模糊模板名和创建中断也应在生产启用前验证。

provider 的每项 PVE task 等待上限为 10 分钟；大型 full clone 应先量测。
错误时保留 Shaula 的 state/Generation 恢复材料，不手动清空 state 或复用 VM/ISO 名称。
ISO、cloud-init 缓存、Terraform state 和 provider 临时目录可能保存 JIT；它们是凭据材料。
guest 临时 JIT 文件清除不代表对拥有 root/sudo 的 workflow 提供隔离。

上游接口与已知边界见 [provider NoCloud 指南](https://github.com/Indexyz/terraform-provider-proxmox/blob/v0.7.1/docs/guides/nocloud-runner-vm.md)。

# 在现有模板中选择 Forgejo

Docker、Kubernetes、Proxmox、AWS、TencentCloud、AliCloud 均复用原有 Template
Sources，没有新增 `forgejo-*` 平台种类。发布 Template 时，在 **bindings** 中选择
`runner_backend`，值为 `github`（默认）或 `forgejo`。变量表单会从模板声明
展示这个选项；也可通过 Template 发布 API 提交。

Docker bindings：

```json
{
  "docker_host": "unix:///var/run/docker.sock",
  "runner_backend": "forgejo"
}
```

Kubernetes bindings：

```json
{
  "namespace": "actions-runners",
  "kubeconfig": "/etc/shaula/kubeconfig",
  "runner_backend": "forgejo"
}
```

选择固定在不可变 Template Revision 中，**不是 Fleet 参数**。同一个 source
可分别发布 GitHub / Forgejo 两个 Profile；Fleet 的 `kind` 必须与所选 Revision
匹配。跨 backend 的 Fleet 引用会被拒绝，自动 follow 也不会把 Fleet 切到另一种
backend。旧 Generation 继续用原始 Revision / bindings 清理，不随新发布改写。

更新服务自带模板后，需要发布新 Revision（或使用 Update from default）才能
使用此选项。旧的单 backend manifest 继续保留原契约，不会被重新解释成可切换模板。

## 在 Web 中创建 Fleet

1. 打开 **Authentication → Create profile**，Runner backend 选择 **Forgejo**，填写实例 URL、scope 和 token；等待 Active。token 只写入，不会从读取面取回。轮换入口保留原实例与 scope，展示依赖 Fleet；旧 token 应保留到旧 Runner 清理完毕。
2. 使用原有 Template Source 发布 Profile，bindings 中选择 `runner_backend: forgejo`。
3. 打开 **Fleets → Create fleet**，选择 Forgejo 和对应认证 Profile；实例与 scope 来自它的 Active revision。模板放置可选 Single template、Weighted pool 或 Shared pool。单模板与 inline 成员选择器只列出 Forgejo Profile；共享池的全部成员由服务端检查 backend 和 label targets，混合 GitHub/Forgejo 的池不可用于此 Fleet。权重只决定新 Runner 使用哪份基础设施，不决定领取哪个 job；已创建 Generation 固定其成员、Revision 和 inputs，失败/重启不会重新抽取。
4. 给 Fleet 使用独立 runner name prefix、最小必要的 labels（bundled 模板用 `name:host`），workflow 的 `runs-on` 中每个名称都必须由 Runner 声明（`runs-on` 是 Runner label 名称集合的子集）。标签不要与持久 Runner 共享，避免它们消耗同一需求。
5. 从 Fleet 的 Jobs 入口查看排队/运行观测与精确 Task 终态。Forgejo 的实例、repository/job/attempt 身份独立保存；列表消失不证明完成，只有已观测 Task ID 命中仓库任务历史才确认结果，仍不建立 Verified Runner 关联。漏掉运行期、没有权限或超出查询窗口时保持 Unknown，见 [Jobs 说明](forgejo-jobs.md)。

Fleet 的 Forgejo 目标、认证 Profile key、prefix 和 labels 在创建后固定；容量与单模板可按既有准入条件更新。普通清退仍需要安全证据；统一的 `runner.max_lifetime_secs` 默认 7200 秒是可中断 Busy 任务的硬保险，不是无损 idle drain。

真实 Docker 与本地 Kind/Kubernetes 的 Shaula 全链路验收见 [证据](evidence/forgejo-followup-2026-09-22/README.md)；重复运行方式见 [harness](../scripts/forgejo-lifecycle/README.md)。VM/云平台及其它集群部署尚未完成真实验收。token scope 的最小配置和只读 Active 探测的边界见 [权限矩阵](forgejo-permissions.md)。

## Docker / Kubernetes 镜像和启动

- `runner_image` 默认 `auto`，从该 backend 的官方 digest pin 选择镜像。
  可显式选择同 backend 的有限 alias；不能通过镜像参数覆盖发布时的 backend。
- GitHub 使用官方 `actions-runner:2.337.0`，JIT / Setup Info 路径不变。
- Forgejo 使用官方 `code.forgejo.org/forgejo/runner:13.1.0`，固定 OCI index
  `sha256:c4af85fd9f0dd03788676a534781a87c71aa2c6a37737143e017eb94d4312952`
  （包含 linux amd64 / arm64）。Docker 需事先拉取对应的精确 pin。
- Forgejo 仍运行 `one-job --wait`。一次任务完成后通过精确注册缺失等证据回收，
  **不**凭进程退出或单次 `idle` 推断可以安全删除；没有增加领取代理。
- Forgejo token 由 Shaula 在 apply 后写入容器文件 / Kubernetes Secret，
  不进入 Terraform 变量、命令行或环境变量。Docker 的匿名 `/data` volume 随
  Destroy 删除；Kubernetes Pod 通过必需的 Secret key 等待安全引导。
- **Kubernetes 原始 plan/state 是凭据材料**：Destroy refresh 会从 Secret 回读
  单 Runner token，并可能保留在 plan/state/backup 中；这是操作者明确接受的边界。
  工作目录与备份必须 owner-only，不能上传原始文件，日志只用经过脱敏的 API。
  这不允许管理 token、tfvars、metadata、argv/env 或 Setup Info 携带 Runner token。
  不支持要求 token 永不进入 Terraform plan/state 的部署。

## VM：Proxmox / AWS / TencentCloud / AliCloud

同样在原有 provider bindings 中加入 `"runner_backend": "forgejo"` 即可，
其余云凭据、网络、规格选项不变。无需独立 Forgejo template kind，不需要 SSH、
回调服务或领取代理。Proxmox 仍拥有一个 VM + 专用 NoCloud ISO；云模板仍各拥有
一个实例，沿用原始 Terraform state 清理。

- 需要干净的 Ubuntu 22.04 x86_64 + cloud-init / systemd 基础镜像。Proxmox 的
  `proxmox_template_name` 默认仍为 `GitHub-Runner`，可选择另一个符合要求的 VM
  模板；Forgejo 路径不要求预装 GitHub Runner。基础镜像不能预启用 Runner 服务、
  保留注册/旧 cloud-init 状态或 Shaula startup marker；Runner 用户不可有管理权限。
- 固定下载官方 `forgejo-runner-13.1.0-linux-amd64`，SHA-256 为
  `29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376`。
  不使用自建 Runner 或 `latest`。网络必须能访问 Ubuntu 软件源、Forgejo release
  及目标 Forgejo 实例。除 git 外的 workflow 工具（例如 Node）由镜像/准备脚本提供。
- Shaula 服务端先以 `ephemeral: true` 预注册。cloud-init 只接收该 Runner 的
  UUID/URL/labels 和 **单 Runner token**；管理 token、云 provider 凭据不进入 VM。
  root `0600` 初始文件转入 `/run/shaula-forgejo/token`，目录 `0700`、文件 `0600`，
  然后以非 root 用户运行 `one-job --wait --token-url file:...`，不二次注册。
- 单次启动 marker、`Restart=no`、不 enable unit 和 `NoNewPrivileges=true` 防止
  重启复用和任务提权。准备脚本、checksum、凭据文件或 Proxmox seed 保护失败时，
  不启动 Runner。身份 JSON 不作为 shell 代码展开；token 不进入 argv/env。
- 该 VM opt-in 契约是 `shaula.forgejo-vm-cloud-init/v1`。**token 会进入受保护的
  Terraform 输入、plan/state、云 user-data 或 Proxmox ISO**。这与现有 VM JIT
  相同：base64/sensitive 不是加密，云 metadata 也可能被同一 VM 的任务读取；
  不适合要求 token 完全不落 Terraform 的部署。容器投递方式不受此例外影响。
- VM 暂不提供 container-only Setup Info；不支持 Docker/LXC backend targets。

## Ephemeral 与资源回收

[Forgejo 官方说明](https://forgejo.org/docs/v15.0/admin/actions/security/#ephemeral-runners)
明确：ephemeral 由服务端强制最多一个 job，但只在已领取任务完成或超时后删除注册；
一直没有任务的 Runner 可能永久等待。它**不负责删除 VM / Pod / 容器**。

Shaula 在精确注册缺失等可靠终结证据下销毁底层资源；普通清退不会把单次 `idle`
或进程退出当作安全依据。GitHub / Forgejo 共用服务端
[Runner 最大存活时间](runner-lifetime.md)，默认成功创建后 2 小时。
超时允许中断正在执行的任务，也用于兜底清理未领任务的等待 Runner；不是“已空闲”
的证明，且不绕过所有权/隔离校验。失败的删除保留状态继续对账，而非丢弃资源记录。

## 当前范围

Forgejo Fleet 需要独立的 `forgejo_token` 认证和显式 `:host` labels，例如
`linux:host`；workflow 使用 `runs-on: [linux]`。这里的 `host` 指一次性的 Runner
容器内或 VM 内，不是 Shaula 服务端。容器不挂载宿主 Docker socket、不支持 DinD。
Fleet UI 与 Jobs 已按 backend 分支呈现；精确 Task 结果不等于已验证的 Runner 关联。

验证分层：Rust 测试覆盖发布、backend / 镜像不匹配、follow 和生命周期；
`scripts/template-backends/plan.mjs` 使用真实 Terraform 及锁定 provider、仅读取的
本地 API fixture 生成两平台 × 两 backend 的 saved plan，并可交给 Rust plan
admission 验证。`scripts/template-backends/vm.mjs` 用真实 Terraform 计算四种 VM
的原始 user-data 表达式（移除 provider/resource 块，不访问云），覆盖两个 backend、
默认值、凭据隔离和编码大小；在私有 fixture 中执行 Bash / Python 引导、checksum、
权限及失败路径，用户切换、包安装、下载和块设备操作使用测试替身。

```sh
TERRAFORM=/path/to/terraform PYTHON=/path/to/python3 node scripts/template-backends/vm.mjs
```

这些测试不创建资源，不证明真实 cloud-init / systemd / 云平台 / 集群或完整 Forgejo
E2E 验收；已有 [one-job 协议实验](evidence/forgejo-one-job-2026-09-21/README.md)
也不替代目标环境中的 boot → job → VM/磁盘/ISO 销毁验收。

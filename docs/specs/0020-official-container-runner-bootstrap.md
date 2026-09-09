# Official container runners and external bootstrap

Status: accepted, 2026-09-09. 本规范拥有容器 Runner 的镜像来源、启动门槛与宿主侧 bootstrap 契约；理由见 [ARD-0024](../ard/0024-bootstrap-official-runner-images-outside-containers.md)。[spec 0019](0019-workflow-jobs-and-operation-logs.md) 继续拥有日志内容、受众、保留与 Jobs API；实现及真实平台验收见 [实现状态](../IMPLEMENTATION_STATUS.md)。

## 1. Scope and ownership

Docker、Kubernetes 等以容器承载 Runner 的 bundled Template **必须直接使用 GitHub 官方 `ghcr.io/actions/actions-runner` 镜像，并固定真实内容 digest**。Fleet 只能选择 manifest 中批准的官方镜像 alias，不能提供 registry、Dockerfile、启动脚本或任意镜像。不得重新打包派生 Runner 镜像，也不得在 Runner 容器安装或下载 Shaula shim、Setup Info helper、bootstrap sidecar/init container。官方 Runner 自身的 Listener、JIT 解析及 `.setup_info` 读取不属于 Shaula 容器逻辑。

Shaula 在生命周期执行端准备、过滤与交付 Setup Info；Terraform 创建资源并建立阻止 Listener 提前启动的门槛。Runner 只运行官方 `Runner.Listener run`。本契约不在容器内等待 apply、不回连日志服务、不执行 `docker exec` / `kubectl exec` 注入脚本。

生命周期执行端当前是 daemon-owned Template Runtime；未来是运行同一 Runtime 的 `shaula job` Lifecycle Worker。本决定不声称 worker CLI、独立 Executor 或 HTTP-state 生产组合已经完成，也不扩大 Fleet Reconciler 的平台对象模型。

## 2. Versioned capability and immutable inputs

新 Template manifest 声明：

```yaml
input_contract_version: 1
container_bootstrap_contract: shaula.container-bootstrap/v1
```

该能力是固定的 Runtime 协议，不是 publisher 自定义 hook。Docker/Kubernetes 的新 publication 和新 Create 必须声明此能力并只使用官方镜像；缺少能力、未知版本、平台不匹配、资源形态不匹配或同时声明旧 `setup_info_contract` 必须拒绝。原 v1 input envelope 与 `shaula_result` envelope 保持不变；capability 不放入 Fleet 参数、JIT dictionary 或 argv。资源 evidence 由 Runtime 内部适配器按本规范解释，core 仍只处理通用 Create/Destroy 结果。

每个 Generation 固定原 artifact、bindings、inputs、engine、provider lock 和 runtime policy。新 bundled source 通过正常 publication 创建新 Revision；不得重写已保留 Revision、Generation、state、image pin 或旧 conformance。旧 v1/v2 artifact 只保留 parse、recovery 与 Destroy 兼容，不能继续创建新的 Generation；旧 v2 HTTPS delivery 仅为已固定该旧契约的 retained Generation 兼容路径，不是新模板的部署要求。

## 3. One Create with an external bootstrap stage

Create 的顺序固定为：

1. 按原契约锁定 Workspace、执行 init、获取 JIT、冻结输入，检查唯一 create-only saved plan。通用 action/shape admission 通过后、持久化 `ApplyStarting` 之前，还须执行本显式 container bootstrap 契约的前置检查：关键属性必须已知且符合下述启动门槛，不能把未知值当作安全默认值。
2. 获准后仅执行一次 Terraform apply。Terraform 创建尚不能启动 Listener 的资源。
3. apply 结束且 pipes 排空后，完成有界日志归档；验证标准 output、Generation/bindings、原始 state 与资源 evidence。
4. 从本 Generation 本次 Create invocation 的归档读取批准的 apply 投影，在宿主侧生成 Setup Info JSON。归档缺失、withheld、disabled、超限或处理失败降级为 `[]` 或固定 unavailable 标记；不得恢复 raw 输出或读取其他 invocation/Generation。
5. 在仍持有执行 ownership、进程 fence 与本次 effect admission 的前提下，完成平台固定的“交付并开启启动门槛”。身份不符或必需的启动/Secret publish 失败、权限不足、超时或结果不确定均作为 Create 失败进入原 CleanupRequired 路径。Docker 的可选诊断文件 copy 失败可独立降级，不能跳过随后身份复验和 start。
6. 由 GitHub inventory 判断 Runner online/Busy；平台 start、Pod phase 或 Setup Info 写入成功都不是 readiness。

前置检查必须证明 Docker plan 为 `start=false`、直接官方 Listener、唯一批准的 native JIT 输入，且实际解析 image 对应批准的官方 pin；Kubernetes plan 必须为 `Pending` target、required 且缺少 `.setup_info` 的 Secret item、直接官方 Listener 和 native JIT Secret 引用，`data` / `binary_data` 均不能预填或覆盖该启动门槛。错误模板可能在 apply 内就领取 job，因此这些检查不能推迟到 apply 后。它是 `container_bootstrap_contract` 的固定附加检查，不扩展通用 plan policy 或允许新 mutation；apply 后的 exact identity、live gate 与 ownership 复验则用于确认真正创建的资源仍符合契约。

步骤 5 是同一次 Create 的有界 bootstrap，不是新 Update API、任意平台 hook、第二次 apply 或 drift repair。副作用前必须以一次 durable transition 获得当前 bootstrap admission（当前实现为 `BootstrapStarting`），原子验证 Generation、operation 与 ownership/fence；失去 ownership 的旧执行者不能启动或发布。进程崩溃后该已可能执行事实保留，不能因尚未记录成功就重新启动 bootstrap。失败后不得自动重复 start/patch，不得重新 apply；cleanup/recovery 仍通过原始 state、GitHub safe removal 与 delete-only Destroy。保留诊断、原始 inputs/state 和 occupancy，不能以日志失败或 child 退出证明资源不存在。

**日志内容失败与启动门槛失败必须分开。** 内容失败可交付空数组或明确 unavailable 标记后继续。Docker 文件准备/copy 失败可跳过诊断文件，复验 exact identity/stopped 状态后仍须正常 start；Kubernetes 必须成功发布 required key 才能开启门槛，因此 Secret patch 失败不能被当作日志缺失忽略。身份或必需启动失败不得假报 Create 成功。有限 deadline 覆盖 archive 读取和平台子进程，stdout/stderr 与错误不得泄漏 JIT、Secret、bindings 或全文平台响应。

## 4. Docker contract

Terraform 仍仅管理一个 `docker_container`，使用 `start = false`、`must_run = false`、`restart = "no"`、`rm = false`；镜像为批准的官方 digest。容器 command 直接指向 `/home/runner/bin/Runner.Listener run`，不覆盖为 shell/helper。资源初始 env 只通过官方 `ACTIONS_RUNNER_INPUT_JITCONFIG` 交付 JIT。

Runtime 通过宿主上固定的 Docker CLI 与该 Revision 的批准 Unix socket 工作：先用标准 output/state 的 exact container ID 检查身份、Generation/Fleet labels、官方 image 与未启动状态，再尝试把宿主生成的诊断文件复制到停止容器的 `/home/runner/.setup_info`，随后重新验证 exact identity 与 stopped 状态，最后启动这个 exact ID。可选文件准备/copy 失败不阻止正常 start；copy 最多使用 5 秒并必须为后续身份复验和 start 保留 30 秒预算，剩余预算不足时跳过 copy。不能从名称扫描或重建目标，不能启动已运行、已消费或不属于本 Generation 的容器，不能启动后再补文件。CLI argv 不含 JIT、日志正文或凭据；正文经受保护的宿主文件交付，临时文件按有限生命周期清理。

Docker container 配置与 inspect/state 会保留初始 JIT env，这明确修订旧的 declarative-env 禁令。它们均为 credential-grade；同 Runner Execution Domain 的初始环境/进程内存检查风险仍被接受。官方 Listener 捕获并移除普通环境项，验收须证明 job 子进程不继承 JIT；不得据此宣称内存清零或平台管理员不可见。

## 5. Kubernetes contract

Terraform 仍仅管理 generation-scoped Pod + Secret，namespace 与 RBAC 在外部预建。Secret 初始为 `immutable: false`，只包含 `jit_config`；Pod command 直接运行官方 Listener，JIT 使用 `secretKeyRef` 进入 `ACTIONS_RUNNER_INPUT_JITCONFIG`，不把明文嵌入 Pod env value/args/metadata。

Pod 通过 required Secret volume item 引用初始尚不存在的 `.setup_info` key，并将该唯一 key 以只读 `subPath` 挂到 `/home/runner/.setup_info`。缺 key 必须阻止 runner container 启动。Pod 不使用 init/sidecar、内存 staged JIT 副本或容器轮询。Terraform provider 的 create target 为 `Pending`，不能等待 Running/Ready/Listener online，避免与 apply 完成互相等待。

完成 apply 后，Runtime 通过宿主固定 kubectl CLI、原 Revision 的 kubeconfig/namespace 和 output/state 中 exact Pod/Secret identity 读取目标。必须核对两者 UID、名称/namespace、Generation/Fleet labels、Pod gate 和未启动状态；`incarnation` 对该契约必须是 UID，不能用 resourceVersion 冒充。随后对这个 Secret 发出一次 JSON Patch：`test` exact UID 与已读取 resourceVersion，再添加 `.setup_info` 并设置 `immutable: true`。失败或冲突不无条件重试，不替换 JIT，不更新 Pod，不放开未知对象。

完整 JSON 在第一次 main-container 启动之前存在，Secret 投影到达时间不等于 apply 完成时间。`.setup_info` subPath 是只读快照；本 Generation 不更新它。Secret 的 JIT key 不挂到 runner 文件系统，只被官方启动环境引用。部署权限增加的是宿主对 exact namespace Pod/Secret 的读取和 Secret patch，不能向 Pod 授予 API token、kubeconfig、ServiceAccount 权限或 provider credential。

此 UID/resourceVersion 检查仅属于 bootstrap publish，**不改变** Terraform provider 按 namespace/name Destroy 的既有风险与保留规则。Destroy refresh 可以观察 bootstrap 后的 Secret 数据/immutable 值，但不保存新的 desired tfvars，不应用普通 Update plan，也不丢弃原 state。

## 6. Setup Info content and trust

使用 JSON serializer 生成 UTF-8 数组，条目为 `{"Group":"Terraform apply (runner provisioning)","Detail":"..."}`；Group 不以 `_internal_` 开头。正文仅来自 spec 0019 批准的 Create apply 投影，保留明确的截断/缺失事实。Destroy 日志只进入独立归档与 Jobs UI。日志处理始终在宿主执行端，不依赖 Runner 网络、Python、shell 或可写 helper。

此容器契约的完整 Setup Info JSON 硬上限为 **768 KiB**，进一步收紧 spec 0019 的通用日志投影上限。JIT 没有沿用旧 shim 的 64 KiB 限制；Kubernetes 发布前必须按实际 `jit_config` 字节数加 Setup Info JSON 字节数检查 Secret **1 MiB decoded data** 总上限，不能用固定预留假设代替。合计超限时将诊断内容降级为 `[]` 再检查；JIT 加最小 `[]` 仍超限则必需 bootstrap 失败，进入 CleanupRequired，不提交超限 Secret。bundled 官方镜像的固定 root 为 `/home/runner`。官方 image 中若将来包含已有 `.setup_info`，升级模板前必须明确其合并/保留策略并重新验收，不能继续假定可覆盖。新契约不授予 Runner 任何日志读取 capability、OIDC token 或 state/control 权限。

Docker/kubectl 是执行主机的受信依赖，平台凭据只可供精确 Revision 的 Terraform 与 bootstrap 子进程使用。不得转发 daemon 整个环境或允许 Fleet 选择 executable、args、path、kubectl context。固定 CLI 版本、行为、权限和 process fencing 纳入 runtime policy 与 exact-tuple 验收。

## 7. Acceptance

- 静态 admission 拒绝非官方/未固定镜像、旧与新能力混用、任意 hook 及错误平台/shape；default source 中没有 Dockerfile、shim、helper/init/sidecar。
- Create plan 在 `ApplyStarting` 前拒绝关键属性 unknown、Docker 提前 start、非官方实际 image、错误 Listener/JIT，以及 Kubernetes 错误 target 或经 `data`/`binary_data` 预填/覆盖 `.setup_info` 门槛；拒绝时没有 apply 副作用。
- 慢 apply 时 Docker 保持 stopped，Kubernetes main container 保持未启动；apply/output/state 验证完成前 Listener 不可能领取 job。
- 真实 Docker 与 Kubernetes 第一个 GitHub job 的 Set up job 显示本 invocation 的批准 apply 组；销毁后 UI 仍可读独立 apply/destroy 档案。
- 验证 Setup Info 的 768 KiB 上限及 Kubernetes 实际 JIT+JSON 的 1 MiB 总量边界：诊断超限退化为 `[]`，连最小数组也无法容纳时必需 bootstrap 失败。
- 无归档、withheld、正文超限及内容读取失败交付 `[]`/unavailable 后正常启动，Docker 文件准备或 copy 失败也不吞掉正常 start；identity mismatch、同名不同 UID、resourceVersion 冲突、权限失败、patch/start timeout 进入 CleanupRequired，无错误目标 mutation、无重复 Create。
- 在 apply 完成、交付前后、start/patch 请求不确定等边界注入 crash/cancel/admission 撤销，验证 ownership/fencing、原 state 保留与安全 Destroy。
- 验证官方 Listener 的 JIT 消费、普通 job 环境缺失 JIT、没有 Runner API/provider/management 凭据及 argv/log 泄漏；不声称同域 process inspection 隔离。
- 新 Revision 与旧 v1/v2 retained Generation 并存；旧 artifact/input/state 不变，旧协议清理可用，但新 publication/Create 拒绝旧 custom-image 或缺少宿主能力的模板。单元测试或静态文档不能替代 exact provider/image/host CLI/GitHub 的真实验收。

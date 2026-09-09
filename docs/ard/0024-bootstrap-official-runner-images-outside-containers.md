---
status: accepted
date: 2026-09-09
amends: [0004, 0006, 0008, 0009, 0014, 0023]
---

# Bootstrap official Runner images outside containers

本决定将容器 Runner 的 bootstrap 放到 Shaula 生命周期执行端与 Terraform 层。协议与验收的唯一 owner 是 [spec 0020](../specs/0020-official-container-runner-bootstrap.md)；日志与 Jobs 模型仍归 [spec 0019](../specs/0019-workflow-jobs-and-operation-logs.md)。实现证据见 [实现状态](../IMPLEMENTATION_STATUS.md)。

## Motivation

用户要求 Docker、Kubernetes 等容器部署直接使用 GitHub 官方 Runner 镜像。为 Setup Info 重新构建 shim 镜像会引入额外发布、镜像分发和更新负担，把 Shaula 生命周期职责留在 workflow 执行环境。apply 日志只有 apply 结束后才完整，若容器已直接启动 Listener，再补文件会错过首个 job 的 setup 阶段。

因此需要明确保留“apply 完成后、Listener 启动前”这一交付顺序，同时取消容器下载、等待和文件处理逻辑。官方 Runner 自身按其原有机制接收 JIT 并读取 `.setup_info`。

## Decision

所有新 bundled 容器模板直接 pin `ghcr.io/actions/actions-runner` 官方 digest，不重新打包 Runner，也不注入 shim、helper 或 bootstrap init/sidecar。JIT 使用官方 `ACTIONS_RUNNER_INPUT_JITCONFIG` 启动输入，Listener 自身解析并删除普通环境项。此处明确修订 ARD-0004 的 JIT declarative-env 禁令：Docker 初始 container env 保留 JIT；Kubernetes 以 Secret 引用传值。它们和 Terraform state/inputs 均是 credential-grade。环境 unset 仍不表示内存清零或同域进程隔离。

Terraform 创建带启动门槛的资源。Docker 使用 stopped container；Kubernetes 使用缺少 required `.setup_info` key 的 Secret volume，provider 只等待 Pending。Shaula 在同次 Create apply 完成、输出/state 验证与有界归档后，在宿主侧生成批准的 Setup Info，然后完成固定的平台 bootstrap：Docker 向停止容器复制文件并启动 exact ID；Kubernetes 核对 Pod/Secret UID 与 ownership 后，以 UID/resourceVersion 条件 patch Secret，发布 `.setup_info` 并冻结 Secret。Runner 容器本身没有等待脚本或额外网络请求。

通用 create-only plan admission 后、`ApplyStarting` 前，Runtime 必须额外证明本显式 bootstrap 契约的关键已知属性：Docker stopped/direct Listener/native JIT/实际官方 image，以及 Kubernetes Pending/required missing Setup Info gate/direct Listener/native JIT；Secret `binary_data` 也不得预填或绕过门槛。否则错误模板会在 apply 内先启动 job，事后发现已无法补救。此检查仅属于固定容器能力，不改变通用 plan policy；apply 后 live 身份与门槛复验验证实际资源仍相符。

这明确修订 ARD-0008 中“运行时绝不调用平台工具”的绝对限制，及 ARD-0006 中 init-to-memory / 初始 immutable Secret 的交付方式。平台逻辑封装在 Template Runtime 的固定能力实现里；Fleet Reconciler 与 core 不增加 Pod/container 状态机，也不接受任意平台脚本或 lifecycle hook。宿主 docker/kubectl CLI 与相应平台权限成为该能力的受信依赖。

bootstrap 是已获准 Create 的固定尾部，不引入 Update Operation、第二次 apply、运行中修复或自动重试。副作用前再次检查执行 admission，失去 ownership 或遇到身份冲突时拒绝；不确定的 start/publish 结果进入原清理路径。安全移除、delete-only Destroy、original-state/inputs、process fencing 与 Occupancy 不变。Kubernetes bootstrap 使用 UID 条件不等于 Terraform Destroy 已具备 UID 条件。

Setup Info 直接读取本次调用的持久批准投影，不需要容器 bearer capability 或独立 HTTPS origin。内容处理失败退化为空数组，不把日志可用性变成 Runner 的网络依赖；Docker 诊断 copy 失败可跳过文件，随后仍复验 identity 并正常 start；Kubernetes required Secret publish 失败以及任何无法安全识别或启动正确资源的情况属于 Create 失败，不能伪装成仅日志缺失。完整 apply/destroy 档案仍独立保留并由 Jobs UI 读取。

## Compatibility and tradeoffs

`container_bootstrap_contract: shaula.container-bootstrap/v1` 是新 immutable Revision 的显式能力，继续使用 v1 input。新 Docker/Kubernetes publication/Create 必须使用该能力及官方 pin；不得和旧 `setup_info_contract` 叠加，也不能悄悄升级旧 Generation。旧 custom-image 只保留 parse/recovery/Destroy，不能再用于新的 Generation。ARD-0023 的容器 HTTPS/shim 决定仅保留为已固定旧 v2 artifact 的 retained Generation 兼容历史；其 Jobs、日志保留及受众隔离决定继续有效。

宿主侧平台能力增加了受信代码和部署依赖，但消除了自制 Runner 镜像及容器内控制逻辑。Kubernetes 接受短暂可变的 generation Secret，在唯一批准 publish 后转 immutable；该特定 mutation 不能推广成通用 Secret 更新。发布按实际 JIT 与诊断 JSON 的合计字节校验 Secret 容量，诊断可降级为空数组；不假设已删除的 shim 仍为 JIT 提供固定大小限制。Docker 接受初始 declarative env 中 JIT 的保留风险，平台管理员本就位于基础设施凭据边界内；普通 workflow env 无 JIT 仍需验收。

不采用第二次 apply，因为 Create 只能 apply 一次；不在 apply 后向已经运行的 Listener 补文件，因为首 job 可能已初始化；不在 init container 等 apply，因为会阻塞 provider 的启动等待。将完整日志放进初始 tfvars 也不可行，日志当时尚未产生，且不应回写冻结输入。通过显式启动门槛解决顺序，不依赖定时 sleep 或 GitHub 领取速度。

当前执行端仍是 daemon-owned Template Runtime。未来 `shaula job` 接手同一能力，不以本决定宣称完整 worker/HTTP-state 组合已上线。新模板必须经过静态 admission、宿主依赖检查与各平台真实首 job 验收；旧 conformance 不能代表这个新 tuple。

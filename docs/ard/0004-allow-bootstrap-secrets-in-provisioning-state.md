---
status: accepted
date: 2026-09-04
---

# Allow bootstrap secrets in provisioning state and generation resources

> **Bootstrap amendment:** [ARD-0024](0024-bootstrap-official-runner-images-outside-containers.md) supersedes the original container handoff/platform-tool clauses below for new official-image Templates. Docker uses native JIT env and stopped-container copy/start; Kubernetes uses a Secret env reference and conditional host publication before startup. No custom shim/init image is used. The fixed Runtime bootstrap may use host platform tools and exact Revision credentials; no general Update or core platform API is added. The original clauses below describe retained v1/v2 artifacts only. Credential-grade state, no Runner provider/control-plane credentials, and original-state Destroy remain current.

> State ownership is amended by [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md): authoritative state is SQLite-backed HTTP state; DB/WAL/backups and unresolved emergency local state share this credential boundary. Worker control is not a Runner/workflow secret broker.

Shaula v1 直接通过不可变 Template Profile Revision 向 Runner Resource 传递 GitHub JIT bootstrap payload，并接受该 payload 进入 per-runner IaC state、exact original protected input 与 generation-scoped bootstrap carrier。原始受保护输入必须保留到 Destroy 成功且 state 为空后才可清理。当前不引入 Runner 回连 Shaula 的一次性 secret broker；JIT 是一次性 Runner Registration 材料，不是 GitHub App 或 PAT 控制面凭据。

## Consequences

- IaC state、saved plan、Runner Workspace、受保护输入、备份、bootstrap carrier 和交接文件全都是 credential-grade data；JIT 的短有效期不降低访问控制与保留要求。
- Kubernetes Template 只让 init container 只读挂载 Secret，并将文件复制到 `medium: Memory` 的 `emptyDir`；只读 Secret source 由 Pod/Secret Destroy 清理。runner container 只挂载内存卷，bootstrap shim 读取并 unlink staged copy。
- Docker Template 使用 `docker_container.upload.content = var.shaula.jit_config` 写入固定文件；不使用 host source-file path，bootstrap shim 在 Runner 启动前读取并 unlink。
- bundled Profile 禁止 secret-bearing argv，也禁止 declarative Pod/container env、args 或 command 携带 JIT。shim 在 spawn pinned `Runner.Listener` 时只设置 `ACTIONS_RUNNER_INPUT_JITCONFIG`；Runner `CommandSettings` startup 将其捕获到私有内存 map 并 unset ordinary environment entry，`GetJitConfig()` 随后读取该副本。
- Environment unset 不构成 `/proc` 或 memory isolation，因为 Linux 可保留 initial exec environment。v1 明确接受同一 Runner Execution Domain 内、具备进程检查能力的 workflow 可能读取 `Runner.Listener` initial environment/memory 中 JIT 的风险；这不阻止 Profile activation。Conformance 仍须证明 JIT 不在 process argv、声明式资源配置、普通 job environment、workflow context、已消费的 staged path、HTTP read、audit、日志、诊断或 telemetry 中，但不声称 process isolation 或 memory zeroization。
- 平台特定的交付和清理全部属于 Template Artifact；Shaula 不为 Kubernetes Secret、Pod 或 Docker container 实现原生 client 路径。
- GitHub App private key、installation/admin token、PAT 和 derived control-plane token 不得进入 IaC template、Runner Resource 或 workflow；平台管理凭据与 sensitive Template bindings 只进入对应 exact-Revision IaC subprocess，且不得进入 Runner Execution Domain。Shaula HTTP/SQLite credentials 同样不得进入该域。
- workflow 可见的 per-job `GITHUB_TOKEN`，以及 workflow 作者主动注入的 `${{ secrets.* }}`，不改变上述控制面隔离要求。
- JIT payload 和相关 secret-bearing data 不得进入 CLI 参数、HTTP 响应、日志、metrics、trace attributes、普通错误或未脱敏诊断包。
- 未来改为一次性拉取会改变 Template Runtime Interface，需要显式迁移和新的 ADR。

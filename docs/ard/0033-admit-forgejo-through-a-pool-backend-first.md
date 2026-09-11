---
status: proposed
date: 2026-09-11
---

# Admit Forgejo through a pool backend first

决定引入第二个 Runner Backend（Forgejo），但**先交付 Pool 切片**：与 GitHub 控制面并列的第二条
reconcile driver + 独立端口，复用既有 Generation 状态机、worker fencing、state backend 与模板引擎；
等切片在真实实例上验收通过后，再演进到 provider-neutral 的单一路径。协议与验收统一维护在
[spec 0026](../specs/0026-forgejo-runner-backend.md)；理由与代价记录在此。

## Context

Forgejo ≥ 15 与 forgejo-runner v13 已具备承载 Shaula 核心模型的能力，且都能在源码中定位：
`POST …/actions/runners` 立即落库并返回一次性 `uuid`/`token`（`routers/api/v1/shared/runners.go:151`、
`models/actions/runner.go:350`），inventory 暴露 `offline`/`idle`/`active` 与 labels、`ephemeral`
（`services/convert/convert.go:548`），需求可从 jobs 列表读取（`shared/runners.go:52`），
ephemeral 由服务端强制并在任务结束后删除注册（`services/actions/task.go:37,262`、
`services/actions/cleanup.go:144`），runner 侧有 `one-job --wait`（runner `internal/app/cmd/cmd.go:65`）。

同时源码也给出了三条**无法绕过**的缺口：公开 API 不暴露 task → runner
（`services/convert/convert.go:209` 只返回 job 字段），因此 job ↔ Generation 的可靠绑定只有
`--handle` 定向一条路；不存在 message queue / session，需求与观测只能轮询；`DELETE runner`
没有"job 正在运行"保护（`shared/runners.go:186` → `models/actions/runner.go:330` 直接删），
GitHub 侧的 `JobStillRunning` 分类没有远端依据。

Shaula 的当前契约把 `github.com` 写进产品边界（spec 0001 §3 明确排除 GHES 与通用平台抽象），
端口 `GitHubAccessPort`（`crates/shaula-core/src/ports.rs:207`）与 wire 协议
（`crates/shaula-scaleset/src/wire.rs`）都是 scale set 语义，模板引导也写死
`ACTIONS_RUNNER_INPUT_JITCONFIG`（`templates/*/…`）与官方 runner 镜像白名单
（`crates/shaula-core/src/template_image.rs:26`）。耦合面约占 workspace 的一半代码
（229/469 个 .rs 文件出现 github/scale set 术语）。

## Decision

1. **先做 C（Pool 切片）**：Forgejo Fleet 走第二条 driver + 独立 adapter crate + 独立端口；
   本切片不传 `--handle`、不产生 `Verified` 关联、不实现 Auth Handoff、不合并存储列。
   `GitHubAccessPort` 与 `shaula-scaleset` 保持 GitHub 专用，GitHub 路径行为零变化。
2. **C 阶段就把 A 的不变量写进契约**：覆盖式需求快照、inventory 驱动 readiness、
   一 Generation ↔ 一个可证 runner 身份、不确定性按精确名查找分类、Busy 判定不依赖远端保护、
   provider 只影响控制面与引导材料。这些不变量是后续归一化的前提。
3. **代价显式化，不用文档掩盖**：C 下 job ↔ Generation 只能是 `Unverified`/`Ambiguous`；
   空闲 Generation 持续占用基础设施（Forgejo 无预分配，等待 runner 就是成本）；
   凭据轮换 = 收敛后重建而非无缝切换；镜像准入需要新增官方 runner 镜像族。
4. **验收先行**：spec 0026 §10 的 A1–A8 在真实 Forgejo ≥ 15 实例上跑通后，才讨论 A 的端口归一。

## Alternatives considered

- **直接做 A（provider 抽象）**：一次到位，但要在没有真实 Forgejo 反馈的情况下先重构
  `GitHubAccessPort`、Fleet Spec、存储列、模板 envelope 与 manifest 校验，同时冒着碰坏已经在
  生产使用的 GitHub 路径的风险；并且 handle 定向、Auth Handoff 等价物等 A 级语义的取舍恰好需要
  真实实例才能确定。
- **把 Forgejo 语义塞进 `GitHubAccessPort`**：会产生 "scale set id = null" 这类假字段和两套失败
  分类混用，违反 spec 0007 的依赖方向，且让 GitHub 侧的证明（route proof、installation binding）
  失去唯一含义。拒绝。
- **完全独立的 Forgejo 子系统（含独立表与 UI）**：污染最小，但会复制容量算术、生命周期状态机与
  UI 外壳，长期维护两份收敛语义；与"一个 daemon 管多个 Fleet"的既有基线冲突。作为 C 的实现形态
  被拒绝（C 是第二条 driver，不是第二个 daemon）。
- **维持 GitHub-only**：把项目定位收窄到 github.com；代价是放弃自托管 CI 场景。这个选项由 owner
  决定；本 ARD 记录的是"若要支持"的路径，不主张必须支持。

## Consequences

- 短期内存在两条 reconcile driver 与两个 adapter，测试与文档需要分别覆盖；归一化留到 A。
- Jobs 视图在 Forgejo Fleet 上能力弱于 GitHub：只有未验证关联、链接需要一次有界读取（run index），
  失败时只能展示名称与状态。
- 需求信号依赖轮询，规模边界未知（该端点无分页、labels 过滤在服务端内存执行），必须先做 spike 与
  上限（D6），否则不得宣称可用于大实例。
- 模板与镜像准入扩展后，`runner_image_digests` 的"官方镜像族"校验需要按 bootstrap kind 分支；
  这是 C 阶段唯一必须触碰 core 模板校验的地方。
- 凭据模型新增 token 型 profile kind，与 spec 0018 的"仅 GitHub App"不冲突但必须显式区分：
  Forgejo token 不是 PAT 的复活，不得作为 GitHub 凭据形态回到系统里。

## Rollout

1. spec 0026 接受（本 ARD 从 `proposed` 转 `accepted`）与 D5/D6/R4 的冻结项确定。
2. Spike：A1–A3 在本地 docker Forgejo 上跑通，确认 labels 匹配规则与轮询成本。
3. 实现 C：adapter crate、Fleet provider 段、token profile、模板引导材料与镜像准入、Jobs 未验证关联。
4. A1–A8 真实实例验收，证据写入 [实现状态](../IMPLEMENTATION_STATUS.md)。
5. 再评估 A：端口归一、handle 定向、存储列合并——不因 C 已上线而自动获得授权。

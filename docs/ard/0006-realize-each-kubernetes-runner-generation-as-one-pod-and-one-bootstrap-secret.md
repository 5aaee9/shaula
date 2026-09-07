---
status: accepted
date: 2026-09-04
---

# Realize each Kubernetes Runner Generation as one Pod and one bootstrap Secret

Shaula v1 的 Kubernetes Template Profile 为每个 Runner Generation 只管理一个短生命周期 Pod 和一个不可变 JIT bootstrap Secret，并将它们放入平台预先创建的 namespace。具备 `template.publish` 权限的用户通过 Template Profile binding 选择 namespace；模板不管理 Namespace、ServiceAccount 或 RBAC。Pod 设置 `automountServiceAccountToken: false`，且 Runner 不获得 Kubernetes API credential。

artifact manifest 独占声明 `platform: kubernetes`、`bindings_contract: shaula.bindings.kubernetes/v1` 与 `schemas/bindings.schema.json`，输入和输出以 wire name 为 `bindings_digest` 的 exact opaque commitment 绑定该 Revision；该值不得是 sensitive plaintext 的 unkeyed digest/offline verifier。Shaula core 只处理通用 Template Runtime 的 inputs、opaque outputs、state 与 Create/Destroy 结果，不包含 Kubernetes client、watch、object model、admission inspection 或专用 reconcile path。

## Consequences

- Kubernetes provider credential 与 namespace 内权限由平台在 Shaula 之外预置；credential 只进入对应 IaC subprocess，与 Runner Pod 身份完全分离。
- namespace 存在性由普通 provider data source 读取，并由 managed resource 的 `lifecycle.precondition` 阻断 planning；standalone Terraform `check` 仅是 advisory，不能充当安全门。
- Pod 使用 `restartPolicy: Never`。Shaula 在 JIT/IaC 前持久化 stable、collision-resistant 且跨 Generation 永不复用的 `generation_name`；bundled Profile 将该值直接作为 Pod 与 Secret 的 exact `metadata.name`，两者以 `(target binding, namespace name, kind, metadata.name)` 作为逻辑 Resource Key。Normalization/truncation collision 必须在 JIT 或外部 mutation 前 fail closed。
- 只有 init container 可只读挂载 Secret；它把 JIT 复制到 `medium: Memory` 的 `emptyDir`，只读 Secret source 保留到 Pod/Secret Destroy。runner container 只挂载该内存卷，直接挂载 Secret 被禁止。
- pinned bootstrap shim 读取并 unlink staged file，不传 secret argv；它在 spawn pinned `Runner.Listener` 时只设置 `ACTIONS_RUNNER_INPUT_JITCONFIG`，Runner startup 捕获后 unset ordinary env entry。v1 接受同一 Runner Execution Domain 中的 process-inspection 暴露风险；验收证明 ordinary job environment、workflow context、argv、staged path、日志和 telemetry 无 JIT，但不把 unset/unlink 解释为 process isolation 或 memory zeroization。
- IaC dependency graph 必须保证 Secret 先于 Pod 创建，并在 GitHub 安全注销后先销毁 Pod、再销毁 Secret。
- saved-plan policy、worker provenance、输入保留与错误分类沿用 spec 0004；经 [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md) 修订，worker/HTTP state/lock/fencing 以 spec 0010 为准，不再要求 daemon 的逐命令 starting subphase。template 错误仍仅暴露有限 reason/phase。
- v1 接受 HashiCorp Kubernetes provider 根据 Terraform state 中的 namespace/name 删除 Pod 与 Secret；不要求 Kubernetes UID precondition。`metadata.name` 只是上述逻辑 Resource Key 的一部分，不是 `metadata.uid`。Profile attestation 必须固定 provider、命名算法、bindings/namespace 与测试套件，并证明同 Generation 恢复保持 exact name、不同 Generation 不复用/碰撞同一 key、Create collision 不 adopt/import/Update/delete、Destroy 只使用原始 state。
- 这是 trusted target/namespace/name-reservation 决策：在关联 Generation 完成 Destroy 前，target binding 不得重指、namespace 不得删除后同名重建，所有可写该 namespace 的 actor/controller 也不得删除并同名重建 Shaula 保留对象。若任一假设被破坏，provider 可能删除 same-name different-UID replacement，包括重建 namespace 中的对象；这是明确接受的 v1 residual risk，不得宣称 UID-safe。
- Runner Resource 不包含 GitHub App private key、PAT 或 Kubernetes provider credential。job 运行时的 `GITHUB_TOKEN` 或显式 Actions Secret 属于 Workflow Credential。
- missing namespace、identity collision、已知 replacement drift 或 unsafe admission mutation 必须通过 IaC failure、declared evidence 或外部验收暴露；不得触发 Terraform Update 或同 Generation 的修复性 re-apply。Create collision 必须失败，不能 adopt、import、delete 或接管未知对象；不能识别的完全同名 replacement 受上述 residual-risk 条款约束。
- Shaula 不直接查询 Pod/Secret 来宣称成功或销毁完成；运行时判定只能使用通用 IaC result/state/evidence，真实平台形态由外部验收测试覆盖。
- Kubernetes Secret、IaC state、Runner Workspace、saved plan 与 exact original protected input 持续按 credential-grade data 保护，直到 successful Destroy 与 empty-state proof。

详细契约见 [Kubernetes Runner Resource Specification](../specs/0003-kubernetes-runner-resource.md)。

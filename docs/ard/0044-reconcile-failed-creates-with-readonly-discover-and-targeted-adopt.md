---
status: accepted
date: 2026-11-22
---

# Reconcile failed Creates with read-only discover and targeted adopt

## 决定

在保持内核零 provider 分支、绝不重新 `apply` Create、且未声明能力的模板行为完全不变的前提下，将 [ARD-0040](./0040-destroy-generations-that-never-started-a-create-apply.md) 的「可证明就不要 quarantine」原则向 `ApplyStarting` 之后的失败 Create 延伸为两条分层路径：

1. **阶段 1（全模板通用的锚定与锁卫生）**：
   - `generation_anchor_failed_create` 返回结构化的 `AnchorOutcome`（`Anchored` 或带明确原因的 `Skipped(AnchorSkipReason)`：`NotStarted` / `RevokedOrSealed` / `LockHeld` / `NoSnapshot` / `ManifestUnavailable` / `ShapeIncomplete`），取代原先不透明的布尔返回值与含糊日志。
   - 非 `PlanFailed` 的 Create 失败在返回调用方前由 `Workers::create` 立即 fence 该 worker，使进程树确定清空且残留锁可在后续恢复中受审计清理。
   - `RunnerOperation::retire` 在 `destroy_proof` 发现 `generation_state_identity` 缺失时，先重试一次 `generation_anchor_failed_create`；若 worker 已 fenced 且 state 覆盖完整 shape，直接锚定并进入既有 Destroy，不再被瞬时锁状态误判。

2. **阶段 2（可选的纯只读 `discover` + 针对性 `adopt`）**：
   - 模板 `profile.yaml` 可选声明 `reconcile` 节，指向**同一 artifact 内**的只读探测子模块及可选的最终一致性等待窗口：
     ```yaml
     reconcile:
       discover_module: reconcile
       settle_delay_secs: 65   # 可选，默认 0，上限 600（用于 Proxmox RRD 等延迟视图）
     ```
   - **双重只读锁**：
     - 发布/加载阶段静态校验 `discover_module`：仅允许常规 `.tf` 文件，经 HCL 语法深度守卫解析后只允许 `terraform`、`provider`、`variable`、`locals`、`data`、`output` 顶层块，严格拒绝任何 `resource`、`module`、`import`、`moved`、`removed`、`ephemeral`、`check`、`.tf.json` 或 override 文件，并要求声明 `variable "shaula"` 与 `output "shaula_discover"`。
     - 运行阶段在隔离临时目录执行 `terraform init -backend=false -lockfile=readonly` 与 `terraform plan -out=discover.tfplan` + `terraform show -json`，由 `admit_discover_plan` 再次校验零 managed 变更且 data source 仅 `read`/`no-op`，完全不接触 HTTP state backend。
   - **结构化输出契约 `shaula_discover`**：
     - 必须包含 `contract_version = 1`、`generation_id == generation.id`，且 `roles` 与 `managed_resource_shape` 一一精确覆盖（禁止漏报、重复或未声明类型），每个 role 的发现数不得超过 `exact_count`，每个条目的 `address` 必须匹配声明的 `terraform_type`，`import_id` 为可选非空无控制字符且不以 `-` 开头的字符串（`null` 表示该 provider 资源不支持 `ImportState`）。
   - **三分支处置状态机（在 worker 已 fenced 且满足 `settle_delay_secs` 后执行）**：
     - **远端为空且 state 为空**：`discover` 证明全部声明 role 在远端均不存在 → 原子记录 `FailedCreateIdentityJson` 与 `cleanup_revision` → 直接推进到 `Destroyed` 并封存 state，不执行 `import` 或 `destroy`。
     - **远端资源全部已在 state 中，或所有未入库远端资源都提供了 `import_id`**：仅对 state 中缺失的远端资源执行 `terraform import` 并复核 `state pull` 覆盖全部发现地址 → 锚定 `(lineage, serial)` → 复用既有 `destroy_resources`（含 `admit_destroy_plan_json` 与 `admit_destroy_identity`）清理至空 state 并推进到 `Destroyed`。
     - **任一不可证明或高风险情形一律 fail-closed**：未声明 `reconcile`、存在 `import_id == null` 的未入库远端资源、发现数超过 `exact_count`、`shaula_discover` 契约校验失败均直接 `Quarantined`；`discover` / `import` 的 provider 侧瞬时执行故障（5xx、超时、限流）按既有 `DestroyBackoff` 退避重试，绝不将探测失败当成「远端不存在」。

## 理由

- 生产事故（#9）显示 49/49 个失败 Create 均为 provider 5xx 抖动（44 个在第一个资源失败、state 为空；5 个在第二个资源失败、state 仅含第一个资源），全部因 `records_whole_shape` 无法通过与残留锁时序进入 `Quarantined` 并耗尽 `create_limit`。
- `discover` 模块与主模板同属一个不可变 artifact，共享 digest、`.terraform.lock.hcl` 与 attestation，避免了独立 artifact 的版本漂移与信任链分叉，并复用既有受保护 `shaula.tfvars.json` 交付路径。
- 对 Proxmox（`indexyz/proxmox 0.7.1`）：`proxmox_storage_contents` 可列出 ISO、`proxmox_qemu_vms` 可列出 VM；`proxmox_nocloud_iso` 无 `ImportState` 而 `proxmox_qemu_vm` 支持 `node/vmid` 导入。在该模型下：
  - 44/49（空 state + 远端无 ISO/VM）无需 `import` 直接 `Destroyed`；
  - 5/49（ISO 已在 state + 远端无 VM）无需 `import` 直接锚定后 `destroy` 掉 ISO；
  - 若 VM 实际已创建但未入库，可通过 `import` 纳管再 `destroy`；
  - 若 ISO 实际已上传但未入库，因 `import_id: null` 会 fail-closed 进入 `Quarantined`，避免静默泄漏。
  - `settle_delay_secs: 65` 覆盖了 Proxmox `/cluster/resources` RRD 约 60 秒的统计延迟窗口。

## 被拒绝的方案

- **L2：`managed_resource_shape[].import_id` 字符串插值 DSL + 解析 `terraform import` stderr 中的 `not found`**：
  - 全仓 6 个内置模板中有 4 个（Proxmox、AWS、AliCloud、TencentCloud）的资源 ID 由服务端或 provider 动态分配（Proxmox VM 使用 `vm_id_start` 分配且 `node` 来自运行时 data source），连触发事故的 Proxmox 模板自身都无法静态拼出 `import_id`。
  - Terraform CLI 没有标准错误码区分 `NotFound` 与 `403`/`429`/`5xx`/配置错误，匹配 stderr 文案脆弱且违背 ARD-0040 原则。
  - `terraform import` 在归属验证前直接写入 state，存在串号污染风险。
- **直接放宽 `records_whole_shape` 允许部分 state 锚定**：在没有远端存在性证明时，无法排除崩在 5xx 中的 API 调用已在远端创建未入库资源，会导致静默资源泄漏。
- **把 `discover` 做成独立 artifact 类型**：增加第二条发布/attestation/版本对齐链路，容易与主模板的 `managed_resource_shape` 产生漂移。

## 失败模式清单（实现前枚举）

1. **残留锁阻塞锚定**：Create apply 崩溃未 UNLOCK 且 worker 未 fenced → `Workers::create` 在非 `PlanFailed` 失败时立即 fence，并在 `destroy_proof` 中对已 fenced generation 重试 `anchor_failed_create`。
2. **重启恢复误隔离可 reconcile 的 generation**：daemon 重启时 `recover_worker` 看到部分 state → 仅当已 fenced、有保留 `protected_input` 且 admitted manifest 声明 `reconcile` 时允许恢复为 `cleanup_only` 进入 `CleanupRequired`（不提前写入 `FailedCreateIdentityJson`），否则保持原有立即 Quarantine。
3. **恶意或错误 `discover` 模块篡改基础设施**：静态 HCL 块白名单 + 运行时 `-backend=false` 与 `admit_discover_plan` 双重拦截。
4. **模板作者漏报某个 role 导致静默泄漏**：`DiscoverOutput::validate` 强制 `roles` 集合与 `managed_resource_shape` 完全相等。
5. **未入库资源不支持 `ImportState`（如 `proxmox_nocloud_iso`）**：`import_id: null` 返回 `ReconcileClassification::Unimportable`，直接 `Quarantined`。
6. **Proxmox RRD 延迟导致刚建好的 VM 在 `proxmox_qemu_vms` 中短暂不可见**：`settle_delay_secs` 门控确保未过沉淀窗口前保持 `Pending` 不做空证明。
7. **`import` 参数注入或伪造 `generation_id`**：校验 `generation_id` 精确匹配、禁止 `import_id` 控制字符及 `-` 前缀、禁止重复地址或未声明资源类型。
8. **空 state 完结被 `ResourceStateSeen` 误拒**：`anchor_reconciled` 仅在 daemon 侧已验证 manifest 声明 `reconcile`、worker 处于 `cleanup_only` 且无锁无活跃命令、且快照确为空时，原子写入 `FailedCreateIdentityJson` 与 `cleanup_revision`。
9. **Provider 5xx / 限流 / 超时导致 `discover` 或 `import` 失败**：绝不判空；通过 `DestroyBackoff` 退避重试，保留 `Destroying` 状态。

## 后果

- 未声明 `reconcile` 的模板（如 `docker`）行为完全不变，仅获得阶段 1 的锁卫生与结构化诊断改进。
- `proxmox`、`aws`、`alicloud`、`tencentcloud`、`kubernetes` 内置模板新增 `reconcile/main.tf` 与 `profile.yaml` 中的 `reconcile` 声明；`proxmox` 模板设置 `settle_delay_secs: 65`。
- 后续如果 `indexyz/proxmox` 为 `proxmox_nocloud_iso` 补充 `ImportState`，只需在 `templates/proxmox/reconcile/main.tf` 中填入 `import_id`，内核无需改动。

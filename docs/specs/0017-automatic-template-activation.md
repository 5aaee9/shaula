# Automatic Template Activation

- Status: Accepted (2026-09-08); implementation and deployment are tracked separately.
- Decision: [ARD-0021](../ard/0021-activate-templates-after-static-validation.md).
- Amends the activation rules in specs 0001–0008 and 0010.

## 1. Outcome

Template Profile 的当前候选版本静态校验通过后，由 daemon 自动激活。用户发布一次即可
等待它出现在 Fleet 的可选列表，不需要 Activate 按钮、手工证明 JSON 或额外请求。
`Ready` 表示静态校验通过、等待自动激活的短暂状态；`Active` 表示允许新的 Fleet 引用，
不表示已通过完整平台运行测试或 Runner 已成功注册。GitHub authentication 的真实访问
验证与激活规则不变。

发布 Template 的 `template.publish` 权限同时授权通过校验后的自动激活。完整 conformance
报告独立保留，仍要求 `template.attest`；它是运行验证证据，不再是 Template 激活门槛。
该规则替代旧规范中所有“只有 passing attestation 才能 Active/被 Fleet 选择”的要求。

## 2. Reconciliation and upgrade

- 周期扫描处理未退休 Profile 的 current desired `Validating` 和 `Ready` Revision。
  重新读取已发布的不可变 artifact，验证 manifest、必需文件与 dependency lock；保留
  既有 publication 对 engine、bindings 和 Fleet-input policy 的校验。校验不运行
  Terraform apply、不请求 GitHub，也不创建 Runner。
- 通过校验后，以短数据库事务记录验证结果并完成 `Ready -> Active`。更新 desired
  Revision 的状态、Profile active/observed head、激活记录、相关 Profile Change 和
  outbox；读者不能观察到只有 head 或只有审计成功的半次激活。
- 事务重新检查 Profile incarnation、desired revision、artifact identity、候选状态
  和 retirement 标记。过期扫描不能激活被替换/退休的候选，也不能覆盖新 head。
- 校验失败标记当前候选为 Rejected，并暴露原因；保留之前的 Active。存储读取/解码
  故障是可重试错误，不能被记录成静态校验通过。没有可用 artifact 不得激活。
- 拒绝结果也必须通过相同的 incarnation/desired revision fence，不能覆盖新候选。
  Profile Change 只完成该精确版本对应的未终结 Publish，不修改历史或 Retire change。
- 已经 Active 的同一版本是 no-op：不重写激活身份、不重复审计，不受再次扫描或
  后续 conformance 提交影响。版本晋升不改写任何既有 Fleet/Generation 的 exact pin。
- 部署升级后，已有 `Ready`（例如 `local-docker r1`）走同一重新校验及事务路径，
  自动补齐激活，无需重新发布或直接编辑数据库。已有 Active 与历史证明保持原样。

## 3. Activation provenance and conformance

自动激活生成独立的、带 `static-validation-v1:` 命名空间的 opaque activation ID，
稳定绑定 Profile key、incarnation、Revision 和 artifact digest。与 head 同事务写入的
不可变 activation audit 记录其 ID、`method=static_validation`、验证规则版本与精确
版本身份；记录不得包含 bindings 明文、凭据或原始运行输出。它不伪造 passing
conformance attestation，也不写入 `template_conformance_attestations`。

为保留已存储 Fleet、Generation 和 API 的兼容性，既有 `active_attestation_id`、
`template_attestation_id`、`attestation_id` / `attestationId` 名称在本次作为历史 wire/
storage 字段保留，承载 opaque activation provenance ID。历史值仍指向原 conformance
记录；新命名空间值指向 activation audit。字段非空仅证明激活身份，不证明 conformance
通过。运行时继续固定并保留该 ID、exact Revision、artifact 和原 inputs；不做大范围
列重命名或伪造旧证明。

Conformance PUT/GET 的权限、请求约束、完整 subject 比较、不可变记录、幂等和审计继续
成立，但任何结果（passed、failed、mismatched 或 stale）都不能激活、降级或替换 Profile
head/activation ID。真实平台验收仍决定可声称的兼容性与运行保证，不能从 Active 推断。

## 4. Operator UI

Templates 页面说明“通过验证后自动激活”，显示 Active revision。暂时停在 Ready 时解释
为等待自动激活；Revision GET 增加 nullable `reason`，仅返回有界静态校验原因码，
失败详情显示该原因，不包含 artifact 原文或凭据。Fleet 仍只允许有 Active revision 且未退休的 Profile，
沿用 [spec 0016](0016-fleet-profile-selection.md) 的刷新、原 pin 和草稿保留规则。
不添加手动 Activate 操作、不把静态校验结果显示成完整运行验证通过。

## 5. Acceptance

1. 真实 artifact/Profile publication 后正常扫描自动 Active，无 attestation；真实 Fleet
   admission 和 exact input-contract 能使用该版本。
2. r2 通过后晋升，r1 的已准入 Fleet pin 不变；r2 被拒绝时保留 r1 Active。
3. 已存储 Ready 在重启/扫描后激活，重复扫描幂等；事务故障不留下半次激活或重复事件。
4. retirement、desired revision/incarnation 变化和校验读取故障不能导致错误激活。
5. conformance 继续可记录/读取/重放，但不能改变自动或历史激活身份。
6. 浏览器覆盖自动晋升后的 Profile 可选与 Template inputs 自动加载；HTTP/SQLite
   回归覆盖实际准入。Rust fmt/clippy、完整 workspace nextest、前端 build/lint/format
   和相关浏览器测试通过；生产部署后核对 `local-docker` 自动成为 Active 且可选择。

实现与验收结果记录在 [implementation status](../IMPLEMENTATION_STATUS.md)。

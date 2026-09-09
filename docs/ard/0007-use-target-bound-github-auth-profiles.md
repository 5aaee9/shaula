---
status: accepted
date: 2026-09-04
---

# Use target-bound GitHub Auth Profiles

[ADR-0015](0015-route-one-github-app-profile-to-multiple-accounts.md) 已将单 installation / 固定 Target allowlist 改为同 App 的多账户 TargetPolicy、Revision-scoped bindings 与 exact context。[ARD-0022](0022-retire-legacy-github-authentication.md) / [spec 0018](../specs/0018-github-app-only-authentication.md) 进一步 supersede 本文历史决定中的 PAT/v1 支持、旧格式升级及 ref-only execution；当前仅支持 schema 2 GitHub App。下述 no-fallback、凭据隔离、quiesce、ownership 与 retention 约束继续有效。

历史选择曾将 GitHub App 与 PAT 并列支持，以照顾不同 GitHub 部署方式；该兼容义务现已取消。当前每个 Fleet 只引用一个允许其 GitHub Target 的 v2 GitHub App Profile，且只按明确 selector 与冻结 binding 路由；运行时不得尝试 PAT、旧格式、旧 revision 或其他 Profile 作为 fallback。

生产环境只使用纯 Rust `shaula-scaleset` 实现 GitHub Scale Set 协议。固定并经评审、精确 pin 到某个 commit 的 Go `github.com/actions/scaleset` oracle 只用于 conformance/differential tests，不随 Shaula 发布，也不由生产进程调用。

GitHub Auth Profile 是稳定身份与 Target policy，credential rotation 产生不可变 GitHub Auth Revision。Fleet 的期望与已观测认证身份始终是完整 `(profile_key, revision)` tuple；同 Profile promotion 与零 Resource Occupancy 的跨 Profile replacement 进入同一个持久 Auth Handoff，而不是两套迁移语义。

## Consequences

- Fleet Spec 不内联 credential；SQLite 的 Auth Revision 明文保存 GitHub App private key，Fleet durable state 保存完整 `desired_auth_ref`/`observed_auth_ref` tuple 和 exact context。历史 PAT rows/bytes 保留为受保护历史，不能继续执行。
- 新 publication 只接受 v2 GitHub App private key 作为 write-only secret input。读取响应只返回非 secret metadata 与 credential presence；audit、errors、logs、traces 和 metrics 永不回显或散列 credential，包括历史 PAT。
- SQLite 主库、WAL/SHM、online/backup/migration copies、crash dumps、文件权限、retention/disposal 和 host access 全部属于 credential-grade security boundary。
- Profile incarnation 固定已验证 numeric App identity；同 App 的 key、TargetPolicy 和 installation bindings 可通过 Candidate publication 演进，并须通过 spec 0011 的 numeric identity continuity、路由收敛和 live-dependent coverage 检查。不同 App 需要新 key，旧格式不能隐式升级。Fleet 只有在 Resource Occupancy 为零且不存在 acquisition/GitHub/Runner 活动操作时才能换 key，GitHub Target 与 Scale Set identity 始终不可变。
- 两类 handoff 都先停止 acquisition、等待跨越 fence 的请求得到持久分类，再对已绑定 Fleet 做只读 ownership proof；未绑定 Fleet 只验证 access/numeric Target identity 并记录 exact context。observed ref 与已验证 context 必须原子确认，ref equality 不是授权。
- Auth Handoff 不得 create/adopt、绑定或改变 Scale Set ID、建立/替换 session 或签发 JIT。上述操作由 ordinary Fleet reconcile 独占；只有它为 observed tuple 建立 ready session 后才恢复 acquisition。
- Decommission 永久禁止新 acquisition，但允许 v2 cleanup-only Auth handoff；该路径必须有 exact verified context，且不得建立 acquiring session、create/adopt、改变 Scale Set ID 或签发 JIT。旧格式 retained execution 必须按 spec 0018 在部署前用上一版本显式恢复，不因 cleanup 而重新获得兼容执行路径。
- 旧 Auth Revision 仅在不存在任何 Profile desired/active/observed head、Fleet desired/observed tuple、in-flight effect/session、Decommission cleanup 或 recovery reference 时才可 GC；`Blocked` 不代表 acknowledgement，也不能释放引用。
- GitHub App private key、derived installation/admin token 和 PAT 都是 Control-Plane Credential，不得传入 IaC Template、Runner Resource 或 workflow。短生命周期 derived token 不是持久 Auth Revision。
- GitHub 自动提供给 job 的是受 job/repository 权限约束的 `GITHUB_TOKEN`，不是 PAT；只有 workflow 显式引用某个 Actions Secret 时，该自定义 secret 才对 job 可见。
- 认证或权限故障只降级使用该 Profile 的 Fleet。`401`、`403` 和 access-filtered `404` 不得被解释为 Scale Set 不存在并进入 Create。
- GitHub App 必须通过 organization/repository、多 org 与个人动态仓库路由的真实 `github.com` 验收，并由固定 Go oracle 做差异测试；PAT/旧版/未知格式必须被拒绝，不能只凭 client construction 成功宣称认证可用。

具体权限矩阵和失败语义由 [Multi-Fleet Runner Scale Set Controller Specification](../specs/0001-shaula-runner-scale-set.md) 定义。

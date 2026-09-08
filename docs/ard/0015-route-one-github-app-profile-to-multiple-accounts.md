---
status: accepted
date: 2026-09-07
---

# Route one GitHub App Profile to multiple explicitly selected accounts

一个 GitHub App 可以安装到个人账户和多个组织，但每次 installation 都有独立身份与权限范围。Shaula 当前把 installation ID 固定在 Auth Profile identity 中，并只允许 exact Target allowlist，导致同一个 App private key 必须重复配置，且个人新仓库需要新 Profile。这里决定让 Profile 固定 **App identity**，Auth Revision 保存 credential、明确的 Target policy 和验证后冻结的多个 account/installation bindings；具体 Fleet Target 在使用凭据前解析到唯一 context。

已接受。本地实现与现有运行时集成边界见 [implementation status](../IMPLEMENTATION_STATUS.md)；真实 GitHub 路由验收由 spec 0011 §9 定义，尚未执行。本决定部分替代 [ADR-0007](0007-use-target-bound-github-auth-profiles.md) 与 [ADR-0009](0009-manage-profile-resources-through-http-and-sqlite.md) 中“单 installation / policy 在 Profile incarnation 内固定”的选择。详细协议唯一维护于 [spec 0011](../specs/0011-multi-account-github-authentication.md)。

## Decision

- Target policy 使用 exact organization、exact repository 和 typed account-repositories selector。个人账户的 selector 覆盖其当前及未来拥有且 installation 获准访问的仓库；Fleet 本身继续绑定具体 organization/repository。
- 用 App JWT 在 Candidate validation 和具体 Target resolution 中发现 installation，核对 App、numeric account/repository identity、权限与 suspension。一个 Profile 不再由 Operator 填写一个全局 installation ID，也不自动信任 App 的所有 installations。
- 已验证 account/installation binding 在 Auth Revision 中冻结。重装得到的新 installation 通过显式 publication 和受控 Handoff 接受，不能作为后台 credential fallback。
- 同 App 的 policy 和 binding 可通过同 key 的新 Revision 演进；Candidate 整体验证、原子 promotion，禁止删除仍被 live Fleet/cleanup/recovery 依赖的权限。App 或 auth kind 改变仍需新 Profile。
- 保留完整 Auth Revision Ref，并给 Fleet/session/effect 增加 exact Resolved Auth Context。复用原来的 quiesce、ownership proof、CAS、cleanup retention 与 ordinary reconcile 边界。
- 活跃配置的健康度按 installation/Target 隔离；一个账户故障不应停止其他账户。Profile publication 的原子性不等于 runtime failure domain 必须扩大到整个 Profile。

## Considered options

| 方案 | 代价与选择 |
| --- | --- |
| 每 installation 一个 Profile，并定期枚举个人仓库写入 allowlist | 重复 credential、Profile/Fleet 迁移频繁，枚举快照不能覆盖未来仓库；不选 |
| 一个 Profile 聚合其他 Auth Profiles 或 App/PAT credential 列表 | 引入多重 identity、rotation/retirement ownership 和潜在 fallback；本需求只需要一个 App，不选 |
| 根据 Fleet owner 每次自动选 App 可访问的任意 installation | 将 GitHub 的安装变化隐式变成 Shaula 准入，难以解释 username reuse、transfer 与 reinstall；不选 |
| 明确的账户/Target policy，加经过验证且有持久引用的 installation context | 满足一份 credential、多账户和未来仓库，同时保留具体资源归属证明；选择此方案 |

使用结构化 `account_repositories` 而不是自由 `owner/*` glob，是为了区分个人与组织、组织 runners 与仓库 runners，并让“账号拥有的仓库”不会混同“账号可协作访问的仓库”。匹配集合仍与 GitHub installation permissions 取交集。

## Consequences

- 数据库格式、publication DTO、validation worker、target-aware client construction、token cache 和 Handoff 都需要演进；只修改 UI 无法建立这个契约。
- 单个 App private key 仍能代表该 App 的所有 installations；Shaula policy 是 daemon 的准入边界，不是密码学隔离。需要凭据泄漏隔离的账户仍应使用不同 GitHub Apps。
- policy expansion 需要显式 Operator publication，但未来仓库的加入不需要编辑 policy；动态匹配不会自动创建 Fleet。
- 相同 App credential 的 rotation 需要验证全部声明 bindings 和 live Targets。一个账户故障可推迟新 Revision 的整体发布；已 active 的健康账户继续工作。接受这个代价以保留单一 immutable Revision 与现有 staged activation。
- 短期 route proof 与按需刷新会增加 GitHub 请求；有界 cache、per-key singleflight 和限流退避控制成本，不使用无限期旧授权兜底。
- GitHub rename/transfer、同名重建和 installation replacement 不能静默改写 existing Fleet identity。撤权检测也不承诺立即停止已在 GitHub 执行的 Busy job。
- 旧数据保留 exact policy、历史身份编码与 execution references；升级不能自动转换成 wildcard。旧 binary 必须拒绝新 durable format，回退必须处理完整 consistency set。
- GitHub private key / derived token 继续只存在于 daemon credential boundary；不会进入 Lifecycle Worker、Terraform、Runner 或 workflow。PAT 原有支持范围及 OIDC 登录不受该提案扩展。

验收由 spec 0011 定义，尤其覆盖个人新仓库、多组织并行、一个 installation 故障、权限收缩、reinstall、Handoff/restart 与 legacy migration；文档存在不构成实现或生产验收证据。

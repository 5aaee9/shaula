---
status: accepted
date: 2026-09-08
---

# List authentication connections from the Profile registry

`/auth` 当前只接受已知 Profile key。Operator 无法从入口发现已保存的认证，
即使其详情可以读取。spec 0005 已列出 collection endpoint，但 HTTP 路由和 UI 未接入。
本决定以 [spec 0012](../specs/0012-github-authentication-inventory.md) 为详细契约，
补充 [ADR-0012](0012-embed-an-api-driven-operator-ui.md) 的 API 驱动 UI。

## Decision

- 将已有 `auth_list` service 接入受 `auth.read` 保护的 collection route。
  registry 是连接列表的唯一来源；UI 中的 connection 仍是 Auth Profile。
- collection 与 detail 共享脱敏 serializer，保留 legacy/v2 以及 active/desired 的
  Revision 归属。列表不引入另一套 summary 状态模型，不调用 credential handoff 接口，
  不向 service/API/UI 传递或显示凭据；Store 内部沿用现有行读取与脱敏映射。
- `/auth` 默认展示库存；key 搜索、刷新和行选择复用现有 query/UI 机制。
  选中行继续调用详情接口，详情响应继续负责 conditional-write 版本。
- 当前返回完整本地 Profile 集合，并明确替代 spec 0005 Auth collection 的未定义分页占位。
  不静默截断，不自动枚举 GitHub 仓库；未来分页需要 API 与 UI 的共同契约。
- 数据读取失败必须沿错误路径传播；不能为了渲染部分列表而隐藏未能读取的连接。

## Alternatives

| 方案 | 取舍 |
| --- | --- |
| 保留按 key 输入，添加帮助文字 | 仍要求用户预先知道 key，不能满足连接可发现性 |
| 从 Fleets 或浏览器访问历史收集 keys | 遗漏未引用/未访问的认证，产生第二份不完整库存 |
| 直接从 GitHub App 枚举 installations | 远程安装不是 Shaula Profile，可能含未声明授权范围，并引入读取副作用/外部依赖 |
| 新建专用 summary DTO/cache | 需要重复维护 active/desired、legacy/v2 与脱敏规则，本次没有必要 |
| 为此增量新增 cursor/search 协议 | 对当前本地配置库存增加跨层复杂度，缺少规模需求；暂不选择 |
| 挂接 registry collection、复用详情投影 | 满足已有连接展示，并保持单一读模型；选择 |

## Consequences

无需数据库迁移、凭据重录、GitHub 权限修改或新后台 worker。
前端与 HTTP route 随同一个嵌入 UI 的二进制发布。

完整 collection 携带详情的非 secret metadata，读取与响应成本随 Profile 数量增长；
不提供全部记录的事务快照。共享投影避免状态和脱敏分叉，但详情字段演进时必须验证
collection 兼容性与读取成本。只有遇到实际规模需求才应引入独立分页契约或更窄的投影。

列表能显示现有配置，不证明 GitHub live access 或 Fleet readiness；相关语义继续由
[spec 0011](../specs/0011-multi-account-github-authentication.md) 拥有。
验收必须覆盖实际入口、API 权限、旧数据、多账户 policy 归属和失败状态，不能只测试详情 URL。

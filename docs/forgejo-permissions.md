# Forgejo 最小权限

控制面 token 只交给 Shaula 的 `forgejo_token` Auth Profile，不进入 Runner、Terraform
输入、Setup Info 或普通日志。每个 Runner 另有一次性的 ephemeral token。

下表是 Forgejo **16.0.4** 隔离实例的实测组合；v15 路由/权限类别经过源码核对，
不代表所有 v15 patch、反向代理或定制权限策略都已经运行过。

| Fleet scope | 管理 Runner 所需 token scope | token 所属用户条件 | 可选 Jobs 终态读取 |
| --- | --- | --- | --- |
| instance | `write:admin` | 实例管理员 | 另加 `read:repository` |
| user | `write:user` | 目标就是该 token 用户 | 另加 `read:repository` |
| organization | `write:organization` | 目标组织 owner | 另加 `read:repository` |
| repository | `write:repository` | 目标仓库管理权限 | 已包含仓库读取 |

`write:*` 包含该类别的读取，不能代替其它类别。例如 `write:admin` 本身不包含
`read:repository`。历史读取还要求用户本来有权读取对应仓库；增加 token scope
不会授予另一个私有仓库或组织的访问权。Shaula 还核对 repository/owner 数字身份，
不通过名称相似、重命名或上游 URL 扩展 Fleet scope。

验收通过真实 API 验证 scope identity、库存、jobs、ephemeral POST、精确 GET、
DELETE 与删除后 404，并通过 Shaula 的 Auth 发布接口验证 Active。
负例包括只读同类别 token 的 POST/DELETE 被拒绝、非管理员访问 instance 被拒绝、
其它用户的私有仓库和组织不可见。用没有 `read:repository` 的 token 查询历史会失败，
加上该权限才可读取；容量与回收仍可只使用管理权限。

**Active 是有界认证读取成功，不是写权限探测。** 激活不会创建测试 Runner，
因此一个只能读库存的 token 也可能完成读探测，但之后的注册/删除会失败。
部署时应按上表授予实际所需权限，不以 Active 状态替代上述最小权限配置。

重复方式：[`scripts/forgejo-lifecycle`](../scripts/forgejo-lifecycle/README.md)。
凭据轮换时保留旧 token，直到旧 Revision 下的 Runner 清理完成；不要提前撤销
仍被清理路径引用的凭据。Profile 的 Retired 状态释放执行引用，不代表历史诊断
或受保护凭据文件已经物理擦除。

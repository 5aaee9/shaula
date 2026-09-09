# Stored Template Library and Terraform Variable Discovery

- Status: Accepted (2026-09-08); implementation and verification are tracked separately.
- Decision: [ARD-0019](../ard/0019-store-template-sources-and-discover-terraform-variables.md)
- Extends: [Template runtime](0004-template-profile-runtime.md),
  [Profile HTTP](0005-profile-http-control-plane.md), [visual inputs](0014-visual-template-inputs.md).

## 1. Outcome

Templates 页面提供已经存入数据库的默认模板来源。Operator 可以选择 Docker、Kubernetes
或其他已导入来源，查看模板所需变量并发布一个配置好的 Template Profile，无需先手工打包
仓库或从源码猜测参数。上传 archive 和引用已有 digest 仍然可用。

数据库保存原始模板 archive；默认文件目录是导入来源，不是持续覆盖 Profile 的期望配置。
Template Source、Template Artifact 和已发布 Template Profile 的区别见
[CONTEXT.md](../CONTEXT.md)。导入和发现变量不代表模板已通过 conformance 或可以运行 Runner。

## 2. Persistence and filesystem import

- SQLite 按完整原始 tar.gz 的 SHA-256 身份持久化不可变 archive bytes；成功上传必须包含
  数据库提交。重复 digest 幂等复用，同一 digest 不得替换为不同 bytes。
- 保留原有压缩大小、展开大小、文件数、路径、重复路径和链接检查。解包缓存和保留 archive
  sidecar 是可从数据库重建的执行材料，不能作为另一份独立可编辑模板。
- 升级时从原存储的 archive sidecar 导入已有材料并验证原 digest；不得重新打包展开目录
  来冒充已有内容身份。缺失或损坏材料需明确报错，不能删除现有 Revision 或生成新 pin。
- 数据库备份恢复后，在启动扫描/执行之前恢复所需缓存。运行时缓存丢失必须有明确恢复路径，
  且不能从其他 digest 或文件系统最新模板替换被 Generation 固定的材料。
- 配置一个可信的默认模板目录，启动时扫描其直接子目录。每个子目录包含可发布的 manifest、
  Terraform source、lock 和 schemas；默认包与上传包经过相同安全检查。目录不来自浏览器输入。
  文件打包顺序和 metadata 规范化，使相同内容产生稳定 digest；拒绝 symlink/reparse point，
  不导入 `.git`、`.terraform`、state、tfvars、image build context 或凭据文件。
- 默认来源用稳定 key 登记到数据库。重复启动不增加 Profile Revision、Change 或 Active；
  已登记 source key 保留数据库中的 digest，软件升级不静默覆盖它。新版本仍可通过显式
  archive 发布流程采用。删除来源目录不会删除数据库中的来源或已发布模板。
- Nix package 安装 bundled sources，NixOS module 配置对应只读路径。未配置目录的其他
  部署继续接受 HTTP 上传，不依赖运行时源码 checkout。

## 3. Terraform is the variable declaration authority

使用 HCL AST 静态解析 root Terraform 文件；不使用正则表达式猜测结构，不执行 Terraform、
provider、`file()`、命令或任何用户表达式来发现默认值。

Shaula 继续只向 Terraform 传入一个敏感 `variable "shaula"` 系统 envelope。可发现模板将
其 type 写为 object，`bindings` 和 `parameters` 各自声明 object 属性。例如：

```hcl
variable "shaula" {
  sensitive = true
  type = object({
    contract_version = number
    generation       = any
    jit_config       = string
    bindings_digest  = string
    bindings = object({
      docker_host = optional(string, "unix:///var/run/docker.sock")
    })
    parameters = object({
      runner_image = optional(string, "approved-image-alias")
    })
  })
}
```

- 名称、类型、是否为 optional 以及 optional 的静态默认值来自上述 Terraform 声明。
  系统字段不作为 Operator 输入展示，也不暴露整个 envelope 的默认值。
- 静态 literal 支持 null、boolean、number、string、array 和 object；变量引用、函数调用、
  插值或不能安全解释的表达式返回明确不可发现原因，不把它们执行后猜成默认值。
  只发现 root `.tf` 模块；`.tf.json` 和 override 文件明确返回不支持。
  HCL 默认值及 schema 候选值中的数字不能静默舍入或溢出；无法无损表示为 JSON 时
  返回明确错误，不向表单提供被更改的值。
- schema 继续拥有 description/title、敏感 binding 标记、有限选项和额外值约束。
  discovery 检查两个输入组的 key/type 与 schema 一致；schema 若也声明 default，必须与
  Terraform 声明一致。默认值只维护在 Terraform 中，bundled schema 不再复制默认值。
- schema 可以要求 publisher 显式提交一个 Terraform 有默认值的 binding；UI 将其展示为
  必填且提供显式采用默认值的操作。不能据 Terraform optional 绕过现有 admission required。
- 更新 bundled 模板，把原 `try(var.shaula.parameters.x, fallback)` 的默认值移动到
  `optional(type, fallback)`，资源表达式直接读取声明后的属性。Terraform 对 absent 和
  explicit null 都可能应用 optional 默认；现有 bundled parameter schema 不允许 null，
  因此 Fleet admission 的 null/absent 区分保持不变。
- 旧 `type = any` artifact 保持原有发布和执行兼容，discovery 返回 available=false 和
  固定原因。UI 不伪造属性/default；可以继续使用现有人工配置流程。
- 发现结果有界：字段/选项各至多 256，值嵌套至多 16，序列化响应至多 1 MiB。无法支持
  的声明或不一致配置不得以空变量列表伪装成功。
  解析前还限制每个 `.tf` 文件的语法嵌套至多 64 层、operator/template-control 标记
  总计至多 64 个；这是保守资源上限，超出时明确拒绝。字符串、注释与 heredoc 的
  字面括号不计入，插值内部语法仍受限。

Terraform 语义依据：[variable declaration](https://developer.hashicorp.com/terraform/language/block/variable)
及 [optional object attributes](https://developer.hashicorp.com/terraform/language/expressions/type-constraints#optional-object-type-attributes)。

## 4. Read interfaces and permissions

| Interface | Permission | Result |
| --- | --- | --- |
| `GET /api/v1/template-sources` | `template.read` | `{sources: [{key, artifactDigest, platform, engineRef}]}` |
| `GET /api/v1/template-artifacts/{digest}/variables` | `template.read` | 精确 artifact 的变量描述 |

变量响应包含 `artifactDigest`、`available`、可选 `reason`、`bindings` 和 `parameters`。
每个变量包含 `key/label/description/typeName/required/sensitive/options`，以及确有可公开
默认值时的 `defaultValueJson`。选项沿用 `{valueJson}` 原始 JSON token，保留 number 精度。

敏感 binding 不返回默认值、选项或已有 Profile binding 内容，也不返回 system JIT、源码、
完整 schema、目录路径或 archive bytes。默认值表示模板声明，不表示任何已部署环境状态。
缺少 defaultValueJson 与显式 `"null"` 不同。所有文本按纯文本渲染。

使用既有 OIDC 和 private/no-store：401 未认证、403 缺少权限、404 digest 不存在；
409 表示变量声明无法支持或不一致，500 表示数据库/材料读取失败。GET 不创建或激活 Profile，
也不调用 provider。上传及发布继续要求 `template.publish`、CSRF 和既有条件写/幂等语义。

## 5. UI interaction

- Templates 页同时展示可选来源和已经发布的 Profiles，清晰区分二者的用途和状态。
  选择来源后进入 `/templates/new`，携带该来源的完整 metadata 并固定其 digest；
  刷新来源列表不能改写已打开草稿。
- 新建 Template 使用 `/templates/new` 独立页面，发布已有 Profile 的新 Revision 使用
  `/templates/{key}/revisions/new` 独立页面。成功提交后返回 Templates 列表，选中对应
  Profile 并展示已接受的 Change；取消返回列表，Revision 流程保留原 Profile 的选中状态。
- 发布表单支持 Default template、Upload archive、Existing artifact。后两种来源可显式
  Inspect variables；上传后的 inspection 和提交复用同一精确 artifact。
- variables 按 Template bindings / Fleet parameters 分组展示名称、类型、说明、必填、
  非敏感默认值与候选值。可支持的 scalar bindings 用可视化控件编辑，其他形态保留现有
  明确的手动 JSON 配置入口；只维护一份绑定草稿。
- 默认值不自动填入。使用默认 bindings 和采用声明候选选项分别是 publisher 的显式操作。
  采用选项只填充发布草稿，最终仍须 publisher 提交才能形成 immutable Fleet input policy；
  不在 Fleet GET/PUT 时从 schema 或 defaults 自动扩大批准集合。
  `optional(type, null)` 的默认值可展示为 null，但采用默认值时保持字段缺省，避免将
  Terraform 的 omitted 默认错误转换成 schema 不允许的显式 null。
- 敏感字段 write-only，草稿只保留在页面内存。模板更换/读取错误/乱序响应不能覆盖已编辑
  bindings/policy、原 ETag 或 MutationAttempt；terminal session failure 按既有规则清理草稿。
- 所有 Fleet Template inputs 继续直接放在主表单，不进入 Advanced settings。

## 6. Acceptance

1. 上传的完整 archive 确实进入 SQLite；相同 digest 重复上传幂等，错误 digest/恶意 archive
   无法成为可发布材料。删除执行缓存并恢复数据库后能够重建完全相同的材料。
2. 旧 sidecar 导入、重复启动、来源目录移除、同 key 源码更新均不改变既有 Profile/Active/pin。
   默认 Docker/Kubernetes 来源出现在页面，可选择并准备正常发布请求。
3. HCL 注释、多文件、嵌套 object/optional、false/0/空字符串/null/大整数均正确处理；
   duplicate variable/attribute、动态 default、未知类型和 schema 失配明确失败。
4. 旧 any 模板仍可运行且不伪造 defaults。敏感 default/enum/system 内容不进入响应或日志。
5. 原始 Terraform 默认与 UI 发现结果一致；真实 Terraform validate/现有 runtime 测试通过。
6. UI 验证默认来源选择、变量查看、显式采用、无权限、失败和切换时的草稿保留及最终 PUT。
   默认模板导入不跳过 conformance gate，不以成功展示等同于 Runner 已可运行。

验证与部署结果记录在 [implementation status](../IMPLEMENTATION_STATUS.md)。

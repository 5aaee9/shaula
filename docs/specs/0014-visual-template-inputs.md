# Visual Template Inputs Specification

- Status: Accepted (2026-09-08); implementation and verification are tracked separately.
- Decision: [ADR-0018](../ard/0018-render-fleet-inputs-from-approved-template-options.md)
- Extends: [Fleet admission](0002-fleet-http-control-plane.md),
  [Profile reads](0005-profile-http-control-plane.md), [operator UI](0008-embedded-web-ui.md).

## 1. Outcome and scope

创建或编辑 Fleet 时，Operator 选择 Template Profile 后，通过有名称、说明和批准选项的
可视化控件设置 **Template inputs**，不再手写 JSON。全部输入（包括可选项和完整配置
preset）在模板选择下方直接显示；Advanced settings 只容纳其他高级 Fleet 配置。
展开、折叠和后台刷新不能隐藏输入控件或改变输入值。

本规范只替换 Fleet 表单中的 `template_inputs` 编辑区。Template publication 的
`bindings` 和 `fleet_input_policy` 编辑器、模板源码、Terraform variables、GitHub
credentials 均不属于这份编辑器。领域含义见 [CONTEXT.md](../../CONTEXT.md)。

当前实现不是完整 JSON Schema 表单系统：每个输入必须同时满足 artifact parameter schema
和所选 Revision 的 finite alias policy。Docker 的 `runner_image`、Kubernetes 的
`runner_image` / `cpu_request` / `memory_request` 都是有限选项；实际可选值还必须经过该
Revision 的 policy 收窄。不能根据平台名称硬编码这些字段，也不能只根据 schema enum 展示值。

## 2. Input authority and read contract

新增只读接口：

`GET /api/v1/template-profiles/{profileKey}/revisions/{revision}/input-contract`

- 使用既有 OIDC、`template.read` 和 private/no-store 契约；未认证 `401`、缺权限 `403`、
  Profile/Revision 不存在 `404`。可读取仍保留材料的历史 Revision；读取不使其获得新引用资格。
- Profile Registry 从指定 immutable Revision 的 artifact parameter schema 和 input policy
  生成表单投影。响应绑定 `profileKey`、`incarnation`、`revision`、`artifactDigest`，不得混用
  desired head、current Active 或其他 Revision 的材料。
- 此接口不执行 Terraform/GitHub/Docker 调用，不产生 validation job、Change 或持久化表单记录。
  不返回 archive、bindings schema/value、binding commitment、JIT、provider credentials 或运行配置。
  输入别名及其展示说明属于供 Fleet manager 使用的非 secret 契约，publisher 不得将凭据放入其中。
- schema 缺失、空白或存储读取失败返回 `5xx`，不得当成没有输入。schema/policy 不合法、
  不支持或不能生成有界投影时返回 `409 InputContractUnavailable`，给出不含输入值的有界原因。
  普通 Revision metadata GET 仍独立可用，不因这次投影失败而失效。
- 正常无输入返回 `200` 和空 `fields`；这只表示该精确契约允许空输入，不是错误回退。

响应的 v1 形状如下；字段名与 `valueJson` 均为协议数据，不是 HTML 或可执行表达式：

```json
{
  "version": 1,
  "profileKey": "kubernetes-linux",
  "incarnation": "opaque-profile-incarnation",
  "revision": 4,
  "artifactDigest": "sha256:...",
  "mode": "fields",
  "fields": [
    {
      "key": "cpu_request",
      "label": "CPU request",
      "description": "",
      "required": false,
      "options": [{ "valueJson": "\"500m\"" }, { "valueJson": "\"1\"" }]
    }
  ]
}
```

`mode: fields` 的 `fields` 按 key 排序。label 使用有效的 schema `title`，缺少时从 key
生成可读名称，同时可查看原始 key；description 来自有效的 schema 字符串注解。所有文字
按纯文本呈现，前端不能将 Markdown/HTML、URL 或 schema 内容作为指令执行。

### 2.1 Projection rules

1. 后端共用 Fleet admission 的 schema 解析、值校验和 policy 比较逻辑。当前支持的语法
   保持不变：`type/properties/required/enum/additionalProperties` 及已有注解；本功能不添加
   `$ref`、`items`、条件分支、任意数值区间、`sensitive` 或自定义 UI schema。
2. 每个顶层字段的候选值来自 `policy[key]` 数组，并按该字段和 root schema 的约束过滤、
   去重；不得从 schema enum、default、已有值或其他 Profile 推断额外批准值。
   root `additionalProperties` 不允许的 key 不可设置。允许但没有 property 声明的 policy
   key 使用 key 作为标签，仍只提供批准的完整值。顺序沿用 policy 中首次出现的顺序。
3. 必填字段不存在合法选项时返回 `InputContractUnavailable` 并指出 key；可选字段没有合法
   选项时不提供设置控件。不能自动补值、解除 required 或把无法设置的必填字段藏起来。
   fields 模式还必须确认 root 允许 object，且不同的顶层 required key 不超过 32 个；
   无法满足这两个条件时同样不可编辑。返回空 fields 前必须用完整 admission 规则验证 `{}`。
4. root `enum` 约束整个 inputs object 时使用 `mode: presets`，响应以 `presets` 代替
   `fields`；每个 preset 的 `valueJson` 是通过完整 admission 校验的一个 root enum 对象。
   用户选择完整配置，不能通过独立字段选择产生 enum 外的新组合。没有合法 preset 时不可编辑。
5. `valueJson` 是服务端对批准 JSON 值的无损序列化文本；它用于保留原生类型及大整数，不能
   转换成用户可编辑的 JSON 框。前端只提交选中值对应的原始 JSON 值，不能提交 option index、
   显示标签或把 number/boolean 转成 string。最终 Fleet wire format 仍是 JSON object。
6. `default` 只是现有 schema 注解，不能自动补值、预选第一项或解除 required。
   未设置意味着 payload 中没有该 key；不得把它标成一个未经证实的实际运行时默认值。
7. 投影最多 1 MiB UTF-8，最多 256 个可设置顶层字段、每字段或 presets 最多 256 个选项，
   值展示最大嵌套深度 16。超限返回明确不可编辑原因，不能静默截断选项或输入。
   这是可视化投影的边界，不改变现有发布/API admission 范围；一次提交仍最多 32 个顶层输入。

前端不实现另一套 JSON Schema validator。字段投影不是整体 admission 的证明；现有 Fleet
PUT 必须继续在服务端对完整对象执行 required、nested object、root enum 和 policy 校验。

## 3. Form interaction

| 输入形态 | 可视化操作 | 值保留规则 |
| --- | --- | --- |
| string / number / integer 的有限值 | 下拉选择；长列表可搜索 | 展示字符串原文或数值，提交原生类型 |
| boolean | 显式 Yes / No 选择 | 可选字段另有“未设置”，不能把未设置等同 false |
| object / array 的有限值 | 选择完整配置，按属性列表或有序列表展示只读预览 | 不允许编辑子字段或拼接多个选项 |
| 无 type 的 null / mixed enum | 带类型标识的有限选择 | 区分 null、空字符串、数字和同文本字符串 |
| root object enum | 完整配置选择器，展示各配置的参数摘要 | 一次选择一个通过校验的完整 inputs object |

- Required 和 optional 输入及已有错误都直接出现在 Template profile 下方；没有选择时显示
  明确 placeholder，即使只批准一个值也必须由用户选择。Optional inputs 可选择或明确
  “取消设置”。所有输入始终可见，不受 Advanced 折叠状态影响。
- Template inputs 不计入 Advanced 的自定义设置数量。没有可设置输入时显示“此模板无需配置输入”，
  不显示空 JSON 框或无作用的参数编辑入口；其他高级 Fleet 配置仍按 spec 0008 展示。
- `presets` 初始不替用户选值；若原 inputs 对象已匹配某个 preset，则显示该选择。
  完整配置选择器及其预览始终直接显示。
- optional 缺省、`false`、`0`、`""`、`null`、`[]`、`{}` 必须分别表示。
  整数及嵌套数值不得经过有损 JavaScript Number 往返；读取现有 Fleet 和组装 PUT 时都须保留
  JSON 数值精度。显示可以有摘要，但确认具体值时必须可查看完整可读内容。
- 现有值不在当前选项中时显示“已有值不再可选”，保留原值并要求显式选择替代值或取消设置；
  不可静默丢弃、取第一项或把新旧对象合并。root preset 没有匹配项时同样处理。
  这也覆盖完全不在 fields 中的已有 key：单独展示只读的原 key/value 和显式移除入口，
  不能因可选字段没有合法选项就将已有数据隐藏。只修改其他 Fleet 字段时按 §4 保留原值。
- 控件需支持键盘、可访问标签、错误关联、390px 窄屏和可滚动的长选项。校验错误聚焦
  对应输入字段；不能依赖颜色表示错误。服务端整体校验错误显示在表单中，
  不承诺从错误文字猜测字段路径。
- 本轮不提供 raw JSON 编辑模式。复制/检查信息不是绕过 finite policy 的入口。

## 4. Revision selection, drafts and submission

| 场景 | 使用的输入契约 | 提交的 Template reference |
| --- | --- | --- |
| 新建 Fleet，或明确改用其他 Template / Revision | 已读取的 current Active exact Revision | `{key, revision}`，明确固定本次展示的版本 |
| 编辑 Fleet，未改变 Template reference | Fleet GET 的 `resolved.template` exact pin | 原始 `spec.template_profile_ref` 原样保留，包括 bare-key 形式 |

原 bare-key Fleet 已经绑定历史 Revision 时，不能仅为渲染表单把它改成 exact old pin，
否则现有 admission 会将其视为新引用并按 current Active 门禁拒绝。新建/改模板时则不能
继续提交会在服务端重新解析的 bare key，否则可能在用户未看到的新 schema 下接受输入。
默认“使用 Active”表示捕获一次当前 Active，不是随模板升级持续跟随的订阅。

切换 Template 或 revision 后，取消过时的读取结果，按完整 Profile/Revision/artifact 身份
隔离编辑状态。同名字段不能自动跨契约复用。仅当有已填写的输入需要替换时，提示用户确认
丢弃这些输入，或取消切换继续原草稿；请求失败时保留原草稿。不能在后台 refetch 时切换
契约、覆盖现有字段、重置 Fleet ETag 或自动补入新选项。重选同一模板的最新 Active 必须
是明确动作，并展示 Revision 改变。

新建或修改模板时，只有 exact 契约成功加载且必填选择完成才能提交。没有 Active、缺少
`template.read`、契约超限/不支持或读取失败均显示具体原因和适用的重试入口，不能提交 `{}`
作为回退。只有 `fleet.write` 的用户不因此获得 `template.read`。

编辑已有 Fleet 且契约不可读取时，仍可尝试提交其他 Fleet 字段，但输入和 Template reference
必须原样保留且锁定，明确提示无法编辑参数；不能把隐藏值置空。服务端仍完整校验并可能拒绝
该提交。读取返回不受支持的旧值时也不能为容量修改偷偷清除它。

Fleet original snapshot、strong ETag / If-Match 和 MutationAttempt 幂等语义保持不变。
提交前后的 Template promotion/retirement 由既有事务 admission 最终裁决；所选 Revision
不再可新引用时显示错误，保留输入，并要求显式加载/复核新版本，不能自动换版本重发。
`412` 保留冲突编辑；成功 session renewal 保留同一份草稿；logout/terminal `401` 清除它。
所有输入只驻留页面内存，不写浏览器 storage、URL、日志或遥测。

## 5. Acceptance and delivery boundary

1. 真实 Fleet 创建表单选择 Docker/Kubernetes 契约后显示 policy 允许的选择器。schema enum
   中未获 policy 批准的值不可选；必填、可选输入及 presets 均默认可见。
2. `{}`、未设置、false/0/空字符串/null、nested object、array、mixed-type 及大整数 round-trip
   保持语义与精度。默认值和唯一选项均不会自动写入；原值不匹配时不被丢弃。
3. root enum 只允许合法完整 preset。覆盖 nested required/additionalProperties、未知 keyword、
   malformed policy、空必填选项、投影超限和 32-key admission 上限；UI 不能放宽服务端校验。
4. 接口验证 `401/403/404`、历史 Revision、缺失/空白/不可读 artifact、非法 schema 和正常
   空契约的区分。响应/error/log 不泄露 bindings；调用不产生外部请求或持久 mutation。
5. 编辑 bare-key 旧 Fleet 使用其原 exact pin；新建/换模板以展示版本 exact 提交。覆盖加载
   乱序、后台 promotion、retirement、显式刷新、切换取消和切换失败后的草稿保留。
6. 缺少 template.read 或契约读取失败时，新建/换模板被阻止；原 Fleet 容量修改保留原 inputs
   和 reference，仍由既有 admission 裁决。覆盖 original ETag、412、幂等和 session renewal。
7. 浏览器验收包含鼠标/键盘、窄屏、Advanced 折叠后输入仍可见可编辑，以及最终 PUT 的精确 JSON 对象；
   不以截图或 mocked 成功响应替代服务端 contract/admission 测试。

后续实现顺序：共用输入契约解析及投影 → 精确 Revision 只读 API → 类型和值保留的表单控件
→ Fleet 表单接线 → 服务端与浏览器回归。不要扩展成平台专属表单或通用 JSON Schema 引擎。
实现时运行 rustfmt、strict Clippy、全量 workspace nextest、前端 build/lint/browser tests
和既有真实 HTTPS/OIDC 回归；验证记录只写入 [implementation status](../IMPLEMENTATION_STATUS.md)。

本规范定义目标契约；实现、部署和验收结论由 implementation status 单独记录。

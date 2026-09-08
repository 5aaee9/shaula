---
status: accepted
date: 2026-09-08
amends: [0012]
---

# Render Fleet inputs from approved Template options

Fleet 的 Template inputs 将由服务端针对 exact Template Profile Revision 生成可视化输入
契约，前端提供批准值的选择器。详细 API、交互和验收由
[spec 0014](../specs/0014-visual-template-inputs.md) 维护。

## Context

当前 Fleet 表单只有 JSON textarea，要求用户知道参数名称、类型和批准值。上一轮
Advanced settings 缩短了默认表单，但没有降低实际填写参数的难度。现有 Template revision
读取接口只返回元数据，浏览器也没有渲染参数所需的 schema/policy。

Fleet inputs 的 authority 有两层：artifact parameter schema 和 immutable Revision 的
finite alias policy。后者要求每个完整值属于批准集合；object/array 同样是整体批准的值。
root enum 还可能约束整个 inputs 对象。因此通用自由输入表单会展示许多后端不允许的操作，
仅靠字段类型或 schema enum 生成 UI 也不正确。

## Decision

Profile Registry 提供独立的 exact-revision `input-contract` 只读投影，共用 Fleet admission
的契约解析和校验逻辑。前端收到字段说明、required 状态和批准选项；root enum 使用完整
配置 preset。UI 根据值形态选择控件，不解释平台名称、Terraform HCL 或另一套 schema 方言。
完整对象仍由现有 Fleet PUT admission 校验；表单投影不能成为额外的授权 authority。

独立读取接口使用现有 `template.read`，与普通 metadata GET 分开，使 artifact/schema
损坏或不支持的表单契约不会连带破坏 Revision 状态查看。投影按需生成，不增加 SQLite
实体、schema 镜像、后台任务或公开 artifact 下载能力；敏感 bindings 不进入这条读取路径。

选项携带无损 JSON 值表示，前端以可读字段/列表展示，不能任意编辑其内部结构。这样可保留
数值精度和 null/缺省等差异，同时维持原有 Fleet JSON wire format。没有 raw JSON 兜底
编辑入口；不支持的契约明确不可编辑，已有 Fleet 的其他字段仍可保留原 inputs 后提交校验。

全部 Template inputs（包括可选参数和 presets）直接展示，Advanced settings 只保留其他
高级 Fleet 配置；输入不是隐藏的高级选项。schema default 不改变现有语义：
只有用户明确选择的值才成为 Fleet input。取消设置表示省略该 key，不代表 false 或空值。

新建/变更模板时以展示的 current Active exact pin 提交，避免读取和写入落到不同契约；
编辑未变更的引用则用原 Fleet resolved pin 渲染并保留原 wire reference。表单不随后台
promotion 重置，Fleet ETag 和幂等语义也不被新的 contract read 替换。

## Alternatives considered

- 通用 JSON Schema form renderer：需要新增 schema 方言、复杂值编辑与前后端校验同步。
  它不能单独表达当前 finite policy，也可能让用户构造未批准的复合值。
- 简单 key/value 表格或只把 JSON 框藏入 Advanced：仍要求用户了解 key、类型和批准值，
  无法满足主要输入路径的可视化需求，也不能防止错误组合。
- 按 Docker/Kubernetes 编写专属字段：复制 artifact authority，模板升级或第三方模板出现
  时需要改 UI；违背 Template Profile 承载平台能力的现有边界。
- 浏览器下载 artifact 或直接消费完整 schema/policy：扩大公开材料与浏览器解释责任，
  还会重复服务端 grammar/admission。只读投影足以提供所需操作。
- 为 bare-key 提交新增 expected-template-resolution 条件：可以保留新建请求原来的拼写，
  但要增加新的 admission 协议。现有 exact reference 已能固定新建/换模板时展示的契约；
  旧 bare-key 引用保持原样即可维持既有 pin。

## Consequences

选择模板会增加一次受权限控制的 contract read；契约不可读取时无法新建或换模板。新建
Fleet 的 UI 请求会使用 exact reference，CLI/API 原有 bare reference 仍有效，不做数据库
迁移或批量改写已有 Fleet。

可视化编辑范围随已批准的有限值及投影上限约束。将来若需要自由输入、复杂条件、敏感 Fleet
参数或模板发布表单可视化，应分别修订对应 authority 和读写契约，不能通过 UI 特例放开。

本决定已接受。实现进度见
[implementation status](../IMPLEMENTATION_STATUS.md)，不由这份 ARD 宣称完成。

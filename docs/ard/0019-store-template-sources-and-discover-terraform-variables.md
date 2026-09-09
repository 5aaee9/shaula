---
status: accepted
date: 2026-09-08
amends: [0009, 0018]
---

# Store template sources and discover Terraform variables

Template archive bytes 与 Profile 数据共同持久化到 SQLite，文件系统成为导入来源和可重建
执行缓存。默认模板导入为可供 publisher 选择的 Template Sources，不自动创建或激活 Profile。
变量类型和默认值从 Terraform 的 typed `shaula` 声明静态读取，具体契约见
[spec 0015](../specs/0015-template-library-and-variable-discovery.md)。

此前数据库仅保存 Profile metadata，archive 仍独立保存在磁盘，数据库单独恢复无法恢复
模板。将完整原始 archive 按 digest 保存可以维持 Generation 的内容身份并恢复执行材料；
代价是数据库和备份变大。保持压缩/展开上限和不可变去重，不在数据库中复制运行工作区或 state。

默认目录持续覆盖 Profiles 会把文件系统与管理 API 变成两个期望状态来源。首次 seed 到
数据库来源库后，publisher 再冻结 bindings 和 input policy，仍通过原静态验证和 conformance
门禁，可保留现有部署的控制权，也不需要替 Kubernetes 环境猜测凭据。

来源库的首次 seed 策略已由 [ARD-0025](0025-sync-default-templates-and-explicitly-update-published-revisions.md)
修订为每次启动同步当前默认目录；Profile 的发布、配置冻结与独立生命周期继续保留。

默认值复制到 manifest/schema 容易与 Terraform `try(...)` 失配。采用 HCL AST 读取类型声明
中的 `optional(type, default)`，把同一声明同时交给 Terraform 和 discovery 使用；schema
保留批准范围、说明和敏感字段约束，发布发现时检查一致性。这样不需要执行任意表达式来发现
配置，也不引入通用 Terraform expression evaluator。旧 `type=any` 模板仍兼容，但明确说明
无法从其声明发现属性，不根据源码片段猜测运行默认值。

发现默认值和批准 Fleet 参数是两件事。UI 提供显式采用操作，默认值不能自动成为批准 policy，
敏感字段默认值也不能通过只读接口暴露。既有可视化输入的类型、精度、草稿和版本固定规则继续生效。

发布页面使用 checkbox 编辑每个参数的批准集合；Optional 不改变编辑权限，只允许缺省。
无枚举的 scalar 参数可输入并显式添加批准值，boolean 用 radio 区分未选与 false。
默认值作为提示或显式采用入口，不能因页面渲染写入绑定或批准集合。控件共用原始 JSON
token 草稿，保留数值精度、候选外批准值及未编辑参数；不增加另一份授权策略或客户端 schema validator。

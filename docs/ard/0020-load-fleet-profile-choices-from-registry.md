---
status: accepted
date: 2026-09-08
amends: [0012, 0018]
---

# Load Fleet profile choices from the registry

Fleet 表单复用 Profile Registry 的 authentication/template 列表，使用明确的下拉选择器。
选择 Template 后自动读取 current Active 的精确输入契约，保持既有 inputs 和 revision
规则。产品行为与验收由 [spec 0016](../specs/0016-fleet-profile-selection.md) 维护。

authentication 文本框和 Template datalist 要求用户记住 key，输入建议也不明显。
两个现有列表 API 已包含选择所需的身份、Active revision 和状态；复用它们即可，不增加
专属组合 API、Target policy 解释器或按平台写死的选项。Profile Source 仍不是可引用的
Profile，目录导入不能绕过发布/激活流程。

下拉列表用于发现可选引用，服务器仍在 Fleet PUT 时执行最终准入。authentication 保持
key 引用并由服务端解析 Active；Template 则固定用户已经看到的 exact contract。列表
刷新只更新选项和可用性，不更新原 Fleet ETag、已选模板版本或 inputs。

缺权限或列表故障会限制新选择，但不能抹掉旧 Fleet 的引用。编辑表单保留当前值，并将
未修改引用的容量编辑交给原有服务端 admission。这使发现功能不会成为新的授权来源，
也不会把短暂的列表故障变成草稿丢失。

---
status: accepted
date: 2026-09-08
amends: [0005, 0008, 0009, 0018, 0020]
---

# Activate Templates after static validation

Template 发布后静态校验已经通过，却因未接通的外部 conformance 激活流程停在 Ready，
导致用户无法创建 Fleet。决定由 daemon 在当前候选校验成功后自动激活，并在升级后用
相同路径处理已有 Ready。详细状态机、兼容规则与验收由 [spec 0017](../specs/0017-automatic-template-activation.md) 维护。

Active 表示运营者发布的模板允许被引用，不代表完整运行验证通过。conformance 作为
独立的真实测试证据保留；其提交不再控制激活。接受 publication 自动授权激活这一语义，
取消必须另有 `template.attest` 才能使用模板的分权要求；认证、静态校验、retirement、
exact pin、运行时 plan/fencing 与凭据边界继续执行。

激活由数据库事务同时固定版本身份、active head、activation audit 和 outbox，避免
旧扫描覆盖新发布或退休状态。复用既有 immutable audit 记录静态激活来源，使用带版本
命名空间的 opaque ID；旧 attestation 命名字段保留为兼容存储/wire 槽位，不生成虚假的
passing conformance。这样保留历史 pin，不引入重复的激活数据表或手工审批流程。

---
status: accepted
date: 2026-09-09
amends: [0019]
---

# Sync default templates and explicitly update published revisions

默认模板来源库改为当前可信部署目录的投影，每次启动完整校验后原子替换。
使用稳定的 `docker` 和 `kubernetes` key；软件更新直接更新来源内容，移除已撤下的入口。
协议和验收统一维护在 [spec 0021](../specs/0021-default-template-updates.md)。

ARD-0019 的首次导入策略使默认来源永久停留在首次安装的内容。升级只能另起 source key，
最终把旧、新 Docker 和 Kubernetes 同时显示为推荐起点。来源列表只是 publisher 选择的
目录，与不可变 archive、已发布 Revision 和 Generation pin 独立，因此保留旧入口并不是
恢复或 Destroy 的条件。原子同步整个目录集合可去掉专用别名迁移与逐条覆盖的中间状态。

自动覆盖已发布 Profile 会同时改变 bindings/policy 验证的输入，并使 package 升级成为第二个
Profile desired-state 写入者。选择显式 Update：用户检查目标并提交，服务器从精确 base
Revision 复用 write-only bindings 和默认 policy，再进入现有条件发布/校验/自动激活流程。
支持显式替换 input policy，避免 Runner 镜像选项升级后只能重新填写秘密。

Update 是已有 publication 的便捷入口，不引入新 activation 状态机或 Fleet 自动跟随策略。
目标固定 digest，base 固定 incarnation/revision，保留 CAS、幂等和授权边界。没有来源关系
时不猜测模板血缘，只允许用户选择同平台默认源，并由现有 admission 判定兼容性。

代价是启动需要重新校验默认模板，错误的配置目录会阻止启动；这比部分替换来源列表更明确。
旧 archive 和已发布版本继续占用存储，归档回收属于单独生命周期，不和默认入口清理绑定。
更新后 Fleet 仍需显式选择新的 Active，正在执行的 Runner 使用原始材料完成生命周期。

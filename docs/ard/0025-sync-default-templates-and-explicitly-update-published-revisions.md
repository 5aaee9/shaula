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
目标固定 digest，base 固定 incarnation/revision，保留 CAS、幂等和授权边界。

从内置模板发布时，将经过服务器验证的 `source_key` 保存到每个不可变 Template Profile
Revision，读取时返回 `sourceKey`。来源是这个版本的更新入口关联；只记录在 Profile 上会
使旧版本随 head 切换而失去自己的来源，只在浏览器或路由中记住它则无法跨会话和恢复保留。
普通 archive/existing artifact 发布保存空来源，不继承或猜测旧关系。来源记录不外键依赖
可删除的来源列表，撤下默认模板仍保留历史 key 和精确 artifact，不改变运行材料或 Fleet pin。

Update 优先按持久 key 自动定位当前默认模板并固定来源，解决内容升级后 digest 不再匹配、
或同平台有多个模板时重复选择的问题。已知来源撤下或不兼容时明确提示不可用，不自动换成
其他模板；换源通过 New revision 显式发布。只有没有来源的旧版本才使用唯一精确匹配、
再使用平台唯一候选的 review 预选；这不直接写入关联，用户显式提交后才记录所选来源。
候选列表、目标和 base 随 review 一起冻结，手工修改或后台刷新不会再次触发预选。

服务器对新请求验证 source key 与所审阅的 digest、engine 和 platform 相符，来源库变化
导致不匹配时拒绝并要求重新 review，不能偷偷跟随 latest。来源参与普通 PUT/Update 的
幂等身份及 NoOp 比较，因此仅补充来源也能产生新 Revision。有效的已接受请求先重放原结果，
不受后续来源库升级或撤下影响；无来源请求保留既有幂等编码以兼容升级前重试。

一次版本化迁移在来源库同步前，利用旧 catalog 中唯一、精确的 artifact/engine/platform
匹配补录旧版本的来源；缺失、歧义或无法确定 platform 时保持空值。这是只补充来源 metadata
的历史迁移例外，恢复内容关联而不证明旧发布的历史出处；不改 artifact、配置、状态或 pins。
选择一次迁移而非每次启动推断，避免把之后的定制上传误标为默认模板；旧别名被撤下也不
按平台或内容擅自重指向新 key。

代价是启动需要重新校验默认模板，错误的配置目录会阻止启动；这比部分替换来源列表更明确。
旧 archive 和已发布版本继续占用存储，归档回收属于单独生命周期，不和默认入口清理绑定。
新增来源关联需要 schema 迁移和请求兼容；无法确定来源的旧版本仍需一次显式选择或确认。
更新后 Fleet 仍需显式选择新的 Active，正在执行的 Runner 使用原始材料完成生命周期。

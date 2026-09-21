# Runner 最大存活时间

Shaula 服务端对 **GitHub 和 Forgejo 共用**一项硬超时，防止 Runner / 长任务无限运行。
默认从 **资源成功创建**起计时 2 小时；不是任务执行时间，也不是空闲时间。

在 `shaula serve --config …` 使用的 YAML 中配置：

```yaml
runner:
  max_lifetime_secs: 7200 # 默认 2h；秒，必须是正整数
```

NixOS 对应配置：

```nix
services.shaula.settings.runner.max_lifetime_secs = 7200;
```

这是进程级配置，修改后重启 Shaula 生效，适用于该服务管理的所有 Fleet。
不能填 `2h`、负数或 `0`；省略整个 `runner` 段也会启用默认 7200 秒。
它不同于 `execution.operation_timeout_secs`：后者限制单次 Terraform 操作，而非 Runner 寿命。

## 到期行为与风险

**即使 Runner 正在运行任务，到期也会销毁。任务可能失败、中断或暂时仍显示 running。**
如果启动后等待任务 90 分钟，任务执行到 30 分钟左右也可能达到 2 小时上限。
应把需要更长时间的工作负载纳入配置选择；此机制不承诺让每个任务都获得完整 2 小时。

服务端按 reconcile 周期（目前 15 秒）检查到期，受调度、删除预算、Terraform 和外部服务耗时影响，
不是精确的墙钟 kill 定时器。服务停机期间不会执行删除，恢复后用数据库里的原时间继续判断。
时钟回拨可能延迟到期，向前跳变可能提前触发；主机应保持时间同步。

回收按以下顺序推进：

1. 持久化超时回收意图。
2. 用该 Generation 原始 artifact、bindings、provenance 和 state identity 执行 Terraform Destroy。
3. 单独记录资源销毁完成，再清理该 Runner 的远端注册。
4. **两部分都完成**才标记 `Destroyed` 并释放 occupancy。

GitHub 的 `JobStillRunning` 不再阻挡第 2 步；若远端尚不允许删除注册，则只重试第 3 步。
Forgejo 用精确 ID、UUID、名字及 ephemeral 属性核对注册，不会按名字删除冲突对象。
资源删除失败会重试；远端 API 失败不会被当作“已不存在”。
重启或调大超时不会撤销已经记录的回收意图，也不会重复执行已记录成功的资源删除。

硬超时**只放宽 Busy/idle 限制**，不放宽所有权验证、模板 pin、state 校验和效果 fencing。
缺少所有权证据或处于 `Quarantined` 的对象仍需人工处理；配置/凭据无法构建有效 supervisor、
存储/模板材料不可用或平台持续拒绝删除时，不能保证在期限内真正删除。请监控回收错误及保留的 occupancy。
正常缩容、轮换和 decommission 仍走安全清退路径，不因配置上限而立即强杀未到期任务。

## 升级与历史记录

数据库迁移 `m0020_runner_lifetime` 增加成功创建时间、超时意图和资源销毁完成时间。
新 Runner 在成功 Create 结果落库时一次性记录计时起点，后续 readiness、Busy 状态或重启不重置。
升级前有成功结果的历史 Generation 优先采用最早的已完成 Create 时间；若没有该记录，采用迁移时
该行最后观测时间作为一次性保守起点。这种历史回填可能晚于真实创建，但不会每次重启重新延期。
尚未成功创建的记录不猜测起点，继续走原有创建失败恢复流程。

**升级后已经超过配置上限的历史 Runner 可能在首次 reconcile 被销毁。**
有长任务时，请在升级前设置合适的上限并安排维护窗口。

## Forgejo 边界

生产 bootstrap 仍使用官方 `one-job --wait`，不增加任务领取代理。
[非等待模式实验](evidence/forgejo-one-job-2026-09-21/README.md) 显示，领取响应丢失时 `one-job`
可能退出但服务端已分配任务，所以“进程退出”不能独立作为安全回收证据。
正常任务完成后仍按精确注册消失和模板销毁结果回收；硬超时是明确允许中断任务的兜底，
**不是**解决了 [Busy-safe idle drain](forgejo-drain.md)。

本地回归入口：`crates/shaula/tests/runner_lifetime.rs`、`crates/shaula-store/src/tests/forgejo_lifetime.rs`、
`crates/shaula-store/src/tests/runner_lifetime_migration.rs` 和 bootstrap 配置测试。
测试使用真实 SQLite 与故障注入端口，不等同于真实 GitHub / Forgejo / Terraform 平台全链路验收。

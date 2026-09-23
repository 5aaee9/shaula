# Forgejo Jobs 与精确任务结果

Forgejo 的 `/actions/runners/jobs` 只列出等待和运行中的 job。Shaula 保留这些快照：
`waiting → queued`、`running → running`，消失本身只能得到 Unknown，不能推断成功。
Forgejo v15 不返回该接口的 `run_id` 时，UI 显示 Not reported，而不是虚构 run。

## 结果来源

独立历史 worker 使用已观测到的 **非零 `repo_id` + `task_id`**：

1. `GET /api/v1/repositories/{id}` 解析当前 repository，并复核数字 ID 及 Fleet scope。
2. `GET /api/v1/repos/{owner}/{repo}/actions/tasks` 中的 `id` 是 **ActionTask.ID**，
   `status` 是 **Task 自己的状态**，不是整个 workflow run 的结果。
3. 仅精确命中的 `success / failure / cancelled / skipped` 成为 retained result。
   提交时再次验证 Fleet incarnation/revision/mutation fence、scope 与当前 Task ID；
   同一 Task ID 对应多个本地 job/attempt 时拒绝提交。

结果、来源、观测时间、workflow 和 run number 在详情中展示，事件区分
`runner_snapshot` / `task_history`。链接由已配置实例和已核对的 repository 构造，
不跟随上游返回的任意 URL。后续消失、滞后的运行快照或重启不会覆盖已确认终态。

**任务结果不是 Runner 归属证明**。`association_status` 仍为 `unverified`，
`generations` 仍为空；没有 `--handle` 路由、领取代理或 Verified 关联。

## 降级和预算

- 历史读取独立于 Runner tick，不持有 Fleet effect gate 或网络期间的 SQLite writer。
  权限不足、超时、接口错误或没有找到 Task，不影响注册、容量和回收。
- 每个 Fleet/scope 每 30 秒最多保留一批读取预算，SQLite 中的预算跨重启保留。
  每批最多 20 个 Task、4 个 repository；repository 并行读取，各自有 10 秒总预算。
- 每个 repository 最多读 10 页；默认每页 50 条（即最多 500 条历史任务）。
  响应大小、条数、重复 ID 及分页变化均受限。超过窗口不做无界扫描。
- 如果没有观测到运行期的 Task ID、整个短任务落在两次轮询之间、任务记录已清理、
  权限不足、scope 改变或结果在历史窗口外，保留 **Unknown**。不是完整工作流归档。
- 不从 job 名称、时间接近、workflow 汇总结果或 Runner 消失推断结果。
  未确认快照保持既有 30 秒 freshness、保留期限及删除策略。

数据库 forward migration `m0022_forgejo_job_enrichment` 只增加预算列；旧 Jobs JSON
和 observation 缺失新增字段仍可读取。结果是可丢失的读投影，不增加 Runner 清理依赖。

权限见 [最小权限](forgejo-permissions.md)。上游事实来源：
[v15 task response](https://code.forgejo.org/forgejo/forgejo/src/tag/v15.0.0/modules/structs/repo_actions.go)、
[ToActionTask](https://code.forgejo.org/forgejo/forgejo/src/tag/v15.0.0/services/convert/convert.go)、
[runner-jobs](https://code.forgejo.org/forgejo/forgejo/src/tag/v15.0.0/routers/api/v1/shared/runners.go)。
实际验证版本及未覆盖边界见 [实现状态](IMPLEMENTATION_STATUS.md)。

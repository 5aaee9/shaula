# Forgejo + Shaula + Docker 真实生命周期验收

2026-09-21 开始验收，2026-09-22 UTC 完成 S1–S4 审查修复后重跑 `scripts/forgejo-lifecycle/run.mjs`。使用真实 `shaula serve`、SQLite、OIDC Bearer 认证、Forgejo 16.0.4、Runner 13.1.0、Terraform 1.9.8、bundled Docker provider 3.0.2。最终结果见 [report.json](report.json)。这不是直接运行 Runner 的旧 smoke，也不是全平台 conformance attestation。

## 结果

- success / failure：排队需求 → 真实模板 Create → ephemeral 单任务执行 → 远端精确注册消失 → Terraform Destroy → Generation Destroyed / occupancy=0 / 容器和匿名凭据卷消失。
- Jobs：真实 Jobs API 保留 running 观测，不生成 Scale Set ID 或已验证的 Generation 关联；任务消失后保持 Unknown、不猜测结论。
- restart：Busy 时 SIGKILL `shaula serve`，重启后复用原 Generation 并完成回收，无重复 Create。
- idleExpiry / busyExpiry：测试配置最大存活 15 秒，Busy 工作流尝试执行 180 秒；空闲及执行中的 Runner 都按硬超时回收。没有将其当作无损 idle drain 证据。
- Destroy DELETE 注入一次 503；注册 DELETE 注入两次 503。两个失败检查点都重启 daemon，确认 occupancy 在双侧清理完成前不释放，资源先销毁，精确注册随后重试删除。

## 环境与边界

本机系统 `docker` 是 Podman，因此另启隔离的 rootless Docker Engine 29.6.2，经 `/version` 的 Engine/runc 组件确认，并设置最低 API 1.24 兼容固定 provider。所有执行均在该 Engine 的 RootlessKit namespace 中。没有改动系统 Podman 或生产服务配置。

本轮首次验收在 workspace 准备阶段失败；最小复现确认 Terraform 在 Rootless namespace 查询 `registry.terraform.io` 时 DNS 超时，而宿主机同版本、同锁文件初始化成功。最终验收使用 namespace 内 `/root/.terraformrc` 指向宿主机下载的官方 provider 3.0.2 filesystem mirror。`init -lockfile=readonly` 仍校验原锁文件，验收后锁文件逐字节未变；未修改 Terraform 二进制、模板或 provider 包。这个环境条件不证明 namespace 中公共 registry 的在线可达性。

历史阶段二目录为 `/tmp/shaula-lifecycle-1d432fe0-KrcU4K`、`/tmp/shaula-lifecycle-81dc23a6-0bGKGC`，阶段三重跑为 `/tmp/shaula-lifecycle-d0960251-IKUNRL`。本轮失败目录 `/tmp/shaula-lifecycle-5101ff4d-G9MAVu` 与最终通过目录 `/tmp/shaula-lifecycle-7eb48ea2-NwAZXd` 均未提交；最终目录权限为 0700，含 credential-grade 数据库、状态和诊断。这里只提交对应最终重跑的脱敏 `report.json`，八项检查全部通过，包括移除冗余卷集合后的精确凭据卷回收断言。Jobs 写入故障隔离由独立 SQLite/supervisor 回归测试证明，本真实验收没有注入数据库故障。早期失败还暴露并修复了周期扫描错误拒绝 Forgejo token 的生产接线问题，以及管理表单不符合既有 Forgejo HTTP 写入格式的问题；回归测试覆盖扫描不得替代在线认证。

最终验收后，隔离 Docker Engine 的容器和卷列表均为空，随后已停止该独立 Engine；没有清理或改动系统 Podman 资源。

没有验收：云 VM/Kubernetes、最小权限矩阵、无损提前空闲清退、注册响应丢失或真实 GitHub 任务。测试故障代理只拦截 DELETE，既不代理生产任务领取，也不提供 acquisition fence。

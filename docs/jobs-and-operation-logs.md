# Jobs 与 Terraform 执行日志

Jobs 按 Scale Set listener 实际观测的 workflow job 展示。名称、仓库、workflow run 和执行结果缺失时显示 Unknown；不会根据扩容先后猜测 job 与 Runner 的对应关系。可从 Jobs 详情进入关联 Runner 的 Apply/Destroy 历史，或从 **Unassigned runners** 查看预热、创建失败及尚未关联的 Runner。

## 日志与权限

daemon 默认把各次 Create/Destroy 调用的执行记录写入 SQLite，将批准后的 `init`、`plan`、`apply` 文本写入 `data_dir/logs`。UI 将 Create 标为 Apply；Destroy 使用保存的删除 plan，日志的命令 phase 仍为 `apply`。创建前预初始化也有独立执行记录。重试不覆盖已有记录，Workspace 清理不删除这些日志。

Jobs、Runner 和执行状态需要 `fleet.read`；日志正文还需要独立的 `logs.read`。`fleet.write` 不授予读取日志的权限。将该 scope 添加到 [OIDC authorization grant](oidc-deployment.md) 后重启 daemon。页面只保留当前会话的内存数据，不将日志保存到 localStorage；注销或认证失效会清除页面和查询缓存。

归档不是原始 provider stdout/stderr 的副本。日志先过滤已知 JIT、bindings、parameters 和其他凭据，再按发布策略保留进度及批准的诊断；其他内容用 withheld 标记代替。Terraform state、plan JSON、output JSON、环境变量和命令参数不在日志 API 中。执行结果与日志可用性分别显示：例如 Destroy 成功时日志仍可能 partial；日志完整也不证明 workflow job 成功。

可在现有 bootstrap YAML 的顶层设置：

```yaml
operation_logs:
  invocation_bytes: 33554432       # 每次调用 32 MiB
  quota_bytes: 10737418240         # 整个归档 10 GiB
  retention_days: 30
  metadata_retention_days: 90     # 不短于正文保留期
```

日志预算超限时优先回收过期记录，再回收较早封存的日志。活跃日志按预算保留截断内容；页面显示 partial/expired 等可用性。历史 GC 不授权删除 Runner、Terraform state、原始输入或其他恢复证据。

## 将 Apply 日志送入 GitHub Set up job

该交付需要支持 `shaula.setup-info/v1` 的新 shim 镜像以及 `input_contract_version: 2` 的 Template。已有 v1 Template 继续使用原始输入格式。更改 daemon 不会让旧镜像自动获得新 helper。

1. 使用更新后的 `templates/docker/image/Dockerfile` 构建镜像，取得部署环境实际可用的新内容 digest。
2. 按 [Setup Info 模板指南](setup-info-templates.md) 使用 v2 Template 生成器，从 Docker 或 Kubernetes bundled Template 生成独立 source，传入新镜像 pin。
3. 按正常模板库流程导入、验证并发布该 source；选择其新 revision。
4. 配置专属 HTTPS origin，其可信反向代理只转发到下面的独立 loopback listener。该 origin 应与管理 UI 和 worker/state origin 分离；代理不得记录 Authorization 请求头。

```yaml
setup_info:
  listen: 127.0.0.1:9091
  advertised_origin: https://runner-setup.example.com
  capability_ttl_seconds: 3600
  wait_seconds: 60
  requests_per_second: 2
  burst: 4
  max_concurrent_requests: 64
```

这个 listener 只提供 `GET /runner/v1/generations/{id}/setup-info`。daemon 在冻结输入前签发只读 Generation capability；容器主进程 shim 在启动 Runner Listener 前通过受保护 descriptor 获取 Apply 投影，原子合并到 runner 根目录 `.setup_info`。Kubernetes 的等待发生在主容器，避免 init container 等待 Terraform apply 完成形成依赖循环。Destroy 日志仅在管理 UI 中读取。

服务未配置、日志不可用、请求失败或等待超时会跳过交付并继续原本的 JIT 启动流程；JIT 校验本身的失败仍按原契约处理。`.setup_info` 的 Group 为 `Terraform apply (runner provisioning)`。已有分组会保留，已有文件不合法时会保留原文件并放弃本次写入。

## 验证与排查

先在 UI 检查 Runner 的执行尝试、命令结束状态及日志可用性，再检查所选 Template 的 v2 契约和镜像 pin。实际部署应各执行一次 Docker/Kubernetes workflow job，确认首次 `Set up job` 出现上述分组，并在销毁后仍能从 UI 读取对应 Destroy 日志。源码、单元测试和浏览器 fixture 测试不替代这一真实环境验收。

契约与验收要求见 [spec 0019](specs/0019-workflow-jobs-and-operation-logs.md)；架构理由见 [ARD-0023](ard/0023-retain-operation-logs-and-present-workflow-jobs.md)。

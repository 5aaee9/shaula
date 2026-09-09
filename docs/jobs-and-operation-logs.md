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

新的 Docker/Kubernetes Template 直接使用官方 Runner 镜像，并声明 `container_bootstrap_contract: shaula.container-bootstrap/v1`，输入仍为 v1。按 [Setup Info 模板指南](setup-info-templates.md) 导入新的 immutable source/Revision，准备执行主机的 Docker/kubectl 依赖与原 Profile 平台权限。无需自行打包镜像或为 Runner 配置 HTTPS 日志服务。

Terraform 创建尚不能启动 Listener 的资源。Shaula 在本次 apply 结束、归档和 output/state 验证后生成批准的 `.setup_info`，再通过固定宿主 bootstrap 交付并启动。Docker 使用 stopped container；Kubernetes 以 required missing Secret item 阻止 main-container 启动，Shaula 条件发布后冻结 Secret。Setup Info 的 Group 为 `Terraform apply (runner provisioning)`；容器只运行官方 Listener，不下载或加工日志。Destroy 日志仅从管理档案读取。

日志正文不可用或投影失败退化为空数组或固定 unavailable 标记；Docker 诊断文件准备/copy 失败可跳过文件并在复验身份后继续 start；无法安全确认资源或完成必需的 Secret patch/start 则进入 Create 的清理路径，不作为单纯日志缺失忽略。旧 v2 HTTPS listener/能力只保留给已固定旧 artifact 的 retained Generation 兼容；新 publication/Create 必须使用官方镜像与宿主能力，新模板不使用该路径，也不会重写旧 Generation。

## 验证与排查

先在 UI 检查 Runner 的执行尝试、命令结束状态及日志可用性，再检查所选 Template 的 container bootstrap 契约、官方镜像 pin 和宿主工具权限。实际部署应各执行一次 Docker/Kubernetes workflow job，确认首次 `Set up job` 出现上述分组，并在销毁后仍能从 UI 读取对应 Destroy 日志。源码、单元测试和浏览器 fixture 测试不替代这一真实环境验收。

日志与 Jobs 契约见 [spec 0019](specs/0019-workflow-jobs-and-operation-logs.md)；官方容器 bootstrap 见 [spec 0020](specs/0020-official-container-runner-bootstrap.md) 与 [ARD-0024](ard/0024-bootstrap-official-runner-images-outside-containers.md)。

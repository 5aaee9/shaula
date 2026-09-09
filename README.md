# Shaula

**通过 Terraform 管理 GitHub Actions 的按需 Runner。**

Shaula 是一个自托管的 GitHub Actions Runner Scale Set 容量控制器。它根据 job 需求创建临时 Runner，在任务结束并确认可以安全清退后销毁资源，让你在自己的基础设施上运行 CI，并通过 Web UI 管理容量、查看任务状态和排查创建、清理过程中的问题。

## 功能

- **按需扩缩容**：为每个 Fleet（一组独立管理的 Runner）设置容量上下限，根据 GitHub 分配的任务调整 Runner 数量。
- **多 Fleet 管理**：一个服务管理多个组织或仓库的 Scale Set，各自选择 GitHub 认证、基础设施模板和容量策略。
- **Terraform 模板**：提供 Docker 和 Kubernetes 模板来源，支持导入自定义模板、发布固定版本，并在 UI 中配置 Runner 参数。
- **Jobs 视图**：按 workflow job 展示已观测的状态，关联执行它的 Runner，查看创建和清理进度。
- **保留执行日志**：按执行尝试保存 apply / destroy 日志，Runner 销毁后仍可在保留期内排查问题；可选将脱敏后的 apply 输出交付到 GitHub 的 **Set up job** 日志。
- **集中认证与权限**：通过 GitHub App 管理 GitHub 连接，使用 OIDC 登录 Web UI，并分别控制管理操作和日志读取权限。
- **内置 Web UI 与 API**：UI 随 Rust 可执行文件一起发布；提供 Nix 构建环境和 NixOS 服务模块。

项目仍在开发中。Docker 已有真实单 job 冒烟验证，Kubernetes 模板与完整生命周期的验收仍有待完成；Setup Info 需要额外配置新镜像、v2 模板和 HTTPS 交付入口。已实现范围和验证进度见 [实现状态](docs/IMPLEMENTATION_STATUS.md)。

## 开始使用

1. **部署服务**：按 [Nix / NixOS 指南](docs/nix.md) 构建和部署，或按 [开发指南](docs/development.md) 从源码构建。
2. **配置登录**：准备 OIDC Provider、HTTPS 入口和访问授权，参见 [OIDC 部署](docs/oidc-deployment.md)。
3. **连接 GitHub 与基础设施**：在 Web UI 中添加 GitHub App 认证，选择并发布适合目标环境的 Template，再创建 Fleet，配置目标组织或仓库以及容量上下限。
4. **运行与观察**：将 workflow job 路由到对应 Scale Set，在 Fleets 查看容量和健康状态，在 Jobs 查看任务、Runner 和基础设施执行日志。

日志权限、保留策略和 apply 输出交付的配置见 [Jobs 与执行日志](docs/jobs-and-operation-logs.md)。

## 文档

| 你想了解 | 入口 |
| --- | --- |
| 部署、开发与全部文档 | [文档索引](docs/README.md) |
| 构建二进制、开发 UI 和运行检查 | [开发指南](docs/development.md) |
| 构建 Runner 镜像、启用 Setup Info | [Runner 镜像](docs/runner-image.md) · [Setup Info 模板](docs/setup-info-templates.md) |
| 功能完成度与已验证环境 | [实现状态](docs/IMPLEMENTATION_STATUS.md) |
| 产品契约与架构选择 | [Specs](docs/specs/) · [ARD](docs/ard/) |
| Fleet、Runner 等领域术语 | [术语表](docs/CONTEXT.md) |

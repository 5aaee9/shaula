# 使用官方镜像交付 Setup Info

[文档索引](README.md) · [项目首页](../README.md)

新的 Docker/Kubernetes bundled Template 直接使用官方 Runner 镜像，通过 `container_bootstrap_contract: shaula.container-bootstrap/v1` 启用宿主 bootstrap，输入仍为 v1。不需要构建镜像、运行 source generator，或为 Runner 设置独立 HTTPS 日志 origin。

## 部署准备

1. 使用支持该 manifest 能力的 Shaula 版本。执行主机需要 Terraform、锁定 provider，以及平台对应的 Docker/kubectl CLI。当前由 daemon-owned Template Runtime 执行；独立 `shaula job`/HTTP-state 生产集成仍单独记录在实现状态。
2. 确认 [官方 image pin](runner-image.md) 可用。Docker 在原批准 Unix socket 对应的 Engine 中预先拉取；Kubernetes 节点能够拉取官方 digest。
3. 按正常模板库流程导入当前 `templates/docker` 或 `templates/kubernetes` source，校验并发布新的 immutable Revision。Fleet 选择新 Revision 后，只有新 Generation 使用新契约；不修改旧 Revision、运行中资源或 retained state。
4. 执行身份具有原 provider 权限。Docker 使用该 Revision 的批准 socket；Kubernetes 另需对目标 namespace 的 Pod/Secret 读取和 Secret patch 权限，以便验证并条件发布。凭据只供宿主 Terraform/bootstrap 子进程使用，不挂到 Runner，不给 Pod 增加 ServiceAccount token。
5. 配置日志保留和 `logs.read` 授权，见 [Jobs 与执行日志](jobs-and-operation-logs.md)。日志不可用不会自动开放原始 provider 输出。

## 运行顺序

Docker apply 创建 `start=false` 的 container。Shaula 等 apply 完成、验证 output/state 后，从本 invocation 的批准日志生成 JSON，检查 exact container ID 与 ownership，将文件复制到 `/home/runner/.setup_info`，随后启动官方 Listener。

Kubernetes apply 创建初始可变的 JIT Secret 和 Pod。Pod 的 required Secret volume item 引用尚不存在的 `.setup_info`，provider 只等待 Pending；main container 此时不能启动。Shaula 校验 exact Pod/Secret UID、labels 和启动门槛，再通过含 UID/resourceVersion `test` 的一次 JSON Patch 添加 `.setup_info` 并设置 `immutable=true`。容器只读挂载该文件，JIT 通过独立 Secret env reference 进入官方 Listener。

日志缺失、withheld 或投影失败时交付 `[]` 或固定 unavailable 标记，Runner 按原 JIT 流程启动。Docker 诊断文件准备/copy 失败可跳过文件，但 start 前仍复验精确身份。身份错误、必需的 Kubernetes Secret patch 或 start 权限问题/不确定结果属于 Create 失败，进入正常安全清理；不能一边绕过门槛一边显示创建成功。没有容器内 wait、下载、exec 注入或第二次 Terraform apply。Setup Info JSON 最多 768 KiB；Kubernetes 还按实际 JIT 与 JSON 字节总量检查 Secret 1 MiB 上限。超限时先将诊断降级为 `[]`，若连该最小值也无法容纳则按必需 bootstrap 失败处理。

## 兼容与验证

旧 v2 `setup_info_contract` 的 HTTPS listener 和 capability 仅供已经固定旧 artifact 的 retained Generation 兼容；旧模板不能再用于新的 publication/Create，新契约不能与它叠加。升级 daemon 不会改写旧 Generation 或自动迁移旧镜像。历史资料可能描述旧 shim；新部署遵循本指南和 [spec 0020](specs/0020-official-container-runner-bootstrap.md)。

真实部署应验证 Docker/Kubernetes 首个 workflow job 的 **Set up job** 出现 `Terraform apply (runner provisioning)`，内容对应本次 apply，且 Destroy 后仍能在 Jobs UI 读取各次日志。还需覆盖慢 apply、日志不可用、错误身份/UID、patch 冲突、平台权限不足与启动结果不确定。源码和单元测试不能替代这些真实平台检查；已完成范围见 [实现状态](IMPLEMENTATION_STATUS.md)。

升级前需要将仍使用旧自制镜像/旧 bootstrap 契约的 Fleet 切换到新发布的官方镜像 Revision。旧 Revision 和 retained Generation 的读取、恢复、Destroy 仍可用，但旧 Fleet 不能继续用旧模板创建新的 Runner；daemon 不会自动改写其模板或替用户迁移。

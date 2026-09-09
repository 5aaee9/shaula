# 官方 Runner 容器镜像

[文档索引](README.md) · [项目首页](../README.md)

Docker 与 Kubernetes 模板直接使用 GitHub 官方 `ghcr.io/actions/actions-runner` 镜像。Shaula 不构建派生 Runner 镜像，不在镜像中安装 shim、Setup Info helper、启动脚本或其他软件。需要的工具应由 workflow 自己安装；模板不能借此恢复自制 bootstrap 镜像。

当前 bundled Template 固定 Runner **2.337.0** 的官方 multi-platform index：

```text
ghcr.io/actions/actions-runner:2.337.0@sha256:e5496277be5d09bc968b3d64911b74e219ac4a3f2edce956a3ecf9271bea1ef4
```

已核实的 Linux amd64 child manifest 为 `sha256:5036480998280bb21e32ade9fe1b02b493861ac314b62ba1aea320b94f56ec97`。Docker 主机需预先拉取 exact image；Kubernetes 节点需能够拉取该官方 pin。tag 用于辨认版本，digest 决定内容，不能只使用可变 tag 或把自制镜像的 digest 冒充官方 digest。

```sh
docker pull ghcr.io/actions/actions-runner:2.337.0@sha256:e5496277be5d09bc968b3d64911b74e219ac4a3f2edce956a3ecf9271bea1ef4
```

模板直接运行 `/home/runner/bin/Runner.Listener run`，使用官方 `ACTIONS_RUNNER_INPUT_JITCONFIG` 输入。Docker 的 initial container env 保存 JIT；Kubernetes 用 generation Secret 的 `secretKeyRef` 传值。官方 [CommandSettings.cs](https://github.com/actions/runner/blob/v2.337.0/src/Runner.Listener/CommandSettings.cs#L101) 捕获并删除普通环境项，[Runner.cs](https://github.com/actions/runner/blob/v2.337.0/src/Runner.Listener/Runner.cs#L217) 解析 JIT 后创建配置。普通 job 子进程不应继承这个变量；平台 inspect/state 与初始进程环境仍可能含 JIT，不能把 unset 当作内存清零或同域隔离。

Setup Info 在 Shaula 生命周期执行主机上生成。Terraform 创建 stopped Docker container 或被 required Secret key 阻止启动的 Kubernetes Pod；apply 结束后 Shaula 交付 `/home/runner/.setup_info` 再开启启动门槛。容器不下载日志，也不执行等待/处理脚本。具体部署步骤见 [Setup Info 模板指南](setup-info-templates.md)。

更新 Runner 版本应核实新的官方 digest、Listener 输入和 `.setup_info` 行为，更新 manifest/runtime policy 并发布新 Template Revision。旧 Generation 保留原 image/artifact/input/state；不能把更新后的文件当作旧 conformance 的延续。真实首 job、普通 job 环境及安全 Destroy 验收见 [spec 0020](specs/0020-official-container-runner-bootstrap.md)，当前证据边界见 [实现状态](IMPLEMENTATION_STATUS.md)。

升级前需要将仍使用旧自制镜像/旧 bootstrap 契约的 Fleet 切换到新发布的官方镜像 Revision。旧 Revision 和 retained Generation 的读取、恢复、Destroy 仍可用，但旧 Fleet 不能继续用旧模板创建新的 Runner；daemon 不会自动改写其模板或替用户迁移。

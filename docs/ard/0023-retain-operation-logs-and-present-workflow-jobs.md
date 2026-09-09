---
status: accepted
date: 2026-09-09
amends: [0003, 0004, 0006, 0008, 0012, 0013, 0014]
---

# Retain operation logs and present workflow jobs

本决定接受日志保存、Runner setup 信息投递及 Jobs 页面设计；实现进度见 [实现状态](../IMPLEMENTATION_STATUS.md)。容器 bootstrap 部分已由 [ARD-0024](0024-bootstrap-official-runner-images-outside-containers.md) 修订。
状态模型、协议、授权、保留期限、配额与验收的唯一 owner 是
[spec 0019](../specs/0019-workflow-jobs-and-operation-logs.md)。

## Motivation

Terraform Create/Destroy 输出描述了 Runner 怎样创建、何处失败及怎样清理。
现有 phase/reason 只能定位失败阶段，不能代替事后排障所需的执行记录。
Workspace 在清理后会消失，Destroy 也可能经历多次尝试，因此不能把最新进程输出
或仍存活的 Runner 容器作为日志的唯一副本。

用户希望在 GitHub job 的 setup log 中看到 provisioning apply 经过，并在 Web UI
查看 workflow job 状态及相关操作日志。Jobs 的主对象应是 GitHub workflow job；
Generation 是资源生命周期身份，二者不能因为暂时一对一就合并。
预热 Runner 尚无 job，创建失败可能从未接到 job，而 job assignment 可撤销后重派。

## Decision

### Workflow jobs are observations with explicit resource associations

Jobs 页面以已观测的 GitHub workflow job 为主，利用现有 Scale Set 协议提供的元数据。
本次不引入强制 GitHub webhook、额外 REST 抓取、额外权限或完整历史同步依赖。
没有观测到的名称、链接或执行结果保持未知；页面明确其观测覆盖范围。

Scale Set 的 opaque `jobId` 不是 GitHub REST workflow-job numeric ID，不能把它
直接拼成 REST 请求或 GitHub job URL，也不能通过显示转换暗示两者等价。
Job identity、消息去重与 assignment observation 的精确范围由 spec 定义。

`JobCompleted` 的 `canceled` 可能表示未及时领取导致 assignment 被撤销、job
重新排队；不能仅凭该消息把整个 workflow job 宣告最终取消。
[pinned Scale Set oracle](https://github.com/actions/scaleset/blob/cb0405b2d874500e75ae34eff8d582ab75956b45/README.md#job-reassignment)
记录了这种 assignment 重试；UI 保留它与已执行 job completion 的区别。

Generation 始终拥有自己的 Create/Destroy 日志。Job 只有在已有 exact remote
Runner identity 与受信 Generation 记录相符时才能关联这些日志；名称、时间邻近、
队列顺序、acquired capacity 或总量变化都不能用来猜测资源归属。
不完整或矛盾的身份保持未关联；预热和从未分配的失败 Generation 有独立排障入口。

Jobs 是观测读模型。其展示状态、日志关联和日志完成标记都不产生 capacity、Busy、
安全移除、Destroy 或 empty-state completion 权限。
ARD-0003 的 reconciliation-hint 与 GitHub inventory/removal 安全边界继续适用。

### Operation invocations own durable logs

每次 Create/Destroy operation invocation 的执行尝试都有独立、持久的日志身份，
绑定 Generation、operation 与 attempt；Destroy 重试追加新记录，不覆盖先前输出。
Create 的日志重放或缺失不能授权第二次 apply。日志关联不以 job 是否存在为前提。

Template Runtime 在进程输出 drain 处捕获并过滤 stdout/stderr，通过 daemon 所有的
存储接口归档，记录过滤规则版本、完成、截断或采集不完整等事实。
进程退出、timeout 与采集失败不能只留下一个无法区分原因的空文件；详细顺序与
结束协议由 spec 维护。
持久日志不随 Workspace 或 Runner 删除而消失，按独立 retention/quota 管理。

v1 持久化经过过滤、脱敏的完整输出文本，保留非 secret 诊断，不只保存成功摘要。
不新增可回放原始 secret 字节的档案或 raw API；持久化前就执行凭据边界。
面向 operator 与 Runner 的投影按各自受众分别生成，不能因为 operator 可读就
自动公开到 workflow。未知或无法安全公开的片段用明确占位保留省略事实。
Terraform `sensitive`、`-no-color` 或一次字符串替换都不构成完整脱敏保证。
过滤或投影失败时标记 withheld，输出超限时明确标记截断，均不回退为原始字节。

日志系统不改变执行结果，也不能用日志内容推断进程已被 fence 或资源已不存在。
日志存储的准入、配额、降级及恢复规则由 spec 定义，不允许静默假报日志完整。
具体读取授权与保留策略只在 spec 规定；日志的脱敏不把它变为公开数据。

### Container bootstrap is superseded by ARD-0024

最初接受的 v2 方案由自制 Runner shim 使用独立 HTTPS capability，在容器内等待和下载 apply 投影。该方案仅作为旧 v2 artifact 的兼容历史保留；它不再是新 Docker/Kubernetes Template 的实现选项。

[ARD-0024](0024-bootstrap-official-runner-images-outside-containers.md) 改为直接使用官方 Runner 镜像，由 Terraform 建立启动门槛，并由宿主侧生命周期执行端在 apply 完成后生成、交付 Setup Info 再启动 Listener。其新能力与 v1 input、平台固定 bootstrap、失败/恢复边界归 [spec 0020](../specs/0020-official-container-runner-bootstrap.md)。不变的是独立受众、完整 apply 之后交付、Destroy 日志不依赖 Runner、旧 artifact/input/state 不变，以及 GitHub readiness 的独立权威。

## Alternatives and tradeoffs

原方案因平台能力与首 job 竞态拒绝 apply 后 copy/Secret 更新；ARD-0024 已用明确的停止/缺 key 启动门槛、exact identity 检查和单次 Create admission 修订此取舍。运行中 exec 注入、通用更新与第二次 apply 仍不采用。

不把完整 apply 输出作为同次 apply 的初始 Terraform input：完整输出此时尚未
产生，且会把日志绑定到冻结的 plan/input 或敏感 state。共享宿主目录只适合
特定部署，不能作为 Docker/Kubernetes 通用投递契约。

不把所有 stderr/stdout 直接转发到 GitHub：provider 输出可能包含凭据，job 日志
的读者范围不同于 Shaula 的 credential boundary。独立投影增加处理成本，但保留
了 operator 排障内容与 workflow 可见内容之间可审查的边界。

## Rollout

先交付 Runtime 采集、持久档案、受保护投影与 Generation 排障读取，再接入 Jobs
观测读模型及有证据的关联；宿主 bootstrap 投递按 spec 0020 通过新模板版本启用并做真实 job 冒烟。
发布验收须覆盖失败 apply、Destroy 重试、日志存储故障、首 job 时序、投递超时、
跨 Generation 拒绝，以及容器与 Workspace 删除后的历史读取。

当前生产仍使用 daemon-owned local-state lifecycle。新的存储接口由现有 Runtime
调用，未来可随 Runtime 下沉到 Lifecycle Worker；不要求先完成 spec 0010 的全部
worker/state 集成，也不借日志接口提前开放尚未实现的 worker control 通道。
实现状态、兼容 tuple 与测试证据分别更新，不把本 accepted 决定当作已交付证明。

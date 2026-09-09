---
status: accepted
date: 2026-09-09
amends: [0003, 0004, 0006, 0008, 0012, 0013, 0014]
---

# Retain operation logs and present workflow jobs

本决定接受日志保存、Runner setup 信息投递及 Jobs 页面目标设计；这些能力尚未实现。
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

### Bootstrap retrieves the apply projection before starting the Listener

成功交付时，Runner bootstrap shim 在 exec `Runner.Listener` 前，通过独立的、
Runner 可达的 HTTPS 只读通道取回本 Generation 的 Create apply 脱敏投影，
合并已有合法 setup 条目并原子写入 `<runner_root>/.setup_info`。
JSON 使用 `Group`/`Detail`；Shaula 分组不得使用 Runner 保留的 `_internal_` 前缀。
Destroy 日志在 Runner 终止后仍归档并供 UI 排障，不依赖该容器接收它。

下载使用独立、短期、仅限本 Generation 日志读取的 capability。
它不是 worker control、Terraform state 或 management OIDC token，不能换取
JIT、state、其他 Generation 的内容或任何 mutation 权限。
现有仅 loopback 的内部 state/control listener 不对 Runner 开放。
新的 HTTPS 通道具有独立路由和认证边界，不成为管理面 OIDC 的匿名例外。

Descriptor 通过受保护的 bootstrap 文件交付；shim 消费后移除其 staged 副本，
避免传入 Listener argv、普通 job environment、setup log 或遥测。
这只缩小主动暴露面，不承诺同一 Runner Execution Domain 的进程/内存隔离。
若 capability 被该域内进程获取，其授权上限仍只是本 Generation 的脱敏日志。

等待必须有界，读取、解析、合并或落盘失败均降级后继续既有 JIT 启动流程；
日志功能不能把健康 Runner 无限阻塞。JIT 本身的严格验证和失败行为保持独立。
这接受一个明确取舍：任意慢的 apply、首个 job 必有完整日志、零新增启动等待
三者不能同时保证。超出等待预算的 job 可以没有完整 apply 内容，UI 仍保留档案。

Kubernetes 的等待放在 runner 主容器的 shim，init container 只负责 staged
文件复制。当前 pinned provider 等待 Pod phase `Running`；如果 init container
等 apply 完成，会形成 apply 等 Pod 启动、Pod 等日志结束的循环。
主容器中的 shim 已可运行，不要求 `Runner.Listener` 已上线才能完成该 provider gate。
未来的 provider、probe 与 Create 等待策略不能把 Listener 上线或日志到达作为
apply 结束前提；单独的 readiness condition 与 Pod phase 不应混为一谈。

### Templates opt into a versioned bootstrap contract

日志投递通过显式版本化的 system input、manifest 能力声明及 pinned shim/image
约定协作。精确字段和版本由 spec 定义；不使用任意 Fleet 参数或隐藏环境变量
绕过 input admission，也不通过篡改 JIT configuration dictionary 夹带新文件。

Docker 模板在 Create 时上传新的 protected bootstrap descriptor；Kubernetes
模板在既有 bootstrap Secret 中携带并经 init 复制该 descriptor，主容器只挂载
staged volume。这个扩展修订此前 Secret 仅含 JIT 的契约，须经新 revision 的
静态校验与对应平台验收；不新增 daemon 原生平台客户端或事后对象修改路径。

旧 Profile、旧 image 与保留 Generation 继续使用原 input/bootstrap contract，
没有日志投递能力时明确显示不支持。不能改写旧 artifact、pin、protected input、
state 或兼容证据来把它们伪装成支持新协议；新能力从新 Generation 生效。

## Alternatives and tradeoffs

不采用 apply 后 `docker cp`、`kubectl exec` 或 Secret 更新：它们引入原生平台
能力、状态外 mutation 和首 job 竞态。第二次 Terraform apply 也违反现有
Create-once 与 Create/Destroy-only 的生命周期边界。

不把完整 apply 输出作为同次 apply 的初始 Terraform input：完整输出此时尚未
产生，且会把日志绑定到冻结的 plan/input 或敏感 state。共享宿主目录只适合
特定部署，不能作为 Docker/Kubernetes 通用投递契约。

不把所有 stderr/stdout 直接转发到 GitHub：provider 输出可能包含凭据，job 日志
的读者范围不同于 Shaula 的 credential boundary。独立投影增加处理成本，但保留
了 operator 排障内容与 workflow 可见内容之间可审查的边界。

## Rollout

先交付 Runtime 采集、持久档案、受保护投影与 Generation 排障读取，再接入 Jobs
观测读模型及有证据的关联；bootstrap 投递通过新模板版本启用并做真实 job 冒烟。
发布验收须覆盖失败 apply、Destroy 重试、日志存储故障、首 job 时序、投递超时、
跨 Generation 拒绝，以及容器与 Workspace 删除后的历史读取。

当前生产仍使用 daemon-owned local-state lifecycle。新的存储接口由现有 Runtime
调用，未来可随 Runtime 下沉到 Lifecycle Worker；不要求先完成 spec 0010 的全部
worker/state 集成，也不借日志接口提前开放尚未实现的 worker control 通道。
实现状态、兼容 tuple 与测试证据分别更新，不把本 accepted 决定当作已交付证明。

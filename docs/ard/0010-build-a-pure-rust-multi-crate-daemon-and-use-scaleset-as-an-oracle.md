---
status: accepted
date: 2026-09-04
amended-by: [0014, 0022, 0039]
---

# Build a pure Rust multi-crate daemon and use actions/scaleset as a protocol oracle

Shaula v1 保持一个纯 Rust binary 和多 crate workspace，但经 [ADR-0014](0014-run-lifecycle-workers-with-a-database-http-state-backend.md) 修订了 daemon + 每 Generation 一个 worker 的多进程目标模型；该目标的生产集成进度仍见 [implementation status](../IMPLEMENTATION_STATUS.md)，不能据此认定当前 `serve` 已启动独立 worker。`crates/shaula` 仍是唯一 composition root，clap 的生命周期目标包含 `serve` 和内部 `job`；axum 承载管理 OIDC 与独立 worker/state listeners，SeaORM/SQLite 保存 desired/worker/state/lock facts，serde 定义 versioned envelopes，reqwest 实现 GitHub 和内部控制 client，Tokio 负责每个进程的 runtime。

[ARD-0039](0039-user-access-tokens-and-api-client.md) 增加同一 binary 的远程管理 CLI，以及独立的 `shaula-api-types` / `shaula-client`；远程命令只通过管理 HTTP 工作，不初始化 `serve`、数据库或 Terraform。`shaula-forgejo` 是 [ARD-0033](0033-admit-forgejo-through-a-pool-backend-first.md) 对应的独立 Runner Backend adapter，不是新的 Template Platform crate。当前 crate ownership 统一见 [spec 0007](../specs/0007-rust-workspace-architecture.md)。

`github.com/actions/scaleset` 是 Go module，不能作为 Rust crate 链入生产 binary。`crates/shaula-scaleset` 因此以 Rust/reqwest 实现所需协议；固定 commit 的上游 Go SDK、fixtures、测试和 `internal/testserver` 仅作为 test-only protocol oracle。发布物、生产 dependency graph 和运行时都不包含 Go helper、cgo/FFI 或 Go runtime。

后果：

- Shaula 负责维护与上游 public-preview protocol 的兼容性；升级 oracle commit 是显式、reviewed compatibility change，不跟随 `main` 漂移。
- oracle gate 比较 Rust client 与相同 commit Go SDK 的 normalized HTTP method、path、query、headers、body、response/error classification、token refresh、Scale Set/session/JIT/inventory/removal 行为。
- Go 的 `internal/testserver` 受 Go `internal` import rule 限制；test harness 必须在固定上游 checkout/module boundary 内构建并运行 wrapper，不能把它当作普通外部 package import。
- `internal/testserver` 只内建 registration token 与 Actions Service connection bootstrap，其他请求委托给传入 handler；oracle harness 必须为 Scale Set、session、message、JIT、inventory/removal 场景提供脚本化 handler，不能把 testserver 当作完整 GitHub 模拟器。
- oracle/testserver 不能替代真实服务；GitHub App × organization/repository（含多账户路由）的 `github.com` end-to-end matrix 仍是 release gate。[ARD-0022](0022-retire-legacy-github-authentication.md) / spec 0018 已取消 GitHub PAT；旧格式应验证拒绝，而不是保留成功验收要求。Shaula 自签发的个人 Access Token 属于管理 API 凭据，不恢复 GitHub PAT 支持。
- Rust listener 可以拥有更清晰的 durable ACK seam；兼容目标是 wire protocol 和 externally observable outcomes，不是复制 Go `listener.Run` 的内部 callback/ACK ordering。
- `shaula-core` 不依赖 `axum`、`sea-orm`、`reqwest`、`clap`、Terraform implementation 或任何 Kubernetes/Docker crate。HTTP DTO、SeaORM entity 和 Scale Set wire DTO 不得成为 domain types。
- exact Rust toolchain、crate versions/features、TLS backend、Cargo.lock 和 upstream oracle commit 在 Phase 0 固定并进入 SBOM/upgrade policy。

ADR-0014 的同 binary Rust Lifecycle Worker 不改变 Go 排除规则。被拒绝的方案仍是 Rust daemon 加 Go sidecar/FFI：它虽然能直接链接 SDK，却新增 IPC、凭据传递、supervision、跨进程 tracing、双语言供应链和第二个发布制品，不符合已选择的纯 Rust runtime。

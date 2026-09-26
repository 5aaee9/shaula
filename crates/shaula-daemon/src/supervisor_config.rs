use super::*;

impl FleetSupervisor {
    pub(crate) async fn upsert_ownership(
        &self,
        scale_set_id: Option<i64>,
        state: &str,
        attempt_id: Option<String>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .scale_set_upsert(shaula_core::registry::ScaleSetRow {
                fleet_key: self.config.fleet_key.clone(),
                scale_set_id,
                owned_scale_set_id: None,
                name: self.identity.scale_set_name.clone(),
                runner_group: self.identity.runner_group.clone(),
                fingerprint: fingerprint(&self.identity),
                state: state.to_string(),
                attempt_id,
                now,
            })
            .await
    }

    /// Constructor dependencies. The generation's template pin and inputs
    pub fn new(
        deps: FleetSupervisorDeps,
        config: FleetSupervisorConfig,
        identity: shaula_core::github::ScaleSetIdentity,
    ) -> Self {
        let FleetSupervisorDeps {
            store,
            handoff,
            github,
            runtime,
            limits,
        } = deps;
        Self {
            diagnostics: None,
            runner_max_lifetime: shaula_core::runner_lifetime::DEFAULT_MAX_LIFETIME,
            clock: None,
            limits,
            store,
            handoff,
            handoff_github: Arc::clone(&github),
            handoff_authority: (config.auth_profile_key.clone(), config.auth_revision),
            execution_ready: true,
            revision_clients: std::collections::HashMap::new(),
            github,
            runtime,
            config,
            identity,
            runtime_guard: None,
            listener: None,
            setup_info_issuer: None,
        }
    }

    #[must_use]
    pub fn with_runner_max_lifetime(mut self, limit: std::time::Duration) -> Self {
        self.runner_max_lifetime = limit;
        self
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn shaula_core::ports::Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    #[must_use]
    pub fn with_setup_info_issuer(
        mut self,
        issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
    ) -> Self {
        self.setup_info_issuer = issuer;
        self
    }

    #[must_use]
    pub fn with_handoff_github(mut self, github: Arc<dyn GitHubAccessPort>) -> Self {
        self.handoff_github = github;
        self
    }

    #[must_use]
    pub fn with_handoff_authority(mut self, authority: (String, i64)) -> Self {
        self.handoff_authority = authority;
        self
    }

    #[must_use]
    pub fn with_execution_ready(mut self, ready: bool) -> Self {
        self.execution_ready = ready;
        self
    }

    #[must_use]
    pub fn with_revision_clients(
        mut self,
        clients: std::collections::HashMap<(String, i64), Arc<dyn GitHubAccessPort>>,
    ) -> Self {
        self.revision_clients = clients;
        self
    }
}

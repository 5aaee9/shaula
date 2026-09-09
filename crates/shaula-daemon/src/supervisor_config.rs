use super::*;

impl FleetSupervisor {
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
        }
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn shaula_core::ports::Clock>) -> Self {
        self.clock = Some(clock);
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

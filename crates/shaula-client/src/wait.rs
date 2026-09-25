use super::*;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    Fleet,
    Profile,
    TemplatePool,
}
impl ChangeKind {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "fleet" => Ok(Self::Fleet),
            "profile" => Ok(Self::Profile),
            "pool" | "template-pool" => Ok(Self::TemplatePool),
            _ => Err(Error::Invalid("unknown change kind")),
        }
    }
    pub fn route(self) -> &'static str {
        match self {
            Self::Fleet => "fleet-changes",
            Self::Profile => "profile-changes",
            Self::TemplatePool => "template-pool-changes",
        }
    }
}
impl Client {
    pub async fn change(
        &self,
        kind: ChangeKind,
        id: &str,
    ) -> Result<Resource<types::Change>, Error> {
        self.read(&["api", "v1", kind.route(), id], &[]).await
    }
    pub async fn wait(
        &self,
        kind: ChangeKind,
        id: &str,
        timeout: Duration,
    ) -> Result<types::Change, Error> {
        if id.is_empty() {
            return Err(Error::Invalid("NoOp has no change to wait for"));
        }
        let wait = async {
            loop {
                let change = self.change(kind, id).await?.data;
                match change.state.as_str() {
                    "Converged" => return Ok(change),
                    "Rejected" | "Failed" | "Superseded" => {
                        return Err(Error::Tracking(change.state))
                    }
                    _ => tokio::time::sleep(Duration::from_secs(1)).await,
                }
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            Error::Tracking("timeout; accepted operation continues on the server".into())
        })?
    }
}

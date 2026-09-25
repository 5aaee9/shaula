use super::*;
use types::{AccessToken, IssueToken, Page, Secret, TokenIssue};
pub struct AccessTokens<'a>(&'a Client);
impl Client {
    pub fn access_tokens(&self) -> AccessTokens<'_> {
        AccessTokens(self)
    }
}
impl AccessTokens<'_> {
    pub async fn list(&self, query: &Query) -> Result<Resource<Page<AccessToken>>, Error> {
        self.0.read(&["api", "v1", "access-tokens"], query).await
    }
    pub async fn get(&self, id: &str) -> Result<Resource<AccessToken>, Error> {
        self.0.read(&["api", "v1", "access-tokens", id], &[]).await
    }
    pub async fn current(&self) -> Result<Resource<AccessToken>, Error> {
        self.0
            .read(&["api", "v1", "access-tokens", "current"], &[])
            .await
    }
    pub async fn issue(&self, body: &IssueToken, key: String) -> Result<TokenIssue, Error> {
        let body =
            serde_json::to_string(body).map_err(|_| Error::Invalid("invalid issue request"))?;
        let attempt = self
            .0
            .prepare(
                Method::POST,
                &["api", "v1", "access-tokens"],
                &body,
                None,
                Some(key),
            )
            .await?;
        #[derive(serde::Deserialize)]
        struct Wire {
            access_token: AccessToken,
            secret_available: bool,
            token: Option<String>,
        }
        let response = self.0.send(&attempt).await?;
        let status = response.status().as_u16();
        let wire: Wire = super::transport::decode(response).await?.data;
        match (status, wire.secret_available, wire.token) {
            (201, true, Some(secret)) => Ok(TokenIssue::Issued {
                metadata: wire.access_token,
                secret: Secret::new(secret),
            }),
            (200, false, None) => Ok(TokenIssue::RecoveredWithoutSecret(wire.access_token)),
            _ => Err(Error::Protocol),
        }
    }
    pub async fn revoke(&self, id: &str, version: ResourceVersion) -> Result<(), Error> {
        let attempt = self
            .0
            .prepare(
                Method::DELETE,
                &["api", "v1", "access-tokens", id],
                "",
                Some(MutationOptions::update(version)),
                None,
            )
            .await?;
        self.0.no_content(&attempt).await
    }
    pub async fn revoke_current(&self) -> Result<(), Error> {
        let attempt = self
            .0
            .prepare(
                Method::DELETE,
                &["api", "v1", "access-tokens", "current"],
                "",
                None,
                None,
            )
            .await?;
        self.0.no_content(&attempt).await
    }
}

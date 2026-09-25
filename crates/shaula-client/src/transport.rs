use super::*;
use crate::problem::http_error;
use futures::StreamExt;
use serde::de::DeserializeOwned;

impl Client {
    fn url(&self, segments: &[&str]) -> Result<Url, Error> {
        if segments
            .iter()
            .any(|s| s.is_empty() || matches!(*s, "." | "..") || s.chars().any(char::is_control))
        {
            return Err(Error::Invalid("invalid resource identifier"));
        }
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Invalid("invalid origin"))?
            .clear()
            .extend(segments);
        Ok(url)
    }
    pub(crate) async fn read<T: DeserializeOwned>(
        &self,
        path: &[&str],
        query: &[(String, String)],
    ) -> Result<Resource<T>, Error> {
        tokio::time::timeout(self.timeout, self.read_attempts(path, query))
            .await
            .map_err(|_| Error::Transport)?
    }
    async fn read_attempts<T: DeserializeOwned>(
        &self,
        path: &[&str],
        query: &[(String, String)],
    ) -> Result<Resource<T>, Error> {
        let url = self.url(path)?;
        let deadline = tokio::time::Instant::now() + self.timeout;
        for attempt in 0..3 {
            let result = self
                .http
                .get(url.clone())
                .query(query)
                .bearer_auth(self.credential.expose())
                .header("accept", "application/json")
                .timeout(deadline.saturating_duration_since(tokio::time::Instant::now()))
                .send()
                .await;
            match result {
                Ok(response)
                    if (response.status().as_u16() == 429
                        || response.status().is_server_error())
                        && attempt < 2 =>
                {
                    let seconds = response
                        .headers()
                        .get("retry-after")
                        .and_then(|s| s.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(1 << attempt);
                    if seconds > 10
                        || tokio::time::Instant::now() + Duration::from_secs(seconds) >= deadline
                    {
                        return Err(http_error(response).await);
                    }
                    tokio::time::sleep(Duration::from_millis(
                        seconds * 1000 + u64::from(uuid::Uuid::new_v4().as_bytes()[0]),
                    ))
                    .await;
                }
                Ok(response) => {
                    return decode_bounded(
                        response,
                        if path.last() == Some(&"logs") {
                            2 * 1024 * 1024
                        } else {
                            16 * 1024 * 1024
                        },
                    )
                    .await
                }
                Err(_) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))).await
                }
                Err(_) => return Err(Error::Transport),
            }
        }
        Err(Error::Transport)
    }
    pub(crate) async fn health(&self, path: &str) -> Result<bool, Error> {
        let response = self
            .http
            .get(self.url(&[path])?)
            .bearer_auth(self.credential.expose())
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        match response.status().as_u16() {
            200 => Ok(true),
            503 => Ok(false),
            _ => Err(http_error(response).await),
        }
    }
    pub(crate) async fn prepare(
        &self,
        method: Method,
        path: &[&str],
        body: &str,
        options: Option<MutationOptions>,
        key: Option<String>,
    ) -> Result<MutationAttempt, Error> {
        self.url(path)?;
        if !body.is_empty() {
            let _: types::Document = types::Document::parse(body.into())
                .map_err(|_| Error::Invalid("invalid JSON body"))?;
        }
        let identity = match self.session().await?.data.principal {
            Some(principal) => AttemptIdentity::Principal(principal),
            None if !self.credential.expose().starts_with("shaula_pat_") => {
                AttemptIdentity::LegacyOidc(self.credential.clone())
            }
            None => return Err(Error::Unsupported),
        };
        let (precondition, key) = match options {
            Some(o) => (Some(o.precondition), o.idempotency_key),
            None => (
                None,
                key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
        };
        if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid idempotency key"));
        }
        Ok(MutationAttempt {
            origin: self.origin.as_str().into(),
            identity,
            method,
            path: path.iter().map(|s| s.to_string()).collect(),
            body: Secret::new(body.into()),
            precondition,
            key,
            content_type: "application/json",
        })
    }
    pub async fn execute<T: DeserializeOwned>(
        &self,
        attempt: &MutationAttempt,
    ) -> Result<Resource<T>, Error> {
        let response = self.send(attempt).await?;
        decode(response).await
    }
    pub(crate) async fn send(&self, attempt: &MutationAttempt) -> Result<reqwest::Response, Error> {
        if attempt.origin != self.origin.as_str() {
            return Err(Error::Invalid("mutation origin or principal changed"));
        }
        let session = self.session().await?.data;
        let same_identity = match &attempt.identity {
            AttemptIdentity::Principal(principal) => session.principal.as_ref() == Some(principal),
            AttemptIdentity::LegacyOidc(credential) => {
                credential.expose() == self.credential.expose()
            }
        };
        if !same_identity {
            return Err(Error::Invalid("mutation origin or principal changed"));
        }
        let path: Vec<_> = attempt.path.iter().map(String::as_str).collect();
        let mut request = self
            .http
            .request(attempt.method.clone(), self.url(&path)?)
            .bearer_auth(self.credential.expose())
            .header("idempotency-key", &attempt.key)
            .header("content-type", attempt.content_type)
            .body(attempt.body.expose().to_owned());
        match &attempt.precondition {
            Some(WritePrecondition::CreateOnly) => request = request.header("if-none-match", "*"),
            Some(WritePrecondition::Match(v)) => request = request.header("if-match", v.as_str()),
            None => {}
        }
        request.send().await.map_err(|_| Error::Transport)
    }
    pub(crate) async fn no_content(&self, attempt: &MutationAttempt) -> Result<(), Error> {
        let response = self.send(attempt).await?;
        if response.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(http_error(response).await)
        }
    }
    pub(crate) async fn upload(
        &self,
        digest: &str,
        bytes: Vec<u8>,
    ) -> Result<Resource<types::Document>, Error> {
        let response = self
            .http
            .put(self.url(&["api", "v1", "template-artifacts", digest])?)
            .bearer_auth(self.credential.expose())
            .header("content-type", "application/octet-stream")
            .timeout(self.upload_timeout)
            .body(bytes)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        decode(response).await
    }
}

pub(crate) async fn decode<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<Resource<T>, Error> {
    decode_bounded(response, 16 * 1024 * 1024).await
}
async fn decode_bounded<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<Resource<T>, Error> {
    if !response.status().is_success() {
        return Err(http_error(response).await);
    }
    let headers = response.headers();
    let mut version = None;
    for key in ["shaula-resource-version", "etag"] {
        if headers.contains_key(key) {
            if headers.get_all(key).iter().count() == 1 {
                version = headers
                    .get(key)
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| ResourceVersion::parse(s).ok());
            }
            break;
        }
    }
    if !headers
        .get("content-type")
        .and_then(|s| s.to_str().ok())
        .is_some_and(|s| {
            s.split(';')
                .next()
                .is_some_and(|t| t.trim() == "application/json")
        })
    {
        return Err(Error::Protocol);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Transport)?;
        if bytes.len() + chunk.len() > limit {
            return Err(Error::Protocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Resource {
        data: serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?,
        version,
    })
}

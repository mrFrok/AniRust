// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{ApiCode, Error, Result};

/// Base URL the shipped app falls back to when its mirror discovery finds
/// nothing. The app resolves mirrors through a chain of fetchers (a constant,
/// then `config/urls`, then a remote list) because the domain gets blocked;
/// [`ClientBuilder::base_urls`] exists for the same reason.
pub const DEFAULT_BASE_URL: &str = "https://api-s.anixsekai.com/";

/// Value the app sends for the `API-Version` header on `search/releases`.
/// Without it that endpoint answers with the older v1 shape.
pub(crate) const SEARCH_API_VERSION: &str = "v2";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_MAX_RETRIES: u32 = 2;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);

/// Wraps a response payload alongside the `code` every Anixart body carries.
///
/// Flattening lets one deserialization pass both validate the status and
/// produce the payload, instead of parsing the body twice.
#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    code: i32,
    #[serde(flatten)]
    payload: T,
}

/// Payload for endpoints that answer with nothing but a `code` — the various
/// `add`/`delete`/`watch` mutations.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Ack {}

pub struct ClientBuilder {
    base_urls: Vec<String>,
    token: Option<String>,
    timeout: Duration,
    max_retries: u32,
    user_agent: Cow<'static, str>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            base_urls: vec![DEFAULT_BASE_URL.to_owned()],
            token: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            user_agent: Cow::Borrowed(concat!("AniRust/", env!("CARGO_PKG_VERSION"))),
        }
    }
}

impl ClientBuilder {
    /// Candidate base URLs, tried in order when a request fails transiently.
    /// A missing trailing slash is added, since paths resolve relative to it.
    #[must_use]
    pub fn base_urls<I, S>(mut self, urls: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base_urls = urls
            .into_iter()
            .map(|u| {
                let mut s: String = u.into();
                if !s.ends_with('/') {
                    s.push('/');
                }
                s
            })
            .collect();
        self
    }

    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Retries per base URL for transient failures. 0 disables retrying.
    #[must_use]
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    #[must_use]
    pub fn user_agent(mut self, ua: impl Into<Cow<'static, str>>) -> Self {
        self.user_agent = ua.into();
        self
    }

    pub fn build(self) -> Result<Client> {
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .user_agent(self.user_agent.as_ref())
            .build()?;

        // Parsed once at construction so every request avoids re-parsing, and
        // a malformed override is reported here rather than on first use.
        let base_urls = if self.base_urls.is_empty() {
            vec![DEFAULT_BASE_URL.to_owned()]
        } else {
            self.base_urls
        };
        let base_urls = base_urls
            .iter()
            .map(|u| url::Url::parse(u))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Client {
            http,
            base_urls,
            token: Arc::new(RwLock::new(self.token)),
            max_retries: self.max_retries,
        })
    }
}

/// Anixart API client.
///
/// Authentication is a `token` **query parameter**, not a header. Endpoints
/// that need one call [`Client::require_token`] first, so an anonymous client
/// fails with [`Error::Unauthenticated`] rather than a puzzling API refusal.
///
/// Cloning is cheap — the underlying `reqwest::Client` shares its connection
/// pool.
///
/// Clones share the token as well as the pool. Signing in happens once and has
/// to reach every request the application is going to make, including the ones
/// held by tasks that were handed a clone before anyone had signed in.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_urls: Vec<url::Url>,
    token: Arc<RwLock<Option<String>>>,
    max_retries: u32,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_urls", &self.base_urls)
            // Never render the token.
            .field("authenticated", &self.is_authenticated())
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl Client {
    #[must_use]
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Anonymous client against the default base URL.
    pub fn new() -> Result<Self> {
        ClientBuilder::default().build()
    }

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.read_token().is_some()
    }

    #[must_use]
    pub fn token(&self) -> Option<String> {
        self.read_token()
    }

    /// Signs the client in, or out with `None`.
    ///
    /// Takes `&self` because every clone shares one token: a session that only
    /// reached the clone it was set on would be a session that works for
    /// whichever request happened to hold the right copy.
    pub fn set_token(&self, token: Option<String>) {
        match self.token.write() {
            Ok(mut slot) => *slot = token,
            // Only reachable if a thread panicked mid-write, which cannot
            // happen here: nothing runs under this lock but an assignment.
            Err(poisoned) => *poisoned.into_inner() = token,
        }
    }

    fn read_token(&self) -> Option<String> {
        match self.token.read() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(crate) fn require_token(&self) -> Result<()> {
        self.read_token().map(|_| ()).ok_or(Error::Unauthenticated)
    }

    /// Sends a request and answers with its `code`, whatever it is.
    ///
    /// For the endpoints whose success is a code of its own — a friend request
    /// is 2 when it was accepted and 3 when it was sent — where the caller
    /// decides which codes mean what.
    pub(crate) async fn send_code(&self, mut spec: RequestSpec) -> Result<i32> {
        #[derive(Deserialize)]
        struct Code {
            #[serde(default)]
            code: i32,
        }
        spec.any_code = true;
        let answer: Code = self.send(spec).await?;
        Ok(answer.code)
    }

    /// Sends a request by path and answers with the body exactly as it came.
    ///
    /// For capturing responses as test fixtures and for looking at an endpoint
    /// before it has a method of its own. The token goes along when the client
    /// has one; no retries, no mirrors, and no decoding — what is returned is
    /// what the server said, including a refusal.
    pub async fn raw(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
    ) -> Result<String> {
        let base = self.base_urls.first().ok_or(Error::Status {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        })?;
        let mut req = self
            .http
            .request(method, base.join(path.trim_start_matches('/'))?)
            .query(query);
        if let Some(token) = self.read_token() {
            req = req.query(&[("token", token)]);
        }
        Ok(req.send().await?.text().await?)
    }

    /// Starts a request. `path` is relative and must not begin with `/`, so it
    /// resolves against the base URL's trailing slash.
    pub(crate) fn get(&self, path: impl Into<Cow<'static, str>>) -> RequestSpec {
        RequestSpec::new(reqwest::Method::GET, path.into())
    }

    pub(crate) fn post(&self, path: impl Into<Cow<'static, str>>) -> RequestSpec {
        RequestSpec::new(reqwest::Method::POST, path.into())
    }

    /// Sends `spec`, retrying transient failures and falling through to the
    /// next mirror once a mirror's retries are exhausted.
    ///
    /// A deliberate answer from the server — any non-2xx that is not a server
    /// error, or a non-zero body `code` — is final: another mirror is the same
    /// service and would answer identically.
    pub(crate) async fn send<T: DeserializeOwned>(&self, spec: RequestSpec) -> Result<T> {
        debug_assert!(
            !spec.path.starts_with('/'),
            "path must be relative to the base URL: {}",
            spec.path
        );

        let mut last_transient: Option<Error> = None;

        for base in &self.base_urls {
            for attempt in 0..=self.max_retries {
                match self.send_once(base, &spec).await {
                    Ok(payload) => return Ok(payload),
                    Err(err) if err.is_transient() => {
                        tracing::debug!(
                            %base,
                            attempt,
                            path = %spec.path,
                            error = %err,
                            "transient failure"
                        );
                        last_transient = Some(err);
                        if attempt < self.max_retries {
                            tokio::time::sleep(RETRY_BASE_DELAY * 2u32.pow(attempt)).await;
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        // Unreachable with a non-empty base_urls, which the builder guarantees.
        Err(last_transient.unwrap_or(Error::Status {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        }))
    }

    async fn send_once<T: DeserializeOwned>(
        &self,
        base: &url::Url,
        spec: &RequestSpec,
    ) -> Result<T> {
        let url = base.join(&spec.path)?;
        let mut req = self.http.request(spec.method.clone(), url);

        if !spec.query.is_empty() {
            req = req.query(&spec.query);
        }
        if spec.with_token {
            // The endpoint has already established the token exists; an
            // anonymous client reaching here would simply omit it.
            if let Some(token) = self.read_token() {
                req = req.query(&[("token", token)]);
            }
        }
        for (name, value) in &spec.headers {
            req = req.header(*name, value.as_ref());
        }
        req = match &spec.body {
            Body::Empty => req,
            Body::Json(value) => req.json(value),
            Body::Form(fields) => req.form(fields),
            Body::Multipart(upload) => req.multipart(upload.form()?),
        };

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status { status });
        }

        // Read as text rather than `.json()` so a schema change can be
        // diagnosed from the logged body instead of a packet capture.
        let body = response.text().await?;

        // The whole body, code and all, for a caller that judges the code
        // itself. Read without the envelope: the envelope is what takes the
        // code away.
        if spec.any_code {
            return serde_json::from_str::<T>(&body)
                .map_err(|source| Error::Decode { source, body });
        }

        match serde_json::from_str::<Envelope<T>>(&body) {
            Ok(envelope) => {
                let code = ApiCode::from_raw(envelope.code);
                if code.is_success() {
                    Ok(envelope.payload)
                } else {
                    Err(Error::Api { code })
                }
            }
            // The payload did not match, but the status might still explain
            // why — a rejection often replaces the payload with just a code.
            Err(source) => {
                if let Some(code) = parse_code(&body).filter(|c| !c.is_success()) {
                    Err(Error::Api { code })
                } else {
                    Err(Error::Decode { source, body })
                }
            }
        }
    }
}

/// Extracts just `code`, for the case where the payload failed to decode.
fn parse_code(body: &str) -> Option<ApiCode> {
    #[derive(Deserialize)]
    struct CodeOnly {
        code: i32,
    }

    serde_json::from_str::<CodeOnly>(body)
        .ok()
        .map(|c| ApiCode::from_raw(c.code))
}

enum Body {
    Empty,
    Json(serde_json::Value),
    Form(Vec<(&'static str, String)>),
    Multipart(Upload),
}

/// A file going up in a multipart body, kept as bytes rather than as a
/// `reqwest` form: a form is consumed by sending it, and a request that is
/// retried has to be able to build its body again.
pub(crate) struct Upload {
    /// The part's name — what the server reads the file from.
    pub part: &'static str,
    pub file_name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    /// Plain-text parts sent alongside the file.
    pub fields: Vec<(&'static str, String)>,
}

impl Upload {
    fn form(&self) -> Result<reqwest::multipart::Form> {
        let part = reqwest::multipart::Part::bytes(self.bytes.clone())
            .file_name(self.file_name.clone())
            .mime_str(self.mime)?;
        let mut form = reqwest::multipart::Form::new().part(self.part, part);
        for (name, value) in &self.fields {
            form = form.text(*name, value.clone());
        }
        Ok(form)
    }
}

/// A request being assembled. Header and parameter names are `&'static str`
/// because they are always literals at the call sites.
pub(crate) struct RequestSpec {
    method: reqwest::Method,
    path: Cow<'static, str>,
    query: Vec<(&'static str, String)>,
    headers: Vec<(&'static str, Cow<'static, str>)>,
    body: Body,
    with_token: bool,
    /// Hand back the body whatever its `code`, for the few endpoints that
    /// answer success with a code other than 0.
    any_code: bool,
}

impl RequestSpec {
    fn new(method: reqwest::Method, path: Cow<'static, str>) -> Self {
        Self {
            method,
            path,
            query: Vec::new(),
            headers: Vec::new(),
            body: Body::Empty,
            with_token: false,
            any_code: false,
        }
    }

    #[must_use]
    pub(crate) fn query(mut self, name: &'static str, value: impl fmt::Display) -> Self {
        self.query.push((name, value.to_string()));
        self
    }

    /// Adds `name` only when `value` is `Some`, matching the nullable query
    /// parameters the app sends (`sort`, `filter_announce`).
    #[must_use]
    pub(crate) fn query_opt(self, name: &'static str, value: Option<impl fmt::Display>) -> Self {
        match value {
            Some(v) => self.query(name, v),
            None => self,
        }
    }

    #[must_use]
    pub(crate) fn header(
        mut self,
        name: &'static str,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    #[must_use]
    pub(crate) fn json(mut self, value: serde_json::Value) -> Self {
        self.body = Body::Json(value);
        self
    }

    /// Form-urlencoded body, as `auth/*` expects.
    #[must_use]
    pub(crate) fn form<I, V>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, V)>,
        V: Into<String>,
    {
        self.body = Body::Form(fields.into_iter().map(|(k, v)| (k, v.into())).collect());
        self
    }

    /// A multipart body carrying one file.
    #[must_use]
    pub(crate) fn upload(mut self, upload: Upload) -> Self {
        self.body = Body::Multipart(upload);
        self
    }

    /// Appends the client's `token` query parameter.
    #[must_use]
    pub(crate) fn with_token(mut self) -> Self {
        self.with_token = true;
        self
    }
}

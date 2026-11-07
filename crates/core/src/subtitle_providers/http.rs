//! The network boundary for subtitle services.
//!
//! Adapters build `HttpRequest`s and read `HttpResponse`s; they never touch a socket. Tests give
//! them a fake `Transport`, the app gives them `ReqwestTransport`, which only talks HTTPS to the
//! four services' own domains, follows redirects only within them, bounds every body, and never
//! sends or stores cookies.
use super::ProviderProblem;
use serde_json::Value;
use std::{future::Future, pin::Pin, time::Duration};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Domains a subtitle adapter may contact (the domain itself or any subdomain of it).
pub const ALLOWED_DOMAINS: [&str; 4] = ["opensubtitles.com", "subdl.com", "subsource.net", "addic7ed.com"];

pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// The largest response body that will be read.
    pub limit: usize,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self { method: Method::Get, url: url.into(), headers: Vec::new(), body: None, limit: MAX_JSON_BYTES }
    }
    pub fn post_json(url: impl Into<String>, body: &Value) -> Self {
        let mut request = Self { method: Method::Post, url: url.into(), headers: Vec::new(), body: Some(body.to_string().into_bytes()), limit: MAX_JSON_BYTES };
        request.headers.push(("content-type".into(), "application/json".into()));
        request
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }
    pub fn limit(mut self, bytes: usize) -> Self {
        self.limit = bytes;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { status, headers: Vec::new(), body: body.into() }
    }
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn json(&self) -> Result<Value, ProviderProblem> {
        serde_json::from_slice(&self.body).map_err(|_| ProviderProblem::BadResponse)
    }
    /// Seconds to wait, from `Retry-After` (a number of seconds) when the service sent one.
    pub fn retry_after(&self) -> Option<u64> {
        self.header("retry-after").and_then(|v| v.trim().parse().ok()).map(|s: u64| s.min(86_400))
    }
    pub fn is_html(&self) -> bool {
        self.header("content-type").is_some_and(|v| v.to_ascii_lowercase().contains("text/html"))
    }
}

pub trait Transport: Send + Sync {
    /// `Err` means the request could not be completed at all (no connection, timeout, a body over
    /// the limit, an address outside the allowed domains). Any HTTP status is an `Ok` response.
    fn send<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ProviderProblem>>;
}

/// An address an adapter may request: HTTPS, no credentials in the address, on an allowed domain.
pub fn allowed_url(text: &str) -> Option<url::Url> {
    let url = url::Url::parse(text).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let on_allowed_domain = ALLOWED_DOMAINS.iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none() && on_allowed_domain && url.port().is_none_or(|p| p == 443)).then_some(url)
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> anyhow::Result<Self> {
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || allowed_url(attempt.url().as_str()).is_none() {
                attempt.error("Redirect outside the subtitle services")
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .redirect(redirects)
            .user_agent(concat!("CloudstreamDesktop v", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl Transport for ReqwestTransport {
    fn send<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ProviderProblem>> {
        Box::pin(async move {
            let url = allowed_url(&request.url).ok_or(ProviderProblem::Unavailable)?;
            let mut builder = match request.method {
                Method::Get => self.client.get(url),
                Method::Post => self.client.post(url),
            };
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let mut response = builder.send().await.map_err(|_| ProviderProblem::Unavailable)?;
            if response.content_length().is_some_and(|n| n > request.limit as u64) {
                return Err(ProviderProblem::BadResponse);
            }
            let status = response.status().as_u16();
            let headers = response.headers().iter().filter_map(|(n, v)| Some((n.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))).collect();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| ProviderProblem::Unavailable)? {
                if body.len() + chunk.len() > request.limit {
                    return Err(ProviderProblem::BadResponse);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse { status, headers, body })
        })
    }
}

#[cfg(test)]
pub mod fake {
    //! A scripted transport for adapter tests: replies are matched in order of registration by
    //! URL substring, and every request is recorded.
    use super::*;
    use std::sync::{Arc, Mutex};

    type Handler = Box<dyn Fn(&HttpRequest) -> Result<HttpResponse, ProviderProblem> + Send + Sync>;

    #[derive(Default, Clone)]
    pub struct Fake {
        routes: Arc<Mutex<Vec<(String, Handler)>>>,
        pub log: Arc<Mutex<Vec<HttpRequest>>>,
    }

    impl Fake {
        pub fn on(&self, needle: &str, handler: impl Fn(&HttpRequest) -> Result<HttpResponse, ProviderProblem> + Send + Sync + 'static) -> &Self {
            self.routes.lock().unwrap().insert(0, (needle.to_string(), Box::new(handler)));
            self
        }
        pub fn reply(&self, needle: &str, response: HttpResponse) -> &Self {
            self.on(needle, move |_| Ok(response.clone()))
        }
        pub fn requests(&self) -> Vec<HttpRequest> {
            self.log.lock().unwrap().clone()
        }
        pub fn count(&self, needle: &str) -> usize {
            self.requests().iter().filter(|r| r.url.contains(needle)).count()
        }
    }

    impl Transport for Fake {
        fn send<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ProviderProblem>> {
            Box::pin(async move {
                assert!(allowed_url(&request.url).is_some(), "adapters must only request allowed addresses: {}", request.url);
                self.log.lock().unwrap().push(request.clone());
                let routes = self.routes.lock().unwrap();
                match routes.iter().find(|(needle, _)| request.url.contains(needle.as_str())) {
                    Some((_, handler)) => handler(&request),
                    None => Ok(HttpResponse::new(404, b"{}".to_vec())),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_addresses_on_the_four_services_are_allowed() {
        for ok in ["https://api.opensubtitles.com/api/v1/login", "https://dl.subdl.com/subtitle/1.zip", "https://api.subsource.net/api/v1/movies/search", "https://www.addic7ed.com/shows.php", "https://subdl.com/x"] {
            assert!(allowed_url(ok).is_some(), "{ok}");
        }
        for bad in ["http://api.subdl.com/", "https://evil.example/subdl.com", "https://subdl.com.evil.example/", "https://notsubdl.com/", "https://user:pw@subdl.com/", "https://subdl.com:8443/", "file:///etc/passwd", "ftp://subdl.com/"] {
            assert!(allowed_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn response_helpers_read_headers_json_and_retry_after() {
        let response = HttpResponse::new(429, b"{\"a\":1}".to_vec()).with_header("Retry-After", "17").with_header("Content-Type", "text/html; charset=utf-8");
        assert_eq!(response.retry_after(), Some(17));
        assert!(response.is_html());
        assert_eq!(response.json().unwrap()["a"], 1);
        assert_eq!(HttpResponse::new(200, b"nope".to_vec()).json().unwrap_err(), ProviderProblem::BadResponse);
        assert_eq!(HttpResponse::new(429, vec![]).with_header("retry-after", "999999").retry_after(), Some(86_400));
        assert_eq!(HttpResponse::new(429, vec![]).with_header("retry-after", "Wed, 21 Oct").retry_after(), None);
    }
}

//! Narrow HTTP adapter; no provider credentials or callback parameters are logged/reflected.
use super::SsoService;
use crate::{Error, Result};
use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    Request, Response, StatusCode,
};
use std::{
    convert::Infallible,
    net::{SocketAddr, TcpListener},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::oneshot;

pub struct OidcHttpServer {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}
impl OidcHttpServer {
    pub fn start(service: Arc<SsoService>) -> Result<Self> {
        let listener = TcpListener::bind(service.config.listen)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (shutdown, mut stopped) = oneshot::channel();
        let thread=std::thread::Builder::new().name("foks-oidc-http".into()).spawn(move||{
            let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).max_blocking_threads(16).enable_all().build()?;
            runtime.block_on(async move {
                let listener=tokio::net::TcpListener::from_std(listener)?;
                let slots=Arc::new(tokio::sync::Semaphore::new(32));
                let mut connections=tokio::task::JoinSet::new();
                loop {tokio::select! {
                    _=&mut stopped=>break,
                    Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                    accepted=listener.accept()=>{
                        let (stream,_)=accepted?;
                        let Ok(permit)=slots.clone().try_acquire_owned() else {drop(stream);continue;};
                        let service=service.clone();
                        connections.spawn(async move {
                            let _permit=permit;
                            let handler=hyper::service::service_fn(move|req|handle(service.clone(),req));
                            let io=hyper_util::rt::TokioIo::new(stream);
                            let mut builder=hyper::server::conn::http1::Builder::new();
                            builder.keep_alive(false).max_headers(16).max_buf_size(16*1024);
                            let _=tokio::time::timeout(Duration::from_secs(45),builder.serve_connection(io,handler)).await;
                        });
                    }
                }}
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                Ok(())
            })
        })?;
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }
    fn stop(&mut self) -> Result<()> {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)??;
        }
        Ok(())
    }
}
impl Drop for OidcHttpServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
type Body = Full<Bytes>;
fn response(status: StatusCode, body: &'static str) -> Response<Body> {
    let mut response = Response::new(Full::new(Bytes::from_static(body.as_bytes())));
    *response.status_mut() = status;
    for (name, value) in [
        ("content-type", "text/plain; charset=utf-8"),
        ("cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("x-content-type-options", "nosniff"),
    ] {
        response.headers_mut().insert(
            hyper::header::HeaderName::from_static(name),
            hyper::header::HeaderValue::from_static(value),
        );
    }
    response
}
enum Action {
    Start(String),
    Callback {
        state: String,
        code: Option<String>,
        denied: bool,
    },
}
fn action(service: &SsoService, request: &Request<Incoming>) -> Option<Action> {
    if request.method() != hyper::Method::GET
        || request.uri().to_string().len() > 8192
        || request.headers().contains_key("transfer-encoding")
        || request
            .headers()
            .get("content-length")
            .is_some_and(|v| v != "0")
    {
        return None;
    }
    let callback = url::Url::parse(&service.config.redirect_uri).ok()?;
    let start = request.uri().path() == "/oauth2/start";
    if !start && request.uri().path() != callback.path() {
        return None;
    }
    let mut fields = std::collections::BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(request.uri().query()?.as_bytes()) {
        if ![
            "state",
            "code",
            "error",
            "error_description",
            "error_uri",
            "session_state",
            "iss",
        ]
        .contains(&key.as_ref())
            || fields
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return None;
        }
    }
    let state = fields.remove("state")?;
    super::parse_state(&state).ok()?;
    if start {
        return fields.is_empty().then_some(Action::Start(state));
    }
    // RFC 9207 issuer, when present, must be exact. Ignore no caller-chosen issuer URLs.
    if fields
        .get("iss")
        .is_some_and(|iss| iss != &service.config.issuer)
    {
        return None;
    }
    let code = fields.remove("code");
    let error = fields.remove("error");
    if code.is_some() == error.is_some() {
        return None;
    }
    Some(Action::Callback {
        state,
        code,
        denied: error.is_some(),
    })
}
async fn handle(
    service: Arc<SsoService>,
    request: Request<Incoming>,
) -> std::result::Result<Response<Body>, Infallible> {
    let Some(action) = action(&service, &request) else {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            "Invalid authentication request.",
        ));
    };
    // Reserve capacity before spawn_blocking; Tokio's blocking queue must not become an
    // unbounded queue of bearer credentials. Service methods reserve their own separate pools.
    let pool = match &action {
        Action::Start(_) => service.http_starts.clone(),
        Action::Callback { .. } => service.http_completions.clone(),
    };
    // The transport pool is independent from service permits to avoid recursive acquisition.
    let Ok(permit) = pool.try_acquire_owned() else {
        return Ok(response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication is busy. Try again.",
        ));
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match action {
            Action::Start(state) => service.browser_start(&state).map(Some),
            Action::Callback {
                state,
                code,
                denied,
            } => service
                .callback(&state, code.as_deref(), denied)
                .map(|()| None),
        }
    })
    .await;
    Ok(match result {
        Ok(Ok(Some(url))) => {
            if let Ok(location) = hyper::header::HeaderValue::from_str(&url) {
                let mut r = response(
                    StatusCode::SEE_OTHER,
                    "Continue authentication at your identity provider.",
                );
                r.headers_mut().insert(hyper::header::LOCATION, location);
                r
            } else {
                response(
                    StatusCode::BAD_GATEWAY,
                    "Authentication provider unavailable.",
                )
            }
        }
        Ok(Ok(None)) => response(
            StatusCode::OK,
            "Authentication step completed. Return to FOKS to continue.",
        ),
        _ => response(
            StatusCode::BAD_REQUEST,
            "Authentication could not complete. Return to FOKS and check the session status.",
        ),
    })
}

//! Bounded loopback adapter. Request targets, cookies and form values are never logged.
use super::{render, service::*, WebAdminService};
use crate::{Error, Result};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    Request, Response, StatusCode,
};
use std::{
    collections::BTreeMap,
    convert::Infallible,
    net::{SocketAddr, TcpListener},
    sync::{atomic::Ordering, Arc},
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::oneshot;
use zeroize::Zeroizing;
type Body = Full<Bytes>;
const SESSION: &str = "__Host-foks_admin";
const PENDING: &str = "__Host-foks_admin_pending";
struct ReadinessGuard(Arc<WebAdminService>);
impl Drop for ReadinessGuard {
    fn drop(&mut self) {
        self.0.ready.store(false, Ordering::Release);
    }
}
pub struct WebAdminHttpServer {
    address: SocketAddr,
    service: Arc<WebAdminService>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}
impl WebAdminHttpServer {
    pub fn start(service: Arc<WebAdminService>) -> Result<Self> {
        let listener = TcpListener::bind(service.config.listen)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (shutdown, mut stopped) = oneshot::channel();
        let worker = service.clone();
        let (started, ready) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("foks-admin-http".into())
            .spawn(move || {
                let _readiness = ReadinessGuard(worker.clone());
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(16)
                    .enable_all()
                    .build()?;
                let outcome = runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener)?;
                    worker.ready.store(true, Ordering::Release);
                    let _ = started.send(());
                    let slots = Arc::new(tokio::sync::Semaphore::new(32));
                    let jobs = Arc::new(tokio::sync::Semaphore::new(16));
                    let mut connections = tokio::task::JoinSet::new();
                    loop {
                        tokio::select! {
                            _ = &mut stopped => break,
                            Some(_) = connections.join_next(), if !connections.is_empty() => {},
                            accepted = listener.accept() => {
                                let (stream, peer) = accepted?;
                                if !peer.ip().is_loopback() {
                                    continue;
                                }
                                let Ok(permit) = slots.clone().try_acquire_owned() else {
                                    continue;
                                };
                                let service = worker.clone();
                                let jobs = jobs.clone();
                                connections.spawn(async move {
                                    let _permit = permit;
                                    let handler = hyper::service::service_fn(move |req| {
                                        handle(service.clone(), jobs.clone(), req)
                                    });
                                    let mut builder = hyper::server::conn::http1::Builder::new();
                                    builder.keep_alive(false).max_headers(64).max_buf_size(16 * 1024);
                                    let connection = builder.serve_connection(
                                        hyper_util::rt::TokioIo::new(stream), handler,
                                    );
                                    let _ = tokio::time::timeout(Duration::from_secs(20), connection).await;
                                });
                            }
                        }
                    }
                    worker.ready.store(false, Ordering::Release);
                    connections.abort_all();
                    while connections.join_next().await.is_some() {}
                    Ok(())
                });
                runtime.shutdown_timeout(Duration::from_secs(30));
                outcome
            })?;
        ready
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| Error::Config("admin listener failed to initialize"))?;
        Ok(Self {
            address,
            service,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub(crate) fn service(&self) -> Arc<WebAdminService> {
        self.service.clone()
    }
    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }
    fn stop(&mut self) -> Result<()> {
        self.service.ready.store(false, Ordering::Release);
        if let Some(s) = self.shutdown.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            t.join().map_err(|_| Error::Thread)??;
        }
        Ok(())
    }
}
impl Drop for WebAdminHttpServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn response(mut status: StatusCode, body: String) -> Response<Body> {
    let body = if body.len() > 128 * 1024 {
        status = StatusCode::SERVICE_UNAVAILABLE;
        render::page(
            "Temporarily unavailable",
            "<p>Please reopen administration.</p>",
        )
    } else {
        body
    };
    let mut r = Response::new(Full::new(Bytes::from(body)));
    *r.status_mut() = status;
    for (name, value) in [
        ("content-type", "text/html; charset=utf-8"),
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("content-security-policy", "default-src 'none'; script-src 'none'; connect-src 'none'; style-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
        ("permissions-policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    ] {
        r.headers_mut().insert(
            hyper::header::HeaderName::from_static(name),
            hyper::header::HeaderValue::from_static(value),
        );
    }
    r
}
fn failure(status: StatusCode) -> Response<Body> {
    let (title, message) = match status {
        StatusCode::UNAUTHORIZED => ("Signed out", "Open administration from the native application. Recover expired SSO access there before trying again."),
        StatusCode::FORBIDDEN => ("Access denied", "This account or browser request does not have permission."),
        StatusCode::CONFLICT => ("State changed", "The server state was updated. Please reload the page before retrying, or sign out to use a different account."),
        StatusCode::TOO_MANY_REQUESTS => ("Temporarily busy", "Wait for existing requests or sessions to finish, then try again."),
        StatusCode::SERVICE_UNAVAILABLE => ("Temporarily unavailable", "Try again after the host is ready."),
        _ => ("Invalid request", "Reopen administration and use the controls on the page."),
    };
    response(status, render::page(title, &format!("<p>{message}</p>")))
}
fn error(error: Error) -> Response<Body> {
    let status = match error {
        Error::Database(foks_server_db::Error::OperationExpired) => StatusCode::UNAUTHORIZED,
        Error::Database(
            foks_server_db::Error::AuthorizationChanged | foks_server_db::Error::WebWrongUser,
        ) => StatusCode::FORBIDDEN,
        Error::Database(
            foks_server_db::Error::OperationConflict | foks_server_db::Error::Duplicate(_),
        ) => StatusCode::CONFLICT,
        Error::Database(foks_server_db::Error::Capacity(_))
        | Error::WriterQueue
        | Error::ReaderPool => StatusCode::TOO_MANY_REQUESTS,
        Error::Config("admin unavailable" | "admin clock is fenced") => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        Error::Config(_)
        | Error::Protocol(_)
        | Error::Database(foks_server_db::Error::Invalid(_) | foks_server_db::Error::NotFound(_)) => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    failure(status)
}
fn redirect(path: &'static str) -> Response<Body> {
    let mut r = response(StatusCode::SEE_OTHER, String::new());
    r.headers_mut()
        .insert("location", hyper::header::HeaderValue::from_static(path));
    r
}
fn cookie(r: &mut Response<Body>, name: &str, value: Option<&str>, max_age: u64) -> Result<()> {
    let v = format!(
        "{name}={}; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        value.unwrap_or(""),
        if value.is_some() { max_age } else { 0 }
    );
    r.headers_mut().append(
        "set-cookie",
        hyper::header::HeaderValue::from_str(&v).map_err(|_| Error::Config("invalid cookie"))?,
    );
    Ok(())
}

async fn handle(
    service: Arc<WebAdminService>,
    jobs: Arc<tokio::sync::Semaphore>,
    mut req: Request<Incoming>,
) -> std::result::Result<Response<Body>, Infallible> {
    if req.uri().to_string().len() > 8192
        || req
            .headers()
            .iter()
            .map(|(k, v)| k.as_str().len() + v.as_bytes().len() + 4)
            .sum::<usize>()
            > 16 * 1024
        || req.headers().get_all("host").iter().count() != 1
        || req.headers().get("host").and_then(|v| v.to_str().ok())
            != Some(service.config.authority().as_str())
        || req.uri().authority().is_some()
    {
        return Ok(failure(StatusCode::BAD_REQUEST));
    }
    let post = req.method() == hyper::Method::POST;
    if !post && req.method() != hyper::Method::GET {
        return Ok(failure(StatusCode::METHOD_NOT_ALLOWED));
    }
    if post
        && (req.headers().get_all("origin").iter().count() != 1
            || req.headers().get("origin").and_then(|v| v.to_str().ok())
                != Some(service.config.origin().as_str()))
    {
        return Ok(failure(StatusCode::FORBIDDEN));
    }
    if post
        && req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != Some("application/x-www-form-urlencoded")
    {
        return Ok(failure(StatusCode::BAD_REQUEST));
    }
    if req.headers().contains_key("transfer-encoding")
        || req.headers().get("content-length").is_some_and(|v| {
            v.to_str()
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .is_none_or(|n| n > 16 * 1024 || (!post && n != 0))
        })
    {
        return Ok(failure(StatusCode::BAD_REQUEST));
    }
    let Cookies { session, pending } = match parse_cookies(req.headers()) {
        Ok(v) => v,
        Err(e) => return Ok(error(e)),
    };
    let query_values = match parse_fields(req.uri().query().unwrap_or("").as_bytes()) {
        Ok(v) => v,
        Err(e) => return Ok(error(e)),
    };
    let path = req.uri().path().to_owned();
    let Ok(permit) = jobs.try_acquire_owned() else {
        return Ok(failure(StatusCode::TOO_MANY_REQUESTS));
    };
    let mut body = Zeroizing::new(Vec::new());
    let collected = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = req.body_mut().frame().await {
            let frame = frame.map_err(|_| ())?;
            if let Ok(data) = frame.into_data() {
                if body.len() + data.len() > 16 * 1024 || (!post && !data.is_empty()) {
                    return Err(());
                }
                body.extend_from_slice(&data);
            }
        }
        Ok(())
    })
    .await;
    if !matches!(collected, Ok(Ok(()))) {
        return Ok(failure(StatusCode::BAD_REQUEST));
    }
    let fields = match parse_fields(&body) {
        Ok(v) => v,
        Err(e) => return Ok(error(e)),
    };
    let input = Input {
        post,
        path,
        query: query_values,
        fields,
        session,
        pending,
    };
    if let Err(e) = validate_route(&input) {
        return Ok(error(e));
    }
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        dispatch(&service, input).unwrap_or_else(error)
    });
    Ok(
        match tokio::time::timeout(Duration::from_secs(10), task).await {
            Ok(Ok(response)) => response,
            _ => failure(StatusCode::SERVICE_UNAVAILABLE),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_fields_cookies_and_malformed_encoding_fail() {
        for raw in [b"a=1&a=2".as_slice(), b"a=%ff", b"a=%2", b"a=%00"] {
            assert!(parse_fields(raw).is_err());
        }
        let mut h = hyper::HeaderMap::new();
        h.insert(
            "cookie",
            hyper::header::HeaderValue::from_static("a=1; a=2"),
        );
        assert!(parse_cookies(&h).is_err());
    }
    #[test]
    fn cookies_have_host_scope_and_security_headers_apply_to_errors() {
        let mut r = failure(StatusCode::UNAUTHORIZED);
        cookie(&mut r, SESSION, Some(&"a".repeat(64)), 300).unwrap();
        let v = r.headers()["set-cookie"].to_str().unwrap();
        assert!(v.contains("Secure; HttpOnly; SameSite=Strict; Path=/"));
        assert!(!v.contains("Domain"));
        assert_eq!(r.headers()["cache-control"], "no-store");
    }
}

mod parse;
mod routes;
use parse::*;
use routes::dispatch;

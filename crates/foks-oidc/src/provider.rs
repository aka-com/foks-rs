//! Bounded HTTP discovery. Metadata may delegate endpoints, but never to private networks.
use crate::{Error, TokenValidator, MAX_PROVIDER_BYTES, PROVIDER_DEADLINE_MS};
use openidconnect::core::{CoreJwsSigningAlgorithm, CoreProviderMetadata};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};
use url::Url;

#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkPolicy {
    loopback_test: bool,
}
impl NetworkPolicy {
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_test() -> Self {
        Self {
            loopback_test: true,
        }
    }
    pub fn check_url(self, value: &str) -> Result<Url, Error> {
        if value.len() > 4096 {
            return Err(Error::Configuration);
        }
        let url = Url::parse(value).map_err(|_| Error::Configuration)?;
        let local = url.host_str().is_some_and(|h| {
            h == "localhost"
                || h.trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if (url.scheme() != "https" && !(self.loopback_test && local && url.scheme() == "http"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err(Error::Configuration);
        }
        if let Some(host) = url.host_str() {
            if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
                if !self.permits_ip(ip) {
                    return Err(Error::Configuration);
                }
            }
        }
        Ok(url)
    }
    fn permits_ip(self, ip: IpAddr) -> bool {
        if self.loopback_test && ip.is_loopback() {
            return true;
        }
        match ip {
            IpAddr::V4(ip) => public_ipv4(ip),
            IpAddr::V6(ip) => {
                if let Some(v4) = ip.to_ipv4_mapped() {
                    return public_ipv4(v4);
                }
                let s = ip.segments();
                // Global unicast only; exclude documentation, transition and protocol assignments.
                s[0] & 0xe000 == 0x2000
                    && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                    && s[0] != 0x2002
            }
        }
    }
}
fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2))))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}
struct GuardedResolver(NetworkPolicy);
impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.0;
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty()
                || addresses.len() > 64
                || addresses.iter().any(|a| !policy.permits_ip(a.ip()))
            {
                return Err(
                    std::io::Error::other("OIDC address policy rejected destination").into(),
                );
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

pub struct ProviderHttp {
    client: reqwest::blocking::Client,
    policy: NetworkPolicy,
}
impl ProviderHttp {
    pub fn new(policy: NetworkPolicy) -> Result<Self, Error> {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(PROVIDER_DEADLINE_MS))
            .connect_timeout(Duration::from_millis(PROVIDER_DEADLINE_MS))
            .dns_resolver(Arc::new(GuardedResolver(policy)))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self { client, policy })
    }
    pub fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let url = self.policy.check_url(url)?;
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|_| Error::ProviderUnavailable)?;
        if !response.status().is_success() {
            return Err(Error::ProviderUnavailable);
        }
        bounded_response(response)
    }
    pub fn policy(&self) -> NetworkPolicy {
        self.policy
    }
}
fn bounded_response(response: reqwest::blocking::Response) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_PROVIDER_BYTES as u64)
    {
        return Err(Error::DocumentLimit);
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PROVIDER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::ProviderUnavailable)?;
    if bytes.len() > MAX_PROVIDER_BYTES {
        return Err(Error::DocumentLimit);
    }
    Ok(bytes)
}

pub struct Provider {
    pub metadata: CoreProviderMetadata,
    pub validator: TokenValidator,
}
impl Provider {
    /// The discovery URI comes exclusively from the authenticated host configuration.
    pub fn discover(http: &ProviderHttp, config_uri: &str, client_id: &str) -> Result<Self, Error> {
        let bytes = http.get(config_uri)?;
        let metadata: CoreProviderMetadata =
            serde_json::from_slice(&bytes).map_err(|_| Error::Configuration)?;
        http.policy.check_url(metadata.issuer().as_str())?;
        http.policy
            .check_url(metadata.authorization_endpoint().as_str())?;
        http.policy.check_url(
            metadata
                .token_endpoint()
                .ok_or(Error::Configuration)?
                .as_str(),
        )?;
        http.policy.check_url(metadata.jwks_uri().as_str())?;
        if !metadata
            .id_token_signing_alg_values_supported()
            .contains(&CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256)
        {
            return Err(Error::Configuration);
        }
        let jwks = http.get(metadata.jwks_uri().as_str())?;
        let validator = TokenValidator::new(
            metadata.issuer().as_str().to_owned(),
            client_id.to_owned(),
            &jwks,
        )?;
        Ok(Self {
            metadata,
            validator,
        })
    }
}

/// The FOKS server's small start URL must stay on its configured callback origin.
pub fn check_browser_start(
    policy: NetworkPolicy,
    returned: &str,
    redirect_uri: &str,
) -> Result<String, Error> {
    let start = policy.check_url(returned)?;
    let callback = policy.check_url(redirect_uri)?;
    if start.origin() != callback.origin() || start.path() == "/" {
        return Err(Error::Configuration);
    }
    Ok(start.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_cannot_escape_public_or_explicit_loopback_policy() {
        let public = NetworkPolicy::default();
        for address in [
            "http://idp.example/x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://10.0.0.1/x",
            "https://169.254.169.254/x",
            "https://[::ffff:127.0.0.1]/x",
            "https://user:password@idp.example/x",
            "https://idp.example/x#fragment",
        ] {
            assert!(public.check_url(address).is_err(), "{address}");
        }
        assert!(public.check_url("https://idp.example/x").is_ok());
        let test = NetworkPolicy::loopback_test();
        assert!(test.check_url("http://127.0.0.1:1234/discovery").is_ok());
        assert!(test.check_url("http://10.0.0.1/discovery").is_err());
        assert!(check_browser_start(
            public,
            "https://evil.example/login",
            "https://host.example/oauth2/callback"
        )
        .is_err());
        assert!(check_browser_start(
            public,
            "https://host.example/oauth2/start/session",
            "https://host.example/oauth2/callback"
        )
        .is_ok());
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::{io::Write, net::TcpListener};
    fn serve(response: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/document", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buf = [0; 4096];
            assert!(connection.read(&mut buf).unwrap() > 0);
            let _ = connection.write_all(&response);
        });
        (url, thread)
    }
    #[test]
    fn provider_http_refuses_redirects_and_bounds_streamed_documents() {
        let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
        let (url,thread)=serve(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n".to_vec());
        assert_eq!(http.get(&url), Err(Error::ProviderUnavailable));
        thread.join().unwrap();
        let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        response.resize(response.len() + MAX_PROVIDER_BYTES + 1, b'x');
        let (url, thread) = serve(response);
        assert_eq!(http.get(&url), Err(Error::DocumentLimit));
        thread.join().unwrap();
    }
}

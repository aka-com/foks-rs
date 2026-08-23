//! Concrete TCP, TLS, and Snowpack RPC transport.

use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use foks_crypto::device_signing_key_pkcs8;
use foks_proto::SecretSeed;
use foks_rpc::{
    read_probe_response, read_response, read_void_response, write_probe_request,
    DEFAULT_MAX_FRAME_LENGTH,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};

use crate::{authenticated_tls_roots, DeviceCredential, Error, PinnedHost, ProbeTarget, Result};

pub struct FoksClient {
    roots: rustls::RootCertStore,
    timeout: Duration,
    pub(crate) maximum_frame_length: usize,
}

impl Default for FoksClient {
    fn default() -> Self {
        Self::webpki()
    }
}

impl FoksClient {
    pub fn webpki() -> Self {
        Self {
            roots: rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            timeout: Duration::from_secs(15),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn with_roots(roots: rustls::RootCertStore) -> Self {
        Self {
            roots,
            timeout: Duration::from_secs(15),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn probe(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        self.probe_with_stream(target)
    }

    fn probe_with_stream(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let config = self.tls_config(None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        write_probe_request(&mut tls, &target.hostname, 0, None)?;
        read_probe_response(&mut tls, self.maximum_frame_length).map_err(Into::into)
    }

    pub(crate) fn connect_tcp(&self, target: &ProbeTarget) -> Result<TcpStream> {
        let socket_addresses = (target.hostname.as_str(), target.port)
            .to_socket_addrs()
            .map_err(Error::Connect)?
            .collect::<Vec<_>>();
        if socket_addresses.is_empty() {
            return Err(Error::NoAddress(target.address()));
        }
        let mut last_error = None;
        let mut tcp = None;
        for address in socket_addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(stream) => {
                    tcp = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let tcp = tcp.ok_or_else(|| {
            Error::Connect(last_error.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no resolved address")
            }))
        })?;
        tcp.set_read_timeout(Some(self.timeout))
            .map_err(Error::Connect)?;
        tcp.set_write_timeout(Some(self.timeout))
            .map_err(Error::Connect)?;
        Ok(tcp)
    }

    fn tls_config(&self, credential: Option<&DeviceCredential>) -> Result<rustls::ClientConfig> {
        self.tls_config_material(
            &self.roots,
            credential
                .map(|credential| (&credential.seed, credential.certificate_chain.as_slice())),
        )
    }

    pub(crate) fn tls_config_material(
        &self,
        roots: &rustls::RootCertStore,
        credential: Option<(&SecretSeed, &[Vec<u8>])>,
    ) -> Result<rustls::ClientConfig> {
        // Both ring and aws-lc can enter the workspace graph. Name the
        // provider so rustls never has to guess which process default to use.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots.clone());
        match credential {
            None => Ok(builder.with_no_client_auth()),
            Some((seed, certificate_chain)) => {
                if certificate_chain.is_empty() {
                    return Err(Error::CertificateChain);
                }
                let certificates = certificate_chain
                    .iter()
                    .cloned()
                    .map(CertificateDer::from)
                    .collect();
                let mut key = device_signing_key_pkcs8(seed)?;
                let key_bytes = std::mem::take(&mut *key);
                let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_bytes));
                Ok(builder.with_client_auth_cert(certificates, key)?)
            }
        }
    }

    pub(crate) fn connect_tls(
        &self,
        target: &ProbeTarget,
        tcp: TcpStream,
        config: rustls::ClientConfig,
    ) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
        let server_name =
            ServerName::try_from(target.hostname.clone()).map_err(|_| Error::ServerName)?;
        let connection = rustls::ClientConnection::new(Arc::new(config), server_name)?;
        Ok(rustls::StreamOwned::new(connection, tcp))
    }

    pub(crate) fn call(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        credential: Option<&DeviceCredential>,
    ) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(
            &roots,
            credential
                .map(|credential| (&credential.seed, credential.certificate_chain.as_slice())),
        )?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_response(&mut tls, self.maximum_frame_length, 0).map_err(Into::into)
    }

    pub(crate) fn call_with_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(&roots, Some((seed, certificate_chain)))?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_response(&mut tls, self.maximum_frame_length, 0).map_err(Into::into)
    }

    pub(crate) fn call_void(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        credential: &DeviceCredential,
    ) -> Result<()> {
        self.call_void_with_material(
            host,
            target,
            request,
            &credential.seed,
            &credential.certificate_chain,
        )
    }

    pub(crate) fn call_void_with_material(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        request: &[u8],
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<()> {
        let tcp = self.connect_tcp(target)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(&roots, Some((seed, certificate_chain)))?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_void_response(&mut tls, self.maximum_frame_length, 0).map_err(Into::into)
    }

    pub(crate) fn call_after_vhost_selection(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        select_request: &[u8],
        request: &[u8],
    ) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(&roots, None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(select_request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_void_response(&mut tls, self.maximum_frame_length, 0)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_response(&mut tls, self.maximum_frame_length, 1).map_err(Into::into)
    }

    pub(crate) fn call_void_after_vhost_selection(
        &self,
        host: &PinnedHost,
        target: &ProbeTarget,
        select_request: &[u8],
        request: &[u8],
    ) -> Result<()> {
        let tcp = self.connect_tcp(target)?;
        let roots = authenticated_tls_roots(host)?;
        let config = self.tls_config_material(&roots, None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(select_request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_void_response(&mut tls, self.maximum_frame_length, 0)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_void_response(&mut tls, self.maximum_frame_length, 1).map_err(Into::into)
    }
}

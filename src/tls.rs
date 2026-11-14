//! The TLS configuration shared by the HTTPS probe and the Telegram client.
//!
//! One [`ClientConfig`], built once and cached: verification against the certificate
//! roots compiled into the binary (ADR-0024), HTTP/1.1 only. There is no proxy, no
//! way to disable verification, and no client certificate.

use std::sync::{Arc, OnceLock};

use rustls::{ClientConfig, RootCertStore};

/// The process-wide TLS configuration.
///
/// The result — including a failure — is cached, so the roots are read once per process.
pub fn config() -> Result<&'static Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let mut config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Ok(Arc::new(config))
        })
        .as_ref()
        .map_err(Clone::clone)
}

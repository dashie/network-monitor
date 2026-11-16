//! The TLS configuration and connection shared by the HTTPS probe, the Telegram
//! client, and the webhook channel.
//!
//! One [`ClientConfig`], built once and cached: verification against the certificate
//! roots compiled into the binary (ADR-0024), HTTP/1.1 only. There is no proxy, no
//! way to disable verification, and no client certificate.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

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

/// Why a TLS connection could not be opened, and whether waiting can help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectError {
    pub retryable: bool,
    pub reason: String,
}

impl ConnectError {
    fn retryable(reason: String) -> Self {
        ConnectError {
            retryable: true,
            reason,
        }
    }

    fn fatal(reason: String) -> Self {
        ConnectError {
            retryable: false,
            reason,
        }
    }
}

/// Resolve, connect, and complete the TLS handshake within `timeout`.
///
/// The caller supplies the host, the port, and the budget; nothing else is negotiable.
pub fn connect(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<StreamOwned<ClientConnection, TcpStream>, ConnectError> {
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| ConnectError::retryable(format!("cannot resolve {host}: {error}")))?
        .next()
        .ok_or_else(|| ConnectError::retryable(format!("cannot resolve {host}: no address")))?;

    let tcp = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| ConnectError::retryable(format!("cannot connect to {host}: {error}")))?;
    for setting in [
        tcp.set_read_timeout(Some(timeout)),
        tcp.set_write_timeout(Some(timeout)),
    ] {
        setting.map_err(|error| {
            ConnectError::retryable(format!(
                "cannot configure the connection to {host}: {error}"
            ))
        })?;
    }

    let config =
        config().map_err(|reason| ConnectError::fatal(format!("cannot start TLS: {reason}")))?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| ConnectError::fatal(format!("'{host}' is not a server name")))?;
    let connection = ClientConnection::new(Arc::clone(config), server_name)
        .map_err(|reason| ConnectError::fatal(format!("cannot start TLS: {reason}")))?;

    Ok(StreamOwned::new(connection, tcp))
}

//! The webhook channel: one JSON `POST` per message, optionally bearer-authenticated.
//!
//! The URL is supplied by the operator, so the channel prints and reports only
//! `scheme://host[:port]`: the path may itself be a secret. The token is scrubbed from
//! every failure, and a token is refused on an `http` URL by the configuration.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::channel::{Channel, Payload, SendError};
use super::message::{Kind, Message};
use crate::config::{Webhook as Settings, REDACTED_TOKEN};
use crate::probe::http::UrlParts;
use crate::probe::Scheme;

/// The budget of one request: connect, write, and read share it.
const TIMEOUT: Duration = Duration::from_secs(5);
/// The answer is bounded, so a chatty endpoint cannot grow the buffer.
const MAX_RESPONSE: usize = 8 * 1024;

/// The webhook transport of one channel.
#[derive(Clone)]
pub struct Webhook {
    name: String,
    url: UrlParts,
    token: Option<String>,
}

/// The token is a secret: it must never reach a log line through a derived `Debug`.
impl std::fmt::Debug for Webhook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Webhook")
            .field("name", &self.name)
            .field("url", &self.host())
            .field(
                "token",
                &if self.token.is_some() {
                    REDACTED_TOKEN
                } else {
                    "none"
                },
            )
            .finish()
    }
}

impl Webhook {
    pub fn new(settings: &Settings) -> Self {
        Webhook {
            name: settings.name.clone(),
            url: settings.url.clone(),
            token: settings.token.clone(),
        }
    }

    /// `scheme://host[:port]`, never the path a secret may live in.
    fn host(&self) -> String {
        if self.url.port == self.url.scheme.default_port() {
            format!("{}://{}", self.url.scheme.as_str(), self.url.host)
        } else {
            format!(
                "{}://{}:{}",
                self.url.scheme.as_str(),
                self.url.host,
                self.url.port
            )
        }
    }

    fn request(&self, body: &str) -> String {
        let target = if self.url.target.is_empty() {
            "/"
        } else {
            &self.url.target
        };
        let host = if self.url.port == self.url.scheme.default_port() {
            self.url.host.clone()
        } else {
            format!("{}:{}", self.url.host, self.url.port)
        };
        let authorization = self
            .token
            .as_ref()
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        format!(
            "POST {target} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Content-Type: application/json; charset=utf-8\r\n\
             {authorization}\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        )
    }

    fn scrub(&self, error: SendError) -> SendError {
        SendError {
            retryable: error.retryable,
            reason: match &self.token {
                Some(token) if !token.is_empty() => error.reason.replace(token, REDACTED_TOKEN),
                _ => error.reason,
            },
        }
    }

    fn exchange(&self, request: &str) -> Result<(), SendError> {
        match self.url.scheme {
            Scheme::Https => {
                let stream = crate::tls::connect(&self.url.host, self.url.port, TIMEOUT).map_err(
                    |error| SendError {
                        retryable: error.retryable,
                        reason: error.reason,
                    },
                )?;
                exchange_over(stream, request, &self.url)
            }
            Scheme::Http => {
                let stream = connect_plain(&self.url.host, self.url.port)?;
                exchange_over(stream, request, &self.url)
            }
        }
    }

    /// Open the connection the scheme needs — the TLS handshake included — and close it
    /// again without writing a request.
    fn open_connection(&self) -> Result<(), SendError> {
        match self.url.scheme {
            Scheme::Https => crate::tls::connect(&self.url.host, self.url.port, TIMEOUT)
                .map(|_| ())
                .map_err(|error| SendError {
                    retryable: error.retryable,
                    reason: error.reason,
                }),
            Scheme::Http => connect_plain(&self.url.host, self.url.port).map(|_| ()),
        }
    }
}

impl Channel for Webhook {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> Kind {
        Kind::Webhook
    }

    fn open(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn verify(&mut self) -> Result<(), String> {
        self.open_connection()
            .map_err(|error| self.scrub(error).reason)
    }

    fn payload(&self, message: &Message) -> Option<Payload> {
        Some(Payload {
            body: message.json(None),
            media_type: "application/json; charset=utf-8",
        })
    }

    fn send(&mut self, payload: &Payload) -> Result<(), SendError> {
        self.exchange(&self.request(&payload.body))
            .map_err(|error| self.scrub(error))
    }

    fn close(&mut self) {}
}

fn connect_plain(host: &str, port: u16) -> Result<TcpStream, SendError> {
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot resolve {host}: {error}"),
        })?
        .next()
        .ok_or_else(|| SendError {
            retryable: true,
            reason: format!("cannot resolve {host}: no address"),
        })?;
    let stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|error| SendError {
        retryable: true,
        reason: format!("cannot connect to {host}: {error}"),
    })?;
    for setting in [
        stream.set_read_timeout(Some(TIMEOUT)),
        stream.set_write_timeout(Some(TIMEOUT)),
    ] {
        setting.map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot configure the connection to {host}: {error}"),
        })?;
    }
    Ok(stream)
}

fn exchange_over<S: Read + Write>(
    mut stream: S,
    request: &str,
    url: &UrlParts,
) -> Result<(), SendError> {
    let host = &url.host;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot write to {host}: {error}"),
        })?;
    stream.flush().map_err(|error| SendError {
        retryable: true,
        reason: format!("cannot write to {host}: {error}"),
    })?;

    let mut response = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.len() >= MAX_RESPONSE {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(SendError {
                    retryable: true,
                    reason: format!("cannot read from {host}: {error}"),
                })
            }
        }
    }

    let (status, _) = split_response(&response);
    classify(status, host)
}

fn split_response(response: &[u8]) -> (u16, &str) {
    let head_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map_or(response.len(), |index| index + 4);
    let (head, body) = response.split_at(head_end);
    let line_end = head
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(head.len());
    let line = std::str::from_utf8(&head[..line_end]).unwrap_or_default();
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status, std::str::from_utf8(body).unwrap_or_default())
}

/// `2xx` is success; `429`, `5xx`, and a transport failure are worth another attempt;
/// a redirect and any other `4xx` are not.
fn classify(status: u16, host: &str) -> Result<(), SendError> {
    if (200..=299).contains(&status) {
        return Ok(());
    }
    let retryable = status == 429 || (500..=599).contains(&status);
    Err(SendError {
        retryable,
        reason: format!("webhook {host} answered {status}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::message::{Cause, TargetRef};
    use crate::probe::http::parse_url;

    fn settings(token: Option<&str>) -> Settings {
        Settings {
            name: "hook".to_string(),
            url: parse_url("https://hooks.example.test/notify?room=ops").unwrap(),
            token: token.map(str::to_string),
            stabilize: Duration::from_secs(60),
        }
    }

    fn down() -> Message {
        Message::target_down(
            TargetRef::new("icmp://1.1.1.1", Some("cloudflare-dns".to_string())),
            Cause::new("ICMP.NO_REPLY", "no reply within 2000 ms"),
            1_764_000_000_000,
        )
    }

    #[test]
    fn the_payload_is_the_one_line_json_document() {
        let channel = Webhook::new(&settings(None));
        let payload = channel.payload(&down()).expect("every event is rendered");
        assert_eq!(payload.media_type, "application/json; charset=utf-8");
        assert_eq!(payload.body, down().json(None));
        assert!(!payload.body.contains('\n'), "one line on the wire");
    }

    #[test]
    fn the_request_carries_the_path_the_query_and_the_body() {
        let channel = Webhook::new(&settings(None));
        let request = channel.request("{}");
        assert!(request.starts_with("POST /notify?room=ops HTTP/1.1\r\n"));
        assert!(request.contains("Host: hooks.example.test\r\n"));
        assert!(request.contains("Content-Type: application/json; charset=utf-8\r\n"));
        assert!(request.contains("Content-Length: 2\r\n"));
        assert!(!request.contains("Authorization:"));
    }

    #[test]
    fn a_token_becomes_a_bearer_header() {
        let channel = Webhook::new(&settings(Some("s3cret")));
        assert!(channel
            .request("{}")
            .contains("Authorization: Bearer s3cret\r\n"));
    }

    #[test]
    fn a_2xx_is_success_and_a_redirect_is_not_followed() {
        assert!(classify(200, "hooks.example.test").is_ok());
        assert!(classify(204, "hooks.example.test").is_ok());
        let error = classify(301, "hooks.example.test").unwrap_err();
        assert!(!error.retryable);
    }

    #[test]
    fn a_client_error_is_not_retried_and_a_server_error_is() {
        assert!(!classify(400, "hooks.example.test").unwrap_err().retryable);
        assert!(!classify(404, "hooks.example.test").unwrap_err().retryable);
        assert!(classify(429, "hooks.example.test").unwrap_err().retryable);
        assert!(classify(500, "hooks.example.test").unwrap_err().retryable);
    }

    #[test]
    fn the_debug_form_never_shows_the_token_or_the_path() {
        let rendered = format!("{:?}", Webhook::new(&settings(Some("s3cret"))));
        assert!(!rendered.contains("s3cret"), "the token leaked: {rendered}");
        assert!(!rendered.contains("/notify"), "the path leaked: {rendered}");
        assert!(rendered.contains("https://hooks.example.test"));
        assert!(rendered.contains(REDACTED_TOKEN));
    }

    #[test]
    fn a_reported_failure_never_carries_the_token() {
        let channel = Webhook::new(&settings(Some("s3cret")));
        let scrubbed = channel.scrub(SendError {
            retryable: true,
            reason: "cannot reach s3cret".to_string(),
        });
        assert!(!scrubbed.reason.contains("s3cret"));
        assert!(scrubbed.reason.contains(REDACTED_TOKEN));
    }

    #[test]
    fn the_channel_states_its_name_and_kind() {
        let channel = Webhook::new(&settings(None));
        assert_eq!(channel.name(), "hook");
        assert_eq!(channel.kind(), Kind::Webhook);
    }
}

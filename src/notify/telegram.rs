//! The Telegram Bot API client: one HTTPS `POST …/sendMessage` per message.
//!
//! The token travels in the request path, so it is never part of an error: a failure
//! names the host, the status, and the `description` of the answer only.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};

use super::{SendError, Sender};
use crate::config::{Telegram as Settings, REDACTED_TOKEN};

/// The only host this client speaks to (ADR-0024, ADR-0026).
const HOST: &str = "api.telegram.org";
const PORT: u16 = 443;
/// The budget of one request: connect, write, and read share it.
const TIMEOUT: Duration = Duration::from_secs(5);
/// The answer is bounded, so a chatty gateway cannot grow the buffer.
const MAX_RESPONSE: usize = 8 * 1024;

/// The Telegram transport of one channel.
#[derive(Clone)]
pub struct Telegram {
    token: String,
    chat_id: String,
}

/// The token is a secret: it must never reach a log line through a derived `Debug`.
impl std::fmt::Debug for Telegram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Telegram")
            .field("token", &REDACTED_TOKEN)
            .field("chat_id", &self.chat_id)
            .finish()
    }
}

impl Telegram {
    pub fn new(settings: &Settings) -> Self {
        Telegram {
            token: settings.bot_token.clone(),
            chat_id: settings.chat_id.clone(),
        }
    }

    /// The HTTP request that sends `text` to the channel.
    fn request(&self, text: &str) -> String {
        let body = format!(
            "chat_id={}&text={}&disable_web_page_preview=true",
            form_encode(&self.chat_id),
            form_encode(text)
        );
        format!(
            "POST /bot{}/sendMessage HTTP/1.1\r\n\
             Host: {HOST}\r\n\
             Content-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            self.token,
            body.len()
        )
    }

    /// Remove the token from anything this client reports.
    fn scrub(&self, error: SendError) -> SendError {
        SendError {
            retryable: error.retryable,
            reason: if self.token.is_empty() {
                error.reason
            } else {
                error.reason.replace(&self.token, REDACTED_TOKEN)
            },
        }
    }

    /// One request over TLS, classified by the answer.
    fn exchange(&self, request: &str) -> Result<(), SendError> {
        let stream = connect()?;
        let mut stream = stream;

        stream
            .write_all(request.as_bytes())
            .map_err(|error| SendError {
                retryable: true,
                reason: format!("cannot write to {HOST}: {error}"),
            })?;
        stream.flush().map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot write to {HOST}: {error}"),
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
                        reason: format!("cannot read from {HOST}: {error}"),
                    })
                }
            }
        }

        let (status, body) = split_response(&response);
        classify(status, body)
    }
}

impl Sender for Telegram {
    fn send(&mut self, text: &str) -> Result<(), SendError> {
        self.exchange(&self.request(text))
            .map_err(|error| self.scrub(error))
    }
}

/// Open the TLS connection and complete the handshake.
fn connect() -> Result<StreamOwned<ClientConnection, TcpStream>, SendError> {
    let address = (HOST, PORT)
        .to_socket_addrs()
        .map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot resolve {HOST}: {error}"),
        })?
        .next()
        .ok_or_else(|| SendError {
            retryable: true,
            reason: format!("cannot resolve {HOST}: no address"),
        })?;

    let tcp = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|error| SendError {
        retryable: true,
        reason: format!("cannot connect to {HOST}: {error}"),
    })?;
    for setting in [
        tcp.set_read_timeout(Some(TIMEOUT)),
        tcp.set_write_timeout(Some(TIMEOUT)),
    ] {
        setting.map_err(|error| SendError {
            retryable: true,
            reason: format!("cannot configure the connection to {HOST}: {error}"),
        })?;
    }

    let config = crate::tls::config().map_err(|reason| SendError {
        retryable: false,
        reason: format!("cannot start TLS: {reason}"),
    })?;
    let server_name = ServerName::try_from(HOST.to_string()).map_err(|_| SendError {
        retryable: false,
        reason: format!("'{HOST}' is not a server name"),
    })?;
    let connection =
        ClientConnection::new(Arc::clone(config), server_name).map_err(|reason| SendError {
            retryable: false,
            reason: format!("cannot start TLS: {reason}"),
        })?;

    Ok(StreamOwned::new(connection, tcp))
}

/// The status of the answer and its body.
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

/// Turn the answer of the Bot API into a delivery result.
fn classify(status: u16, body: &str) -> Result<(), SendError> {
    let description = description(body);
    let retryable = status == 429 || (500..=599).contains(&status);
    let ok = status == 200 && body.contains("\"ok\":true");
    if ok {
        return Ok(());
    }

    let mut reason = format!("telegram answered {status}");
    if let Some(description) = description {
        reason.push_str(": ");
        reason.push_str(description);
    }
    Err(SendError { retryable, reason })
}

/// The `description` member of a Bot API answer, when the body carries one.
fn description(body: &str) -> Option<&str> {
    let start = body.find("\"description\":\"")? + "\"description\":\"".len();
    let rest = &body[start..];
    let end = rest.find(['"', '\\'])?;
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// `application/x-www-form-urlencoded`: unreserved ASCII unchanged, space as `%20`,
/// every other byte as uppercase `%XX`.
fn form_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            b' ' => encoded.push_str("%20"),
            other => {
                encoded.push('%');
                encoded.push(HEX[(other >> 4) as usize] as char);
                encoded.push(HEX[(other & 0x0f) as usize] as char);
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FileTelegram;
    use crate::notify::fake::FakeSender;
    use crate::notify::{deliver_with, ATTEMPTS};
    use crate::shutdown::Shutdown;

    fn settings() -> Settings {
        let file = FileTelegram {
            bot_token: Some("123456:secret-token".to_string()),
            chat_id: Some("-100200".to_string()),
            stabilize_seconds: None,
        };
        Settings {
            bot_token: file.bot_token.unwrap(),
            chat_id: file.chat_id.unwrap(),
            stabilize: Duration::from_secs(60),
        }
    }

    #[test]
    fn the_encoder_leaves_unreserved_characters_alone() {
        assert_eq!(form_encode("down-1_2.3~"), "down-1_2.3~");
    }

    #[test]
    fn the_encoder_escapes_a_space_a_newline_and_a_parameter() {
        assert_eq!(form_encode("a b"), "a%20b");
        assert_eq!(form_encode("a\nb"), "a%0Ab");
        assert_eq!(form_encode("a&text=b"), "a%26text%3Db");
    }

    #[test]
    fn the_encoder_writes_utf8_bytes_in_uppercase() {
        assert_eq!(form_encode("è"), "%C3%A8");
    }

    #[test]
    fn the_request_carries_the_bot_path_and_the_form_body() {
        let client = Telegram::new(&settings());
        let request = client.request("DOWN site\nCause: TCP.REFUSED");

        assert!(request.starts_with("POST /bot123456:secret-token/sendMessage HTTP/1.1\r\n"));
        assert!(request.contains("Host: api.telegram.org\r\n"));
        assert!(request.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(request.contains("Connection: close\r\n"));
        assert!(request.contains(
            "chat_id=-100200&text=DOWN%20site%0ACause%3A%20TCP.REFUSED&disable_web_page_preview=true"
        ));

        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert!(request.contains(&format!("Content-Length: {}\r\n", body.len())));
    }

    #[test]
    fn a_successful_answer_is_ok() {
        assert!(classify(200, r#"{"ok":true,"result":{"message_id":1}}"#).is_ok());
    }

    #[test]
    fn a_bad_request_is_not_retried() {
        let error = classify(
            400,
            r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#,
        )
        .unwrap_err();
        assert!(!error.retryable);
        assert_eq!(
            error.reason,
            "telegram answered 400: Bad Request: chat not found"
        );
    }

    #[test]
    fn an_unauthorised_answer_is_not_retried() {
        let error = classify(401, r#"{"ok":false,"description":"Unauthorized"}"#).unwrap_err();
        assert!(!error.retryable);
        assert!(error.reason.contains("401"));
    }

    #[test]
    fn a_rate_limit_and_a_server_error_are_retried() {
        assert!(
            classify(429, r#"{"ok":false,"description":"Too Many Requests"}"#)
                .unwrap_err()
                .retryable
        );
        assert!(classify(500, "").unwrap_err().retryable);
        assert!(classify(503, "").unwrap_err().retryable);
    }

    #[test]
    fn a_status_that_contradicts_the_body_is_not_retried() {
        let error = classify(200, r#"{"ok":false,"description":"Bad Request"}"#).unwrap_err();
        assert!(!error.retryable);
    }

    #[test]
    fn the_answer_is_split_into_the_status_and_the_body() {
        let (status, body) = split_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(status, 200);
        assert_eq!(body, "{}");

        let (status, body) = split_response(b"HTTP/1.1 500 Oops\r\n\r\n");
        assert_eq!(status, 500);
        assert_eq!(body, "");
    }

    #[test]
    fn the_client_debug_form_hides_the_token() {
        let rendered = format!("{:?}", Telegram::new(&settings()));
        assert!(
            !rendered.contains("secret-token"),
            "the token leaked: {rendered}"
        );
        assert!(rendered.contains(REDACTED_TOKEN), "unexpected: {rendered}");
    }

    #[test]
    fn a_reported_failure_never_carries_the_token() {
        let client = Telegram::new(&settings());
        let scrubbed = client.scrub(SendError {
            retryable: true,
            reason: "cannot reach bot123456:secret-token".to_string(),
        });
        assert!(
            !scrubbed.reason.contains("secret-token"),
            "the token leaked"
        );
        assert!(scrubbed.reason.contains(REDACTED_TOKEN));
    }

    #[test]
    fn a_retryable_failure_is_attempted_again() {
        let sink = FakeSender::new(vec![
            Err(SendError {
                retryable: true,
                reason: "temporary".to_string(),
            }),
            Err(SendError {
                retryable: true,
                reason: "temporary".to_string(),
            }),
        ]);
        let mut sender = sink.clone();
        let shutdown = Shutdown::new();

        deliver_with(&mut sender, "hello", &shutdown, &[]).expect("the third attempt succeeds");
        assert_eq!(sink.attempts(), 3);
        assert!(sink.sent().iter().all(|text| text == "hello"));
    }

    #[test]
    fn five_failed_attempts_lose_the_message() {
        let failures = std::iter::repeat_with(|| {
            Err(SendError {
                retryable: true,
                reason: "temporary".to_string(),
            })
        })
        .take(ATTEMPTS)
        .collect();

        let sink = FakeSender::new(failures);
        let mut sender = sink.clone();
        let shutdown = Shutdown::new();

        let error = deliver_with(&mut sender, "hello", &shutdown, &[]).unwrap_err();
        assert!(error.retryable);
        assert_eq!(sink.attempts(), ATTEMPTS, "the cap is five attempts");
    }

    #[test]
    fn a_permanent_failure_is_not_retried() {
        let sink = FakeSender::new(vec![Err(SendError {
            retryable: false,
            reason: "bad request".to_string(),
        })]);
        let mut sender = sink.clone();
        let shutdown = Shutdown::new();

        let error = deliver_with(&mut sender, "hello", &shutdown, &[]).unwrap_err();
        assert_eq!(error.reason, "bad request");
        assert_eq!(sink.attempts(), 1);
    }
}

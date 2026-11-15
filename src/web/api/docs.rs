//! `GET /api/v1/docs`: the reference of this API, rendered in the browser from the
//! contract the API already serves.
//!
//! The page loads Swagger UI, vendored so a host with no internet access reads the same
//! reference, and points it at `/api/v1/openapi.json`: the documentation cannot disagree
//! with the document it renders. Nothing here reads the database.

use std::net::TcpStream;

use super::Reply;

/// The page, which names the contract and the two files it loads.
const PAGE: &str = include_str!("../../../assets/api/docs.html");
/// The renderer, at the version and licence `assets/api/swagger-ui/LICENSE` states.
const BUNDLE: &str = include_str!("../../../assets/api/swagger-ui/swagger-ui-bundle.js");
/// The stylesheet of the renderer.
const STYLESHEET: &str = include_str!("../../../assets/api/swagger-ui/swagger-ui.css");
/// The palette of the application on top of it.
const THEME: &str = include_str!("../../../assets/api/theme.css");

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JAVASCRIPT: &str = "text/javascript; charset=utf-8";

/// The page and the files it loads; every other address under the prefix does not exist.
pub(super) fn route(stream: &mut TcpStream, path: &str, reply: Reply) {
    let (content_type, body) = match path {
        "/api/v1/docs" => (HTML, PAGE),
        "/api/v1/docs/swagger-ui.css" => (CSS, STYLESHEET),
        "/api/v1/docs/swagger-ui-bundle.js" => (JAVASCRIPT, BUNDLE),
        "/api/v1/docs/theme.css" => (CSS, THEME),
        _ => {
            super::fail(stream, 404, "NOT_FOUND", None, "no such route", reply);
            return;
        }
    };
    reply.send_text(stream, 200, content_type, body);
}

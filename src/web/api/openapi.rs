//! `GET /api/v1/openapi.json`: the contract of this API.

use std::net::TcpStream;

use super::Reply;

/// The contract of this API, embedded so the binary needs no second file to publish it.
const OPENAPI: &str = include_str!("../../../assets/api/openapi.json");

pub(super) fn route(stream: &mut TcpStream, reply: Reply) {
    reply.send(stream, 200, OPENAPI);
}

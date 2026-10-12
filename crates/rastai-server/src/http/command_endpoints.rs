//! `CommandService.Execute` and `CommandService.ListCommands` over Connect (contracts 3.3, 4.2; layout 11.3). The
//! gate has already run and named the principal; these endpoints only pick the codec, decode the body, call into the
//! command layer and encode the answer. A body that can't be read is refused in the Connect error envelope; a
//! command that fails is an `ExecuteResponse` with `ok: false` and HTTP 200.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use rastai_core::commands::registry::CommandFilter;
use rastai_core::principal::Principal;
use rastai_core::settings::setting_bool;
use rastai_diag::diagnostic::diagnostic;
use rastai_proto::commands::v1::ListCommandsResponse;

use crate::commands::execute::execute_command;
use crate::commands::handlers::command::filtered_commands;
use crate::http::connect::{
    codec_for_content_type, connect_error_response, decode_execute_request, decode_list_commands_request,
    encode_execute_response, encode_list_commands_response, unary_response, unsupported_content_type_response,
};
use crate::server_state::Server;

pub(crate) const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
pub(crate) const LIST_COMMANDS_PATH: &str = "/rastai.commands.v1.CommandService/ListCommands";
/// Whether `ListCommands` includes experimental commands (contracts 11.2).
const EXPERIMENTAL_APIS_SETTING: &str = "plugins.experimental_apis";
const CODE_NO_CREDENTIAL: &str = "RRS-C1002";

pub(crate) async fn execute_endpoint(
    State(server): State<Arc<Server>>,
    Extension(principal): Extension<Option<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(principal) = principal else {
        return connect_error_response(&diagnostic(CODE_NO_CREDENTIAL, &[]));
    };
    let content_type = content_type_of(&headers);
    let Some(codec) = content_type.and_then(codec_for_content_type) else {
        return unsupported_content_type_response(content_type);
    };
    let request = match decode_execute_request(codec, &body) {
        Ok(request) => request,
        Err(refusal) => return connect_error_response(&refusal),
    };
    let response = execute_command(&server, &principal, request).await;
    unary_response(codec, encode_execute_response(codec, &response))
}

/// Every registered command in the section asked for, experimental ones only when `plugins.experimental_apis` is
/// on. Any signed-in caller may list them; running one is what the scope check guards.
pub(crate) async fn list_commands_endpoint(
    State(server): State<Arc<Server>>,
    Extension(principal): Extension<Option<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if principal.is_none() {
        return connect_error_response(&diagnostic(CODE_NO_CREDENTIAL, &[]));
    }
    let content_type = content_type_of(&headers);
    let Some(codec) = content_type.and_then(codec_for_content_type) else {
        return unsupported_content_type_response(content_type);
    };
    let request = match decode_list_commands_request(codec, &body) {
        Ok(request) => request,
        Err(refusal) => return connect_error_response(&refusal),
    };
    let filter = CommandFilter {
        section: request.section,
        include_experimental: experimental_apis_on(&server),
        basic_only: false,
    };
    let response = ListCommandsResponse {
        commands: filtered_commands(&server, &filter),
    };
    unary_response(codec, encode_list_commands_response(codec, &response))
}

/// The request's Content-Type, when it has one that is text.
fn content_type_of(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
}

fn experimental_apis_on(server: &Server) -> bool {
    match server.settings.read() {
        Ok(settings) => setting_bool(&settings, EXPERIMENTAL_APIS_SETTING),
        Err(poisoned) => setting_bool(&poisoned.into_inner(), EXPERIMENTAL_APIS_SETTING),
    }
}

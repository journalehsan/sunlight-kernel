#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std;

extern crate alloc;

mod json_formatter;
mod method;
mod request_builder;
mod response_parser;

pub use json_formatter::pretty_json;
pub use method::{BodyFormat, HttpMethod};
pub use request_builder::{
    build_request, format_url, normalize_url_input, BasicAuthInput, BuiltRequest, KeyValueEntry,
    RequestBuildError, RequestBuildInput,
};
pub use response_parser::{
    describe_fetch_error, parse_response, NoticeSeverity, ParsedResponseDisplay,
};

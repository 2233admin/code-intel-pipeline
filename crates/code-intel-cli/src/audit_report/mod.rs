#[path = "../content_contract.rs"]
mod content_contract;

mod cli;
mod enums;
mod json_helpers;
mod model;
mod registry;
mod render;
mod render_html;
mod validate;

pub(crate) use cli::run_raw;
pub(crate) use model::AuditReport;
pub(crate) use render::render_markdown_section;
pub(crate) use render_html::render_html_document;

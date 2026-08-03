use std::collections::BTreeSet;

use uuid::Uuid;

use super::multimodal::validate_structured_page_request;
use crate::{
    domain::{ImageLimits, ProviderPageAnalysis, StructuredPageRequest},
    errors::{AppError, AppErrorCode, AppResult},
};

pub const PAGE_ANALYSIS_SCHEMA_VERSION: &str = "textbooklens.page-analysis.v1";
pub const MAX_PROVIDER_PAGE_ANALYSIS_BYTES: u32 = 8 * 1024 * 1024;

pub fn validate_structured_request(
    expected_book_id: Uuid,
    request: &StructuredPageRequest,
    limits: ImageLimits,
) -> AppResult<()> {
    if request.schema_version != PAGE_ANALYSIS_SCHEMA_VERSION
        || request.max_output_bytes == 0
        || request.max_output_bytes > MAX_PROVIDER_PAGE_ANALYSIS_BYTES
    {
        return Err(invalid_input());
    }
    validate_structured_page_request(expected_book_id, request, limits)
}

pub fn decode_provider_page_analysis(
    bytes: &[u8],
    expected_schema_version: &str,
    max_output_bytes: u32,
) -> AppResult<ProviderPageAnalysis> {
    if expected_schema_version != PAGE_ANALYSIS_SCHEMA_VERSION
        || max_output_bytes == 0
        || max_output_bytes > MAX_PROVIDER_PAGE_ANALYSIS_BYTES
    {
        return Err(invalid_input());
    }
    let byte_limit = usize::try_from(max_output_bytes).map_err(|_| invalid_input())?;
    if bytes.len() > byte_limit {
        return Err(invalid_input());
    }

    let decoded =
        serde_json::from_slice::<ProviderPageAnalysis>(bytes).map_err(|_| invalid_input())?;
    if decoded.schema_version != expected_schema_version {
        return Err(invalid_input());
    }
    Ok(decoded)
}

pub fn validate_provider_page_batch(
    analysis: &ProviderPageAnalysis,
    expected_page_count: usize,
) -> AppResult<()> {
    if expected_page_count == 0 || analysis.pages.len() != expected_page_count {
        return Err(invalid_input());
    }

    let mut page_numbers = BTreeSet::new();
    if analysis
        .pages
        .iter()
        .any(|page| !page_numbers.insert(page.page_number))
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

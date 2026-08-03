use uuid::Uuid;

use super::{
    multimodal::stage_vision_asset,
    structured::{
        MAX_PROVIDER_PAGE_ANALYSIS_BYTES, PAGE_ANALYSIS_SCHEMA_VERSION,
        decode_provider_page_analysis, validate_structured_request,
    },
};
use crate::domain::{ImageLimits, ImageMime, PageAnalysisBlockKind, StructuredPageRequest};

#[test]
fn strict_decoder_returns_versioned_untrusted_domain_results() {
    let decoded = decode_provider_page_analysis(
        valid_response().as_bytes(),
        PAGE_ANALYSIS_SCHEMA_VERSION,
        4_096,
    )
    .unwrap();

    assert_eq!(decoded.schema_version, PAGE_ANALYSIS_SCHEMA_VERSION);
    assert_eq!(decoded.pages.len(), 1);
    assert_eq!(decoded.pages[0].page_number, 7);
    assert_eq!(
        decoded.pages[0].blocks[0].kind,
        PageAnalysisBlockKind::Paragraph
    );
    assert_eq!(decoded.pages[0].blocks[0].plain_text, "bounded text");
}

#[test]
fn strict_decoder_caps_bytes_before_attempting_json_decode() {
    let over_limit = vec![b'{'; 65];
    assert!(decode_provider_page_analysis(&over_limit, PAGE_ANALYSIS_SCHEMA_VERSION, 64).is_err());
    assert!(
        decode_provider_page_analysis(
            valid_response().as_bytes(),
            PAGE_ANALYSIS_SCHEMA_VERSION,
            MAX_PROVIDER_PAGE_ANALYSIS_BYTES + 1,
        )
        .is_err()
    );
    assert!(
        decode_provider_page_analysis(
            valid_response().as_bytes(),
            PAGE_ANALYSIS_SCHEMA_VERSION,
            0,
        )
        .is_err()
    );
}

#[test]
fn strict_decoder_rejects_unknown_fields_and_schema_version_drift() {
    let unknown_top_level = valid_response().replace(
        r#""pages":["#,
        r#""providerPayload":"must-not-pass","pages":["#,
    );
    assert!(
        decode_provider_page_analysis(
            unknown_top_level.as_bytes(),
            PAGE_ANALYSIS_SCHEMA_VERSION,
            4_096,
        )
        .is_err()
    );

    let unknown_nested = valid_response().replace(
        r#""plainText":"bounded text""#,
        r#""plainText":"bounded text","confidence":0.99"#,
    );
    assert!(
        decode_provider_page_analysis(
            unknown_nested.as_bytes(),
            PAGE_ANALYSIS_SCHEMA_VERSION,
            4_096,
        )
        .is_err()
    );

    let wrong_version = valid_response().replace(
        PAGE_ANALYSIS_SCHEMA_VERSION,
        "textbooklens.page-analysis.v2",
    );
    assert!(
        decode_provider_page_analysis(
            wrong_version.as_bytes(),
            PAGE_ANALYSIS_SCHEMA_VERSION,
            4_096,
        )
        .is_err()
    );
}

#[test]
fn structured_request_validates_schema_limits_and_page_ownership_locally() {
    let book_id = Uuid::new_v4();
    let limits = image_limits();
    let request = structured_request(book_id, PAGE_ANALYSIS_SCHEMA_VERSION, 4_096, limits);
    assert!(validate_structured_request(book_id, &request, limits).is_ok());
    assert!(validate_structured_request(Uuid::new_v4(), &request, limits).is_err());

    let request = structured_request(book_id, "textbooklens.page-analysis.v2", 4_096, limits);
    assert!(validate_structured_request(book_id, &request, limits).is_err());

    let request = structured_request(
        book_id,
        PAGE_ANALYSIS_SCHEMA_VERSION,
        MAX_PROVIDER_PAGE_ANALYSIS_BYTES + 1,
        limits,
    );
    assert!(validate_structured_request(book_id, &request, limits).is_err());
}

fn valid_response() -> String {
    format!(
        r#"{{"schemaVersion":"{PAGE_ANALYSIS_SCHEMA_VERSION}","pages":[{{"pageNumber":7,"blocks":[{{"ordinal":0,"kind":"paragraph","plainText":"bounded text","bounds":{{"x":0.1,"y":0.2,"width":0.3,"height":0.4}},"latex":null,"tableCells":null,"visualDescription":null}}]}}]}}"#
    )
}

fn structured_request(
    book_id: Uuid,
    schema_version: &str,
    max_output_bytes: u32,
    limits: ImageLimits,
) -> StructuredPageRequest {
    StructuredPageRequest {
        model: "fixture-model".to_owned(),
        pages: vec![
            stage_vision_asset(
                book_id,
                Uuid::new_v4(),
                ImageMime::Png,
                10,
                10,
                png(10, 10),
                limits,
            )
            .unwrap(),
        ],
        schema_version: schema_version.to_owned(),
        max_output_bytes,
    }
}

fn image_limits() -> ImageLimits {
    ImageLimits {
        max_images: 2,
        max_encoded_bytes_each: 1_024,
        max_total_encoded_bytes: 2_048,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes
}

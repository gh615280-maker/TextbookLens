use crate::{
    ai::structured::PAGE_ANALYSIS_SCHEMA_VERSION,
    domain::{
        IndexReviewReason, ProviderPageAnalysis, UntrustedNormalizedRect, UntrustedPageAnalysis,
    },
};

use super::validator::{
    BatchValidationFailure, MAX_VALIDATION_RESPONSE_BYTES, PageValidationFailure,
    RequestedPageValidation, validate_batch,
};

fn fixture(source: &str) -> ProviderPageAnalysis {
    serde_json::from_str(source).unwrap()
}

fn request(page_number: u32) -> Vec<RequestedPageValidation> {
    vec![RequestedPageValidation {
        page_number,
        local_text: None,
    }]
}

#[test]
fn exact_schema_membership_ordinals_and_content_are_validated() {
    let source = include_str!("../../../fixtures/indexing/page-ok.json");
    let analysis = fixture(source);
    let pages = validate_batch(&analysis, source.len(), &request(7)).unwrap();
    let page = pages[0].result.as_ref().unwrap();
    assert_eq!(page.page_number, 7);
    assert_eq!(page.blocks.len(), 5);
    assert_eq!(page.review_reason, None);
    assert_eq!(page.blocks[2].latex.as_deref(), Some("E = K + U"));
    assert_eq!(page.blocks[3].table_cells.as_ref().unwrap().len(), 4);
    assert_eq!(
        page.blocks[4].source,
        crate::domain::ContentSource::AiDescription
    );

    let mut wrong_schema = analysis.clone();
    wrong_schema.schema_version = "textbooklens.page-analysis.v2".to_owned();
    assert_eq!(
        validate_batch(&wrong_schema, source.len(), &request(7)),
        Err(BatchValidationFailure::InvalidSchemaVersion)
    );
    assert_eq!(
        validate_batch(&analysis, source.len(), &request(99)),
        Err(BatchValidationFailure::MissingOrUnexpectedPage)
    );
}

#[test]
fn overlap_is_retained_for_review_but_duplicate_ordinals_are_rejected() {
    let review_source = include_str!("../../../fixtures/indexing/page-needs-review.json");
    let review = fixture(review_source);
    let pages = validate_batch(&review, review_source.len(), &request(8)).unwrap();
    assert_eq!(
        pages[0].result.as_ref().unwrap().review_reason,
        Some(IndexReviewReason::SevereOverlap)
    );

    let malformed_source = include_str!("../../../fixtures/indexing/page-malformed.json");
    let malformed = fixture(malformed_source);
    let pages = validate_batch(&malformed, malformed_source.len(), &request(9)).unwrap();
    assert_eq!(pages[0].result, Err(PageValidationFailure::InvalidOrdinal));
}

#[test]
fn response_byte_ceiling_is_checked_before_page_content() {
    let source = include_str!("../../../fixtures/indexing/page-oversize.json");
    let analysis = fixture(source);
    assert_eq!(
        validate_batch(&analysis, MAX_VALIDATION_RESPONSE_BYTES + 1, &request(10)),
        Err(BatchValidationFailure::InvalidResponseSize)
    );
}

#[test]
fn duplicate_missing_and_unrequested_pages_fail_the_whole_batch() {
    let source = include_str!("../../../fixtures/indexing/page-ok.json");
    let mut analysis = fixture(source);
    analysis.pages.push(analysis.pages[0].clone());
    assert_eq!(
        validate_batch(&analysis, source.len(), &request(7)),
        Err(BatchValidationFailure::MissingOrUnexpectedPage)
    );

    let empty = ProviderPageAnalysis {
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        pages: Vec::new(),
    };
    assert_eq!(
        validate_batch(&empty, 2, &[]),
        Err(BatchValidationFailure::EmptyRequest)
    );
}

#[test]
fn every_coordinate_must_be_finite_positive_and_normalized() {
    let source = include_str!("../../../fixtures/indexing/page-ok.json");
    let mut analysis = fixture(source);
    analysis.pages[0].blocks[0].bounds = Some(UntrustedNormalizedRect {
        x: f64::NAN,
        y: 0.0,
        width: 0.5,
        height: 0.5,
    });
    let pages = validate_batch(&analysis, source.len(), &request(7)).unwrap();
    assert_eq!(pages[0].result, Err(PageValidationFailure::InvalidBounds));

    analysis.pages[0].blocks[0].bounds = Some(UntrustedNormalizedRect {
        x: 0.8,
        y: 0.0,
        width: 0.3,
        height: 0.5,
    });
    let pages = validate_batch(&analysis, source.len(), &request(7)).unwrap();
    assert_eq!(pages[0].result, Err(PageValidationFailure::InvalidBounds));
}

#[test]
fn repetition_contradiction_latex_and_table_findings_are_deterministic() {
    let source = include_str!("../../../fixtures/indexing/page-ok.json");
    let mut repeated = fixture(source);
    let repeated_block = repeated.pages[0].blocks[1].clone();
    repeated.pages[0].blocks = vec![
        repeated_block.clone(),
        repeated_block.clone(),
        repeated_block,
    ];
    for (ordinal, block) in repeated.pages[0].blocks.iter_mut().enumerate() {
        block.ordinal = ordinal as u32;
        block.bounds = None;
    }
    let pages = validate_batch(&repeated, source.len(), &request(7)).unwrap();
    assert_eq!(
        pages[0].result.as_ref().unwrap().review_reason,
        Some(IndexReviewReason::IncompleteContent)
    );

    let mut contradiction = fixture(source);
    contradiction.pages[0].blocks.truncate(2);
    let pages = validate_batch(
        &contradiction,
        source.len(),
        &[RequestedPageValidation {
            page_number: 7,
            local_text: Some(
                "甲乙丙丁戊己庚辛壬癸天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏".to_owned(),
            ),
        }],
    )
    .unwrap();
    assert_eq!(
        pages[0].result.as_ref().unwrap().review_reason,
        Some(IndexReviewReason::TextContradiction)
    );

    let mut malformed_latex = fixture(source);
    malformed_latex.pages[0].blocks[2].latex = Some("E = {K + U".to_owned());
    let pages = validate_batch(&malformed_latex, source.len(), &request(7)).unwrap();
    assert_eq!(
        pages[0].result.as_ref().unwrap().review_reason,
        Some(IndexReviewReason::MalformedLatex)
    );

    let mut malformed_table = fixture(source);
    malformed_table.pages[0].blocks[3]
        .table_cells
        .as_mut()
        .unwrap()[1]
        .column = 0;
    let pages = validate_batch(&malformed_table, source.len(), &request(7)).unwrap();
    assert_eq!(
        pages[0].result.as_ref().unwrap().review_reason,
        Some(IndexReviewReason::MalformedTable)
    );
}

#[test]
fn decoder_schema_rejects_model_supplied_ids_and_unknown_fields() {
    let source = include_str!("../../../fixtures/indexing/page-ok.json");
    let mut value = serde_json::from_str::<serde_json::Value>(source).unwrap();
    value["pages"][0]["blocks"][0]["id"] = serde_json::json!("model-owned-id");
    assert!(serde_json::from_value::<ProviderPageAnalysis>(value).is_err());
}

#[test]
fn page_number_zero_and_empty_visible_content_are_rejected() {
    let analysis = ProviderPageAnalysis {
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        pages: vec![UntrustedPageAnalysis {
            page_number: 0,
            blocks: Vec::new(),
        }],
    };
    assert_eq!(
        validate_batch(
            &analysis,
            10,
            &[RequestedPageValidation {
                page_number: 0,
                local_text: None,
            }]
        ),
        Err(BatchValidationFailure::DuplicateRequestedPage)
    );
}

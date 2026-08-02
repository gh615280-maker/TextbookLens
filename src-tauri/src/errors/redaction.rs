use uuid::Uuid;

use super::AppErrorCode;

const REDACTED: &str = "[REDACTED]";
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeDiagnostic {
    pub id: Uuid,
    pub code: AppErrorCode,
    pub detail: String,
}

impl SafeDiagnostic {
    pub fn new(code: AppErrorCode, detail: impl AsRef<str>) -> Self {
        Self {
            id: Uuid::new_v4(),
            code,
            detail: truncate_utf8(&redact(detail.as_ref()), MAX_DIAGNOSTIC_BYTES),
        }
    }
}

pub fn redact(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut ranges = Vec::new();

    mark_authorization_values(input, &lower, &mut ranges);
    for marker in [
        "x-goog-api-key:",
        "x-api-key:",
        "api-key:",
        "api_key=",
        "apikey=",
        "openai_api_key=",
        "anthropic_api_key=",
        "gemini_api_key=",
        "deepseek_api_key=",
        "kimi_api_key=",
    ] {
        mark_value_after(input, &lower, marker, &mut ranges);
    }
    mark_secret_tokens(input, &mut ranges);

    merge_ranges(&mut ranges);
    replace_ranges(input, &ranges)
}

fn mark_authorization_values(input: &str, lower: &str, ranges: &mut Vec<(usize, usize)>) {
    let marker = "authorization:";
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find(marker) {
        let marker_end = offset + relative + marker.len();
        let value_start = skip_ascii_whitespace(input, marker_end);
        let scheme_end = token_end(input, value_start);
        let scheme = lower.get(value_start..scheme_end).unwrap_or_default();
        let value_end = if matches!(scheme, "bearer" | "basic") {
            token_end(input, skip_ascii_whitespace(input, scheme_end))
        } else {
            scheme_end
        };
        if value_start < value_end {
            ranges.push((value_start, value_end));
        }
        offset = marker_end;
    }
}

fn mark_value_after(input: &str, lower: &str, marker: &str, ranges: &mut Vec<(usize, usize)>) {
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find(marker) {
        let marker_end = offset + relative + marker.len();
        let value_start = skip_ascii_whitespace(input, marker_end);
        let value_end = token_end(input, value_start);
        if value_start < value_end {
            ranges.push((value_start, value_end));
        }
        offset = marker_end;
    }
}

fn mark_secret_tokens(input: &str, ranges: &mut Vec<(usize, usize)>) {
    let bytes = input.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        let end = token_end(input, start);
        if start == end {
            start += 1;
            continue;
        }
        let token = &input[start..end];
        let trimmed_start = token
            .find(|character: char| character.is_ascii_alphanumeric())
            .unwrap_or(token.len());
        let candidate = &token[trimmed_start..];
        if candidate.starts_with("sk-") || (candidate.starts_with("AIza") && candidate.len() > 8) {
            ranges.push((start + trimmed_start, end));
        }
        start = end;
    }
}

fn skip_ascii_whitespace(input: &str, mut index: usize) -> usize {
    let bytes = input.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn token_end(input: &str, mut index: usize) -> usize {
    let bytes = input.as_bytes();
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn merge_ranges(ranges: &mut Vec<(usize, usize)>) {
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for &(start, end) in ranges.iter() {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    *ranges = merged;
}

fn replace_ranges(input: &str, ranges: &[(usize, usize)]) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    for &(start, end) in ranges {
        output.push_str(&input[cursor..start]);
        output.push_str(REDACTED);
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_is_capped_on_a_character_boundary() {
        let diagnostic = SafeDiagnostic::new(AppErrorCode::DatabaseError, "文".repeat(10_000));
        assert!(diagnostic.detail.len() <= MAX_DIAGNOSTIC_BYTES);
        assert!(diagnostic.detail.is_char_boundary(diagnostic.detail.len()));
    }
}

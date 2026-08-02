use std::collections::HashMap;

use uuid::Uuid;

pub const DEFAULT_TARGET_CODE_POINTS: usize = 1_000;
pub const DEFAULT_MIN_CODE_POINTS: usize = 800;
pub const DEFAULT_MAX_CODE_POINTS: usize = 1_200;
pub const DEFAULT_OVERLAP: f64 = 0.10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkBlock {
    pub section_id: Uuid,
    pub ordinal: u32,
    pub text: String,
    /// Locator of the first source block intersecting this chunk; search uses it as a
    /// navigation start, not as a claim that the locator spans the entire chunk.
    pub locator_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChunk {
    pub section_id: Uuid,
    pub ordinal: u32,
    pub text: String,
    pub locator_json: String,
    pub code_point_count: usize,
    pub start_code_point: usize,
    pub end_code_point: usize,
    pub source_block_ordinals: Vec<u32>,
}

pub fn chunk_blocks(
    blocks: &[ChunkBlock],
    target: usize,
    min: usize,
    max: usize,
    overlap: f64,
) -> Vec<TextChunk> {
    if blocks.is_empty() || max == 0 {
        return Vec::new();
    }

    let min = min.min(max);
    let target = target.clamp(min.max(1), max);
    let overlap = if overlap.is_finite() {
        overlap.clamp(0.0, 0.90)
    } else {
        0.0
    };
    let mut sections = Vec::<SectionText>::new();
    let mut section_indexes = HashMap::<Uuid, usize>::new();
    for block in blocks {
        if block.text.is_empty() {
            continue;
        }
        let index = match section_indexes.get(&block.section_id) {
            Some(index) => *index,
            None => {
                let index = sections.len();
                sections.push(SectionText::new(block.section_id));
                section_indexes.insert(block.section_id, index);
                index
            }
        };
        sections[index].push(block);
    }

    sections
        .into_iter()
        .flat_map(|section| section.into_chunks(target, min, max, overlap))
        .collect()
}

pub fn chunk_blocks_default(blocks: &[ChunkBlock]) -> Vec<TextChunk> {
    chunk_blocks(
        blocks,
        DEFAULT_TARGET_CODE_POINTS,
        DEFAULT_MIN_CODE_POINTS,
        DEFAULT_MAX_CODE_POINTS,
        DEFAULT_OVERLAP,
    )
}

#[derive(Clone, Debug)]
struct BlockRange {
    start: usize,
    end: usize,
    ordinal: u32,
    locator_json: String,
}

#[derive(Clone, Debug)]
struct SectionText {
    section_id: Uuid,
    code_points: Vec<char>,
    safe_boundaries: Vec<usize>,
    blocks: Vec<BlockRange>,
}

impl SectionText {
    fn new(section_id: Uuid) -> Self {
        Self {
            section_id,
            code_points: Vec::new(),
            safe_boundaries: vec![0],
            blocks: Vec::new(),
        }
    }

    fn push(&mut self, block: &ChunkBlock) {
        if !self.code_points.is_empty() {
            self.code_points.push('\n');
        }
        let start = self.code_points.len();
        for code_point in block.text.chars() {
            self.code_points.push(code_point);
            if is_sentence_terminator(code_point) {
                self.safe_boundaries.push(self.code_points.len());
            }
        }
        let end = self.code_points.len();
        self.safe_boundaries.push(end);
        self.blocks.push(BlockRange {
            start,
            end,
            ordinal: block.ordinal,
            locator_json: block.locator_json.clone(),
        });
    }

    fn into_chunks(
        mut self,
        target: usize,
        min: usize,
        max: usize,
        overlap: f64,
    ) -> Vec<TextChunk> {
        self.safe_boundaries.sort_unstable();
        self.safe_boundaries.dedup();
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < self.code_points.len() {
            let remaining = self.code_points.len() - start;
            let end = if remaining <= max {
                self.code_points.len()
            } else {
                choose_end(&self.safe_boundaries, start, target, min, max)
            };
            if end <= start {
                break;
            }

            let intersecting = self
                .blocks
                .iter()
                .filter(|block| block.start < end && block.end > start)
                .collect::<Vec<_>>();
            let locator_json = intersecting
                .first()
                .map(|block| block.locator_json.clone())
                .unwrap_or_else(|| "null".to_owned());
            let source_block_ordinals = intersecting
                .iter()
                .map(|block| block.ordinal)
                .collect::<Vec<_>>();
            chunks.push(TextChunk {
                section_id: self.section_id,
                ordinal: chunks.len() as u32,
                text: self.code_points[start..end].iter().collect(),
                locator_json,
                code_point_count: end - start,
                start_code_point: start,
                end_code_point: end,
                source_block_ordinals,
            });

            if end == self.code_points.len() {
                break;
            }
            let desired_overlap = ((end - start) as f64 * overlap).round() as usize;
            let desired_start = end.saturating_sub(desired_overlap);
            let minimum_overlap = ((end - start) as f64 * 0.05).floor() as usize;
            let maximum_overlap = ((end - start) as f64 * 0.15).ceil() as usize;
            let next_start = closest_boundary(
                &self.safe_boundaries,
                desired_start,
                end.saturating_sub(maximum_overlap),
                end.saturating_sub(minimum_overlap.max(1)),
            )
            .unwrap_or(end);
            start = next_start;
        }
        chunks
    }
}

fn choose_end(boundaries: &[usize], start: usize, target: usize, min: usize, max: usize) -> usize {
    let lower = start.saturating_add(min);
    let upper = start.saturating_add(max);
    closest_boundary(boundaries, start.saturating_add(target), lower, upper).unwrap_or(upper)
}

fn closest_boundary(
    boundaries: &[usize],
    desired: usize,
    lower: usize,
    upper: usize,
) -> Option<usize> {
    boundaries
        .iter()
        .copied()
        .filter(|boundary| (lower..=upper).contains(boundary))
        .min_by_key(|boundary| (boundary.abs_diff(desired), usize::MAX - boundary))
}

fn is_sentence_terminator(code_point: char) -> bool {
    matches!(
        code_point,
        '.' | '?' | '!' | ';' | '。' | '？' | '！' | '；'
    )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn block(section_id: Uuid, ordinal: u32, text: String) -> ChunkBlock {
        ChunkBlock {
            section_id,
            ordinal,
            text,
            locator_json: format!(r#"{{"block":{ordinal}}}"#),
        }
    }

    fn section_text(blocks: &[ChunkBlock], section_id: Uuid) -> String {
        blocks
            .iter()
            .filter(|block| block.section_id == section_id)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn code_point_slice(text: &str, start: usize, end: usize) -> String {
        text.chars().skip(start).take(end - start).collect()
    }

    #[test]
    fn chunks_on_sentence_boundaries_with_ten_percent_overlap() {
        let section_id = Uuid::new_v4();
        let blocks = (0..30)
            .map(|ordinal| {
                let body = if ordinal % 2 == 0 {
                    "线性代数与能量守恒".repeat(9)
                } else {
                    "matrix energy theorem ".repeat(4)
                };
                block(section_id, ordinal, format!("{body}{ordinal:02}。"))
            })
            .collect::<Vec<_>>();

        let chunks = chunk_blocks_default(&blocks);

        assert!(chunks.len() >= 3);
        let source = section_text(&blocks, section_id);
        for chunk in &chunks {
            assert_eq!(chunk.section_id, section_id);
            assert!((800..=1_200).contains(&chunk.code_point_count));
            assert!(matches!(
                chunk.text.chars().last(),
                Some('.' | '?' | '!' | '。' | '？' | '！' | '；')
            ));
            assert_eq!(
                chunk.text,
                code_point_slice(&source, chunk.start_code_point, chunk.end_code_point)
            );
            assert!(
                chunk
                    .source_block_ordinals
                    .windows(2)
                    .all(|pair| pair[0] <= pair[1])
            );
        }
        for pair in chunks.windows(2) {
            let overlap = pair[0].end_code_point - pair[1].start_code_point;
            let ratio = overlap as f64 / pair[0].code_point_count as f64;
            assert!((0.05..=0.15).contains(&ratio), "overlap ratio was {ratio}");
        }
    }

    #[test]
    fn only_an_oversized_sentence_is_split_at_the_hard_maximum() {
        let section_id = Uuid::new_v4();
        let text = format!("{}。短句结束。", "长".repeat(1_450));

        let chunks = chunk_blocks_default(&[block(section_id, 0, text)]);

        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].code_point_count, DEFAULT_MAX_CODE_POINTS);
        assert!(!chunks[0].text.ends_with('。'));
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.code_point_count <= DEFAULT_MAX_CODE_POINTS)
        );
    }

    #[test]
    fn a_distant_old_sentence_boundary_never_causes_duplicate_chunks() {
        let section_id = Uuid::new_v4();
        let text = format!(
            "{}。{}。{}。",
            "A".repeat(100),
            "B".repeat(999),
            "C".repeat(1_000)
        );

        let chunks = chunk_blocks_default(&[block(section_id, 0, text)]);

        assert!(chunks.len() >= 2);
        for pair in chunks.windows(2) {
            assert!(pair[1].end_code_point > pair[0].end_code_point);
            let overlap = pair[0]
                .end_code_point
                .saturating_sub(pair[1].start_code_point);
            if overlap == 0 {
                assert_eq!(pair[1].start_code_point, pair[0].end_code_point);
            } else {
                let ratio = overlap as f64 / pair[0].code_point_count as f64;
                assert!((0.05..=0.15).contains(&ratio));
            }
        }
    }

    #[test]
    fn chunk_sizes_count_unicode_scalars_not_utf8_bytes() {
        let section_id = Uuid::new_v4();
        let mixed = format!("{}。", "😀𠀀e\u{301}中".repeat(240));

        let chunks = chunk_blocks_default(&[block(section_id, 0, mixed.clone())]);

        assert_eq!(mixed.chars().count(), 1_201);
        assert_eq!(chunks[0].code_point_count, DEFAULT_MAX_CODE_POINTS);
        assert_eq!(chunks[1].end_code_point, mixed.chars().count());
        assert!(
            chunks
                .windows(2)
                .all(|pair| pair[1].end_code_point > pair[0].end_code_point)
        );
    }

    proptest! {
        #[test]
        fn mixed_language_chunks_preserve_section_and_source_order(
            sentence_count in 18usize..42,
            chinese_width in 30usize..70,
            english_width in 30usize..70,
        ) {
            let first_section = Uuid::new_v4();
            let second_section = Uuid::new_v4();
            let mut blocks = Vec::new();
            for section_id in [first_section, second_section] {
                for ordinal in 0..sentence_count as u32 {
                    let text = if ordinal % 2 == 0 {
                        format!("{}。", "教材检索".repeat(chinese_width / 4 + 1))
                    } else {
                        format!("{}!", "ordered textbook search ".repeat(english_width / 24 + 1))
                    };
                    blocks.push(block(section_id, ordinal, text));
                }
            }

            let chunks = chunk_blocks_default(&blocks);
            for section_id in [first_section, second_section] {
                let source = section_text(&blocks, section_id);
                let section_chunks = chunks
                    .iter()
                    .filter(|chunk| chunk.section_id == section_id)
                    .collect::<Vec<_>>();
                prop_assert!(!section_chunks.is_empty());
                for (expected_ordinal, chunk) in section_chunks.iter().enumerate() {
                    prop_assert_eq!(chunk.ordinal, expected_ordinal as u32);
                    prop_assert_eq!(
                        chunk.text.as_str(),
                        code_point_slice(&source, chunk.start_code_point, chunk.end_code_point)
                    );
                    prop_assert!(chunk.code_point_count <= DEFAULT_MAX_CODE_POINTS);
                    prop_assert!(chunk.source_block_ordinals.windows(2).all(|pair| pair[0] <= pair[1]));
                }
                prop_assert_eq!(section_chunks.last().unwrap().end_code_point, source.chars().count());
            }
        }
    }
}

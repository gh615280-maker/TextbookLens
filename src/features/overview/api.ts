import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type {
  LearningOverview,
  LearningOverviewErrorCode,
} from '../../lib/generated/overview';

const MAX_OVERVIEW_SECTIONS = 100_000;
const MAX_SECTION_TITLE_CODE_POINTS = 4_096;
const UINT32_MAX = 0xffff_ffff;

const SOURCE_ORDER = Object.freeze([
  'local_text',
  'ai_transcribed',
  'ai_description',
  'user_corrected',
  'user_note',
  'history_summary',
] as const);

export const LEARNING_OVERVIEW_ERROR_CODES = Object.freeze([
  'LEARNING_OVERVIEW_INVALID_INPUT',
  'LEARNING_OVERVIEW_NOT_FOUND',
  'LEARNING_OVERVIEW_BOOK_NOT_READY',
  'LEARNING_OVERVIEW_BUSY',
  'LEARNING_OVERVIEW_DATA_INVALID',
] as const satisfies readonly LearningOverviewErrorCode[]);

export interface LearningOverviewError {
  readonly code: LearningOverviewErrorCode;
}

const uuidSchema = z.uuid();
const uint32Schema = z.number().int().min(0).max(UINT32_MAX);
const titleSchema = z
  .string()
  .min(1)
  .refine((value) => value.trim() === value)
  .refine((value) => [...value].length <= MAX_SECTION_TITLE_CODE_POINTS)
  .refine(
    (value) =>
      ![...value].some(
        (character) => isControl(character) && character !== '\t',
      ),
  );
const sectionSchema = z
  .object({
    id: uuidSchema,
    parentId: uuidSchema.nullable(),
    ordinal: uint32Schema,
    title: titleSchema,
    localTextItemCount: uint32Schema,
    userNoteCount: uint32Schema,
    completedConversationCount: uint32Schema,
    completedExchangeCount: uint32Schema,
  })
  .strict();
const sourceSchema = z
  .object({
    source: z.enum(SOURCE_ORDER),
    itemCount: uint32Schema,
    coveredSectionCount: uint32Schema,
    coveredPageCount: uint32Schema,
    quoteableAsTextbook: z.boolean(),
  })
  .strict();
const activitySchema = z
  .object({
    userNoteCount: uint32Schema,
    completedConversationCount: uint32Schema,
    completedExchangeCount: uint32Schema,
    citationCount: uint32Schema,
  })
  .strict();
const overviewSchema = z
  .object({
    bookId: uuidSchema,
    format: z.enum(['pdf', 'epub', 'docx']),
    teachingInstructionConfigured: z.boolean(),
    sectionCount: uint32Schema,
    sections: z.array(sectionSchema).max(MAX_OVERVIEW_SECTIONS),
    sources: z.array(sourceSchema).length(SOURCE_ORDER.length),
    activity: activitySchema,
  })
  .strict()
  .superRefine((overview, context) => {
    if (overview.sectionCount !== overview.sections.length) {
      issue(context, 'section count mismatch');
      return;
    }
    const sectionIds = new Set(overview.sections.map((section) => section.id));
    if (sectionIds.size !== overview.sections.length) {
      issue(context, 'duplicate section');
      return;
    }
    for (const [index, section] of overview.sections.entries()) {
      const previous = overview.sections[index - 1];
      if (
        (previous !== undefined && previous.ordinal >= section.ordinal) ||
        section.parentId === section.id ||
        (section.parentId !== null && !sectionIds.has(section.parentId))
      ) {
        issue(context, 'invalid section ordering or ownership');
        return;
      }
    }
    for (const [index, source] of overview.sources.entries()) {
      const expectedSource = SOURCE_ORDER[index];
      const expectedQuoteable =
        expectedSource === 'local_text' ||
        expectedSource === 'ai_transcribed' ||
        expectedSource === 'user_corrected';
      const sectionBased =
        expectedSource === 'local_text' ||
        expectedSource === 'user_note' ||
        expectedSource === 'history_summary';
      if (
        source.source !== expectedSource ||
        source.quoteableAsTextbook !== expectedQuoteable ||
        (sectionBased && source.coveredPageCount !== 0) ||
        (!sectionBased && source.coveredSectionCount !== 0) ||
        source.coveredSectionCount > overview.sectionCount ||
        source.coveredPageCount > source.itemCount
      ) {
        issue(context, 'invalid source contract');
        return;
      }
    }
    const bySource = Object.fromEntries(
      overview.sources.map((source) => [source.source, source]),
    ) as Record<
      (typeof SOURCE_ORDER)[number],
      (typeof overview.sources)[number]
    >;
    const localItems = sum(
      overview.sections.map((section) => section.localTextItemCount),
    );
    const noteItems = sum(
      overview.sections.map((section) => section.userNoteCount),
    );
    const sectionConversations = sum(
      overview.sections.map((section) => section.completedConversationCount),
    );
    const sectionExchanges = sum(
      overview.sections.map((section) => section.completedExchangeCount),
    );
    if (
      localItems !== bySource.local_text.itemCount ||
      countPositive(overview.sections, 'localTextItemCount') !==
        bySource.local_text.coveredSectionCount ||
      noteItems !== bySource.user_note.itemCount ||
      countPositive(overview.sections, 'userNoteCount') !==
        bySource.user_note.coveredSectionCount ||
      overview.activity.userNoteCount !== noteItems ||
      overview.activity.completedExchangeCount !==
        bySource.history_summary.itemCount ||
      countPositive(overview.sections, 'completedExchangeCount') !==
        bySource.history_summary.coveredSectionCount ||
      sectionConversations > overview.activity.completedConversationCount ||
      sectionExchanges > overview.activity.completedExchangeCount
    ) {
      issue(context, 'inconsistent deterministic totals');
    }
  });

export interface LearningOverviewApi {
  get(bookId: string): Promise<Readonly<LearningOverview>>;
}

export class TauriLearningOverviewApi implements LearningOverviewApi {
  async get(bookId: string): Promise<Readonly<LearningOverview>> {
    const parsedBookId = uuidSchema.safeParse(bookId);
    if (!parsedBookId.success) {
      throw overviewError('LEARNING_OVERVIEW_INVALID_INPUT');
    }
    let response: unknown;
    try {
      response = await invoke<unknown>('get_learning_overview', {
        bookId: parsedBookId.data,
      });
    } catch (error) {
      throw stableInvocationError(error);
    }
    const parsed = overviewSchema.safeParse(response);
    if (!parsed.success || parsed.data.bookId !== parsedBookId.data) {
      throw overviewError('LEARNING_OVERVIEW_DATA_INVALID');
    }
    return deepFreeze(parsed.data as LearningOverview);
  }
}

export function isLearningOverviewError(
  value: unknown,
): value is LearningOverviewError {
  return (
    typeof value === 'object' &&
    value !== null &&
    Object.keys(value).length === 1 &&
    z
      .enum(LEARNING_OVERVIEW_ERROR_CODES)
      .safeParse((value as { code?: unknown }).code).success
  );
}

function stableInvocationError(
  error: unknown,
): Readonly<LearningOverviewError> {
  const parsed = z
    .object({ code: z.enum(LEARNING_OVERVIEW_ERROR_CODES) })
    .passthrough()
    .safeParse(error);
  return overviewError(
    parsed.success ? parsed.data.code : 'LEARNING_OVERVIEW_DATA_INVALID',
  );
}

function overviewError(
  code: LearningOverviewErrorCode,
): Readonly<LearningOverviewError> {
  return Object.freeze({ code });
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}

function sum(values: readonly number[]): number {
  const total = values.reduce((current, value) => current + value, 0);
  return Number.isSafeInteger(total) ? total : Number.NaN;
}

function countPositive(
  sections: readonly z.infer<typeof sectionSchema>[],
  field: 'localTextItemCount' | 'userNoteCount' | 'completedExchangeCount',
): number {
  return sections.filter((section) => section[field] > 0).length;
}

function issue(context: z.RefinementCtx, message: string): void {
  context.addIssue({ code: 'custom', message });
}

function isControl(character: string): boolean {
  const codePoint = character.codePointAt(0);
  return (
    codePoint !== undefined &&
    ((codePoint >= 0 && codePoint <= 0x1f) ||
      (codePoint >= 0x7f && codePoint <= 0x9f))
  );
}

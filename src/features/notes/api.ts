import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type { ContentAnchor } from '../../lib/generated/document';

export const MAX_NOTE_CODE_POINTS = 16_384;
const MAX_SELECTED_TEXT_CODE_POINTS = 1_048_576;
const MAX_CFI_CODE_POINTS = 4_096;
const SHA256 = /^[0-9a-f]{64}$/u;
const UTC_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/u;

const uuidSchema = z.uuid();
const uint32Schema = z.number().int().min(0).max(0xffff_ffff);
const positiveUint32Schema = uint32Schema.min(1);
const codePointBounded = (maximum: number) =>
  z.string().refine((value) => [...value].length <= maximum);
const rectSchema = z
  .object({
    x: z.number().finite().min(0).max(1),
    y: z.number().finite().min(0).max(1),
    width: z.number().finite().positive().max(1),
    height: z.number().finite().positive().max(1),
  })
  .strict()
  .refine((rect) => rect.x + rect.width <= 1 && rect.y + rect.height <= 1);
const quoteSchema = z
  .object({
    exact: codePointBounded(MAX_SELECTED_TEXT_CODE_POINTS).refine(
      (value) => value.length > 0,
    ),
    prefix: codePointBounded(64),
    suffix: codePointBounded(64),
  })
  .strict();
const documentLocatorSchema = z.discriminatedUnion('format', [
  z
    .object({
      format: z.literal('pdf'),
      startPage: positiveUint32Schema,
      endPage: positiveUint32Schema,
      rectsByPage: z
        .record(z.string().regex(/^\d+$/u), z.array(rectSchema).max(128))
        .nullable(),
    })
    .strict()
    .refine((value) => value.startPage <= value.endPage),
  z
    .object({
      format: z.literal('epub'),
      cfi: codePointBounded(MAX_CFI_CODE_POINTS).refine(
        (value) => value.trim().length > 0,
      ),
      sectionId: uuidSchema,
    })
    .strict(),
  z
    .object({
      format: z.literal('docx'),
      startBlockId: uuidSchema,
      startOffset: uint32Schema,
      endBlockId: uuidSchema,
      endOffset: uint32Schema,
    })
    .strict(),
]);
const textAnchorSchema = z
  .object({
    kind: z.literal('text'),
    selection: z
      .object({
        locator: documentLocatorSchema,
        quote: quoteSchema,
        sectionId: uuidSchema.nullable(),
      })
      .strict(),
  })
  .strict();
const regionLocatorSchema = z.discriminatedUnion('format', [
  z.object({ format: z.literal('pdf'), page: positiveUint32Schema }).strict(),
  z
    .object({
      format: z.literal('epub'),
      sectionId: uuidSchema,
      cfi: codePointBounded(MAX_CFI_CODE_POINTS).refine(
        (value) => value.trim().length > 0,
      ),
    })
    .strict(),
  z.object({ format: z.literal('docx'), blockId: uuidSchema }).strict(),
]);
const regionAnchorSchema = z
  .object({
    kind: z.literal('region'),
    region: z
      .object({
        locator: regionLocatorSchema,
        rect: rectSchema,
        contentSha256: z.string().regex(SHA256),
        textFallback: quoteSchema.nullable(),
      })
      .strict(),
  })
  .strict();
const contentAnchorSchema = z.discriminatedUnion('kind', [
  textAnchorSchema,
  regionAnchorSchema,
]);
const noteTextSchema = codePointBounded(MAX_NOTE_CODE_POINTS).refine(
  (value) => value.trim().length > 0,
);
const noteSchema = z
  .object({
    id: uuidSchema,
    bookId: uuidSchema,
    sectionId: uuidSchema,
    anchor: contentAnchorSchema,
    selectedText: codePointBounded(MAX_SELECTED_TEXT_CODE_POINTS).nullable(),
    noteText: noteTextSchema,
    revision: z.number().int().min(1).max(1_000_000),
    createdAt: z.string().regex(UTC_TIMESTAMP),
    updatedAt: z.string().regex(UTC_TIMESTAMP),
  })
  .strict();

export const NOTE_ERROR_CODES = Object.freeze([
  'INVALID_INPUT',
  'NOT_FOUND',
  'BOOK_NOT_READY',
  'ANCHOR_NOT_FOUND',
  'REQUEST_CONFLICT',
  'DATABASE_ERROR',
] as const);
export type NoteErrorCode = (typeof NOTE_ERROR_CODES)[number];
export interface NoteError {
  readonly code: NoteErrorCode;
}

export interface Note {
  readonly id: string;
  readonly bookId: string;
  readonly sectionId: string;
  readonly anchor: ContentAnchor;
  readonly selectedText: string | null;
  readonly noteText: string;
  readonly revision: number;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface CreateNoteInput {
  readonly bookId: string;
  readonly sectionId: string;
  readonly anchor: ContentAnchor;
  readonly selectedText: string | null;
  readonly noteText: string;
}

export interface UpdateNoteInput {
  readonly bookId: string;
  readonly noteId: string;
  readonly expectedRevision: number;
  readonly noteText: string;
}

export interface DeleteNoteInput {
  readonly bookId: string;
  readonly noteId: string;
  readonly expectedRevision: number;
}

export interface NotesApi {
  create(input: CreateNoteInput): Promise<Readonly<Note>>;
  update(input: UpdateNoteInput): Promise<Readonly<Note>>;
  delete(input: DeleteNoteInput): Promise<void>;
  get(bookId: string, noteId: string): Promise<Readonly<Note>>;
  list(bookId: string): Promise<readonly Readonly<Note>[]>;
}

export class TauriNotesApi implements NotesApi {
  async create(input: CreateNoteInput): Promise<Readonly<Note>> {
    const normalized = normalizeInput(input);
    const parsed = z
      .object({
        bookId: uuidSchema,
        sectionId: uuidSchema,
        anchor: contentAnchorSchema,
        selectedText: codePointBounded(
          MAX_SELECTED_TEXT_CODE_POINTS,
        ).nullable(),
        noteText: noteTextSchema,
      })
      .strict()
      .superRefine((value, context) => {
        const anchorSection =
          value.anchor.kind === 'text'
            ? value.anchor.selection.sectionId
            : value.anchor.region.locator.format === 'epub'
              ? value.anchor.region.locator.sectionId
              : value.sectionId;
        if (anchorSection !== value.sectionId) {
          context.addIssue({ code: 'custom', message: 'section mismatch' });
        }
        const exact =
          value.anchor.kind === 'text'
            ? value.anchor.selection.quote.exact
            : (value.anchor.region.textFallback?.exact ?? null);
        if (exact !== value.selectedText) {
          context.addIssue({ code: 'custom', message: 'selection mismatch' });
        }
      })
      .safeParse(normalized);
    if (!parsed.success) throw noteError('INVALID_INPUT');
    return this.invokeNote('create_note', parsed.data);
  }

  async update(input: UpdateNoteInput): Promise<Readonly<Note>> {
    const parsed = z
      .object({
        bookId: uuidSchema,
        noteId: uuidSchema,
        expectedRevision: z.number().int().min(1).max(1_000_000),
        noteText: noteTextSchema,
      })
      .strict()
      .safeParse({ ...input, noteText: normalizeLineEndings(input.noteText) });
    if (!parsed.success) throw noteError('INVALID_INPUT');
    return this.invokeNote('update_note', parsed.data);
  }

  async delete(input: DeleteNoteInput): Promise<void> {
    const parsed = z
      .object({
        bookId: uuidSchema,
        noteId: uuidSchema,
        expectedRevision: z.number().int().min(1).max(1_000_000),
      })
      .strict()
      .safeParse(input);
    if (!parsed.success) throw noteError('INVALID_INPUT');
    try {
      await invoke('delete_note', parsed.data);
    } catch (error) {
      throw safeInvocationError(error);
    }
  }

  async get(bookId: string, noteId: string): Promise<Readonly<Note>> {
    const parsed = z
      .object({ bookId: uuidSchema, noteId: uuidSchema })
      .strict()
      .safeParse({ bookId, noteId });
    if (!parsed.success) throw noteError('INVALID_INPUT');
    return this.invokeNote('get_note', parsed.data);
  }

  async list(bookId: string): Promise<readonly Readonly<Note>[]> {
    const parsed = uuidSchema.safeParse(bookId);
    if (!parsed.success) throw noteError('INVALID_INPUT');
    let response: unknown;
    try {
      response = await invoke('list_notes', { bookId: parsed.data });
    } catch (error) {
      throw safeInvocationError(error);
    }
    const notes = z.array(noteSchema).safeParse(response);
    if (!notes.success) throw noteError('DATABASE_ERROR');
    return Object.freeze(notes.data.map(freezeNote));
  }

  private async invokeNote(
    command: 'create_note' | 'update_note' | 'get_note',
    args: Record<string, unknown>,
  ): Promise<Readonly<Note>> {
    let response: unknown;
    try {
      response = await invoke(command, args);
    } catch (error) {
      throw safeInvocationError(error);
    }
    const note = noteSchema.safeParse(response);
    if (!note.success) throw noteError('DATABASE_ERROR');
    return freezeNote(note.data as Note);
  }
}

export function isNoteError(value: unknown): value is NoteError {
  return (
    typeof value === 'object' &&
    value !== null &&
    Object.keys(value).length === 1 &&
    z.enum(NOTE_ERROR_CODES).safeParse((value as { code?: unknown }).code)
      .success
  );
}

function normalizeInput(input: CreateNoteInput): CreateNoteInput {
  return { ...input, noteText: normalizeLineEndings(input.noteText) };
}

function normalizeLineEndings(value: string) {
  return value.replace(/\r\n?/gu, '\n');
}

function freezeNote(note: Note): Readonly<Note> {
  return deepFreeze(note);
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}

function safeInvocationError(error: unknown): Readonly<NoteError> {
  const parsed = z
    .object({ code: z.enum(NOTE_ERROR_CODES) })
    .passthrough()
    .safeParse(error);
  return noteError(parsed.success ? parsed.data.code : 'DATABASE_ERROR');
}

function noteError(code: NoteErrorCode): Readonly<NoteError> {
  return Object.freeze({ code });
}

import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type {
  Citation,
  LearningAction,
} from '../../lib/generated/conversation';
import type { ContentAnchor } from '../../lib/generated/document';

const uuid = z.string().uuid();
const timestamp = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/u);
const boundedText = (max: number) => z.string().min(1).max(max);
const finiteUnit = z.number().finite().min(0).max(1);
const rect = z
  .object({
    x: finiteUnit,
    y: finiteUnit,
    width: finiteUnit,
    height: finiteUnit,
  })
  .strict()
  .refine((value) => value.width > 0 && value.height > 0)
  .refine((value) => value.x + value.width <= 1 && value.y + value.height <= 1);
const locator = z.discriminatedUnion('format', [
  z
    .object({
      format: z.literal('pdf'),
      startPage: z.number().int().positive(),
      endPage: z.number().int().positive(),
      rectsByPage: z.record(z.string(), z.array(rect)).nullable(),
    })
    .strict()
    .refine((value) => value.startPage <= value.endPage),
  z
    .object({
      format: z.literal('epub'),
      cfi: boundedText(4096),
      sectionId: uuid,
    })
    .strict(),
  z
    .object({
      format: z.literal('docx'),
      startBlockId: uuid,
      startOffset: z.number().int().nonnegative(),
      endBlockId: uuid,
      endOffset: z.number().int().nonnegative(),
    })
    .strict(),
]);
const quote = z
  .object({
    exact: boundedText(1_048_576),
    prefix: z.string().max(64),
    suffix: z.string().max(64),
  })
  .strict();
const regionLocator = z.discriminatedUnion('format', [
  z
    .object({ format: z.literal('pdf'), page: z.number().int().positive() })
    .strict(),
  z
    .object({
      format: z.literal('epub'),
      sectionId: uuid,
      cfi: boundedText(4096),
    })
    .strict(),
  z.object({ format: z.literal('docx'), blockId: uuid }).strict(),
]);
const contentAnchor = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('text'),
      selection: z
        .object({ locator, quote, sectionId: uuid.nullable() })
        .strict(),
    })
    .strict(),
  z
    .object({
      kind: z.literal('region'),
      region: z
        .object({
          locator: regionLocator,
          rect,
          contentSha256: z.string().regex(/^[0-9a-f]{64}$/u),
          textFallback: quote.nullable(),
        })
        .strict(),
    })
    .strict(),
]);
const citation = z
  .object({
    id: z.string().regex(/^TL-C[1-9]\d*$/u),
    label: boundedText(256),
    bookId: uuid,
    sectionId: uuid.nullable(),
    locator,
    source: z.enum(['local_text', 'ai_transcribed', 'user_corrected']),
    reviewStatus: z.enum([
      'not_required',
      'indexed',
      'needs_review',
      'user_corrected',
    ]),
    quoteable: z.literal(true),
  })
  .strict();
const action = z.enum([
  'explain',
  'example',
  'derive',
  'translate',
  'ask',
  'continue',
  'overview',
]);
const message = z
  .object({
    id: uuid,
    ordinal: z.number().int().nonnegative(),
    role: z.enum(['user', 'assistant']),
    action,
    content: boundedText(4 * 1024 * 1024),
    providerId: uuid.nullable(),
    modelId: z.string().min(1).max(256).nullable(),
    citations: z.array(citation).max(256),
    createdAt: timestamp,
  })
  .strict();
const conversation = z
  .object({
    id: uuid,
    bookId: uuid,
    annotationId: uuid,
    sectionId: uuid,
    anchor: contentAnchor,
    selectedText: z
      .string()
      .min(1)
      .max(2 * 1024 * 1024)
      .nullable(),
    status: z.literal('completed'),
    messages: z.array(message).min(2).max(4096),
    createdAt: timestamp,
    updatedAt: timestamp,
  })
  .strict()
  .superRefine((value, context) => {
    if (value.messages.length % 2 !== 0 || value.updatedAt < value.createdAt) {
      context.addIssue({
        code: 'custom',
        message: 'invalid conversation shape',
      });
    }
    const selected =
      value.anchor.kind === 'text'
        ? value.anchor.selection.quote.exact
        : (value.anchor.region.textFallback?.exact ?? null);
    if (selected !== value.selectedText) {
      context.addIssue({ code: 'custom', message: 'selection mismatch' });
    }
    value.messages.forEach((item, index) => {
      const assistant = index % 2 === 1;
      if (
        item.ordinal !== index ||
        item.role !== (assistant ? 'assistant' : 'user') ||
        (assistant && (!item.providerId || !item.modelId)) ||
        (!assistant &&
          (item.providerId || item.modelId || item.citations.length > 0)) ||
        (assistant && item.action !== value.messages[index - 1]?.action) ||
        item.citations.some((entry) => entry.bookId !== value.bookId)
      ) {
        context.addIssue({
          code: 'custom',
          message: 'invalid message sequence',
        });
      }
    });
  });

const bookConversation = z
  .object({
    id: uuid,
    bookId: uuid,
    scope: z.literal('book'),
    status: z.literal('completed'),
    messages: z.array(message).min(2).max(4096),
    createdAt: timestamp,
    updatedAt: timestamp,
  })
  .strict()
  .superRefine((value, context) => {
    if (value.messages.length % 2 !== 0 || value.updatedAt < value.createdAt) {
      context.addIssue({
        code: 'custom',
        message: 'invalid book conversation shape',
      });
    }
    value.messages.forEach((item, index) => {
      const assistant = index % 2 === 1;
      const expectedAction = index < 2 ? 'ask' : 'continue';
      if (
        item.ordinal !== index ||
        item.role !== (assistant ? 'assistant' : 'user') ||
        item.action !== expectedAction ||
        (assistant && (!item.providerId || !item.modelId)) ||
        (!assistant &&
          (item.providerId || item.modelId || item.citations.length > 0)) ||
        item.citations.some((entry) => entry.bookId !== value.bookId) ||
        item.createdAt < value.createdAt ||
        item.createdAt > value.updatedAt ||
        (index > 0 && item.createdAt < value.messages[index - 1]!.createdAt)
      ) {
        context.addIssue({
          code: 'custom',
          message: 'invalid book message sequence',
        });
      }
    });
  });

export interface ConversationMessage {
  readonly id: string;
  readonly ordinal: number;
  readonly role: 'user' | 'assistant';
  readonly action: LearningAction;
  readonly content: string;
  readonly providerId: string | null;
  readonly modelId: string | null;
  readonly citations: readonly Citation[];
  readonly createdAt: string;
}

export interface ConversationHistory {
  readonly id: string;
  readonly bookId: string;
  readonly annotationId: string;
  readonly sectionId: string;
  readonly anchor: ContentAnchor;
  readonly selectedText: string | null;
  readonly status: 'completed';
  readonly messages: readonly ConversationMessage[];
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface BookConversationHistory {
  readonly id: string;
  readonly bookId: string;
  readonly scope: 'book';
  readonly status: 'completed';
  readonly messages: readonly ConversationMessage[];
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface BookConversationApi {
  getBook(
    bookId: string,
    conversationId: string,
  ): Promise<BookConversationHistory>;
  deleteBook(bookId: string, conversationId: string): Promise<void>;
}

export class TauriBookConversationApi implements BookConversationApi {
  async getBook(
    bookId: string,
    conversationId: string,
  ): Promise<BookConversationHistory> {
    return parseBookConversation(
      await invoke('get_book_learning_conversation', {
        bookId: uuid.parse(bookId),
        conversationId: uuid.parse(conversationId),
      }),
    );
  }

  async deleteBook(bookId: string, conversationId: string): Promise<void> {
    await invoke('delete_book_learning_conversation', {
      bookId: uuid.parse(bookId),
      conversationId: uuid.parse(conversationId),
    });
  }
}

export interface ConversationApi {
  get(bookId: string, conversationId: string): Promise<ConversationHistory>;
  delete(
    bookId: string,
    conversationId: string,
    annotationId: string,
  ): Promise<void>;
}

export class TauriConversationApi implements ConversationApi {
  async get(
    bookId: string,
    conversationId: string,
  ): Promise<ConversationHistory> {
    return parseConversation(
      await invoke('get_learning_conversation', {
        bookId: uuid.parse(bookId),
        conversationId: uuid.parse(conversationId),
      }),
    );
  }

  async delete(
    bookId: string,
    conversationId: string,
    annotationId: string,
  ): Promise<void> {
    await invoke('delete_learning_conversation', {
      bookId: uuid.parse(bookId),
      conversationId: uuid.parse(conversationId),
      annotationId: uuid.parse(annotationId),
    });
  }
}

function parseBookConversation(value: unknown): BookConversationHistory {
  return deepFreezeBookConversation(
    bookConversation.parse(value) as BookConversationHistory,
  );
}

function deepFreezeBookConversation(
  value: BookConversationHistory,
): BookConversationHistory {
  return deepFreeze(value);
}

export interface ConversationPanelSnapshot {
  readonly conversations: readonly ConversationHistory[];
  readonly pendingDeletes: ReadonlySet<string>;
  readonly revision: number;
}

export class ConversationPanelOwner {
  #active = true;
  constructor(readonly bookId: string) {}
  dispose(): void {
    this.#active = false;
  }
  owns(bookId: string): boolean {
    return this.#active && this.bookId === bookId;
  }
}

export class ConversationPanelStore {
  readonly #api: ConversationApi;
  readonly #conversations = new Map<string, ConversationHistory>();
  readonly #generations = new Map<string, number>();
  readonly #pendingDeletes = new Set<string>();
  readonly #deleted = new Set<string>();
  readonly #listeners = new Set<() => void>();
  readonly #deletedListeners = new Set<
    (bookId: string, annotationId: string) => void
  >();
  #revision = 0;
  #snapshot: ConversationPanelSnapshot = freezeSnapshot([], new Set(), 0);

  constructor(api: ConversationApi = new TauriConversationApi()) {
    this.#api = api;
  }

  snapshot(): ConversationPanelSnapshot {
    return this.#snapshot;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  onDeleted(
    listener: (bookId: string, annotationId: string) => void,
  ): () => void {
    this.#deletedListeners.add(listener);
    return () => this.#deletedListeners.delete(listener);
  }

  async open(
    bookId: string,
    conversationId: string,
    annotationId: string,
    owner: ConversationPanelOwner,
  ): Promise<boolean> {
    const key = conversationKey(bookId, conversationId);
    if (
      this.#deleted.has(key) ||
      this.#pendingDeletes.has(key) ||
      !owner.owns(bookId)
    ) {
      return false;
    }
    const generation = this.#nextGeneration(key);
    let loaded: ConversationHistory;
    try {
      loaded = freezeConversation(await this.#api.get(bookId, conversationId));
    } catch (error) {
      if (
        generation !== this.#generations.get(key) ||
        this.#deleted.has(key) ||
        this.#pendingDeletes.has(key) ||
        !owner.owns(bookId)
      ) {
        return false;
      }
      throw error;
    }
    if (
      generation !== this.#generations.get(key) ||
      this.#deleted.has(key) ||
      this.#pendingDeletes.has(key) ||
      !owner.owns(bookId) ||
      loaded.id !== conversationId ||
      loaded.bookId !== bookId ||
      loaded.annotationId !== annotationId
    ) {
      return false;
    }
    this.#conversations.set(key, loaded);
    this.#emit();
    return true;
  }

  async refresh(conversationId: string): Promise<boolean> {
    const current = [...this.#conversations.entries()].find(
      ([, value]) => value.id === conversationId,
    );
    if (!current) return false;
    const [key, value] = current;
    if (this.#deleted.has(key) || this.#pendingDeletes.has(key)) return false;
    const generation = this.#nextGeneration(key);
    let loaded: ConversationHistory;
    try {
      loaded = freezeConversation(await this.#api.get(value.bookId, value.id));
    } catch (error) {
      if (
        generation !== this.#generations.get(key) ||
        this.#deleted.has(key) ||
        this.#pendingDeletes.has(key)
      ) {
        return false;
      }
      throw error;
    }
    if (
      generation !== this.#generations.get(key) ||
      this.#deleted.has(key) ||
      this.#pendingDeletes.has(key) ||
      loaded.id !== value.id ||
      loaded.bookId !== value.bookId ||
      loaded.annotationId !== value.annotationId
    ) {
      return false;
    }
    this.#conversations.set(key, loaded);
    this.#emit();
    return true;
  }

  async delete(value: ConversationHistory): Promise<void> {
    const key = conversationKey(value.bookId, value.id);
    if (this.#pendingDeletes.has(key) || this.#deleted.has(key)) return;
    this.#pendingDeletes.add(key);
    this.#nextGeneration(key);
    this.#emit();
    try {
      await this.#api.delete(value.bookId, value.id, value.annotationId);
    } catch (error) {
      this.#pendingDeletes.delete(key);
      this.#emit();
      throw error;
    }
    this.#pendingDeletes.delete(key);
    this.#deleted.add(key);
    this.#conversations.delete(key);
    this.#emit();
    for (const listener of this.#deletedListeners)
      listener(value.bookId, value.annotationId);
  }

  isDeleted(conversationId: string): boolean {
    const suffix = `:${conversationId}`;
    return [...this.#deleted].some((key) => key.endsWith(suffix));
  }

  #nextGeneration(key: string): number {
    const next = (this.#generations.get(key) ?? 0) + 1;
    this.#generations.set(key, next);
    return next;
  }

  #emit(): void {
    this.#revision += 1;
    this.#snapshot = freezeSnapshot(
      [...this.#conversations.values()].sort(
        (left, right) =>
          left.updatedAt.localeCompare(right.updatedAt) ||
          left.id.localeCompare(right.id),
      ),
      this.#pendingDeletes,
      this.#revision,
    );
    for (const listener of this.#listeners) listener();
  }
}

export const conversationPanels = new ConversationPanelStore();

function parseConversation(value: unknown): ConversationHistory {
  return freezeConversation(conversation.parse(value) as ConversationHistory);
}

function freezeConversation(value: ConversationHistory): ConversationHistory {
  return Object.freeze({
    ...value,
    anchor: deepFreeze(value.anchor),
    messages: Object.freeze(
      value.messages.map((item) =>
        Object.freeze({
          ...item,
          citations: Object.freeze(
            item.citations.map((entry) => deepFreeze(entry)),
          ),
        }),
      ),
    ),
  });
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) {
      deepFreeze(child);
    }
  }
  return value;
}

function conversationKey(bookId: string, conversationId: string): string {
  return `${bookId}:${conversationId}`;
}

function freezeSnapshot(
  values: readonly ConversationHistory[],
  pending: ReadonlySet<string>,
  revision: number,
): ConversationPanelSnapshot {
  return Object.freeze({
    conversations: Object.freeze([...values]),
    pendingDeletes: new Set(pending),
    revision,
  });
}

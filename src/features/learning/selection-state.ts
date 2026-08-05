import type { ContentAnchor } from '../../lib/generated/document';
import type {
  LearningAction as PreparedLearningAction,
  LearningContentKind,
  PreparationSummary,
  PrepareLearningRequestMetadata,
} from './learning-contract';
import type {
  RegionSelectionResult,
  SelectionSnapshot,
} from '../reader/contracts';

export const LEARNING_MENU_ACTIONS = Object.freeze([
  'explain',
  'example',
  'derive',
  'translate',
  'ask',
  'note',
] as const);
export type LearningMenuAction = (typeof LEARNING_MENU_ACTIONS)[number];

export interface LearningProfile {
  readonly id: string;
  readonly modelId: string;
}

export interface MenuPosition {
  readonly x: number;
  readonly y: number;
}

export interface CapturedRegion {
  readonly mimeType: 'image/png';
  readonly width: number;
  readonly height: number;
  readonly bytes: Uint8Array;
  release(): void;
}

/**
 * This is deliberately a value object.  It never points at Selection, a DOM
 * node, a reader adapter, or a viewport rectangle, so rerendering cannot
 * silently change the learning request.
 */
export interface LearningSelectionSnapshot {
  readonly bookId: string;
  readonly sectionId: string;
  readonly profile: LearningProfile;
  readonly anchor: ContentAnchor;
  readonly selectedText: string | null;
  readonly contentKind: LearningContentKind;
  readonly origin: 'text' | 'region';
  readonly position: MenuPosition;
  readonly capture: CapturedRegion | null;
}

export interface PreparedLearningHandoff {
  readonly preparationId: string;
  readonly summary: Readonly<PreparationSummary>;
  readonly action: PreparedLearningAction;
}

/** Phase 12 owns consuming the opaque preparation and any persisted UI. */
export interface LearningSurfacePort {
  handoff(prepared: Readonly<PreparedLearningHandoff>): void | Promise<void>;
}

export const DEFERRED_LEARNING_HANDOFF_EVENT = 'textbooklens:learning-prepared';

/** Phase 11 emits only an ephemeral safe boundary signal. Phase 12 owns consumption. */
export const deferredLearningSurfacePort: LearningSurfacePort = Object.freeze({
  handoff(prepared: Readonly<PreparedLearningHandoff>) {
    window.dispatchEvent(
      new CustomEvent(DEFERRED_LEARNING_HANDOFF_EVENT, { detail: prepared }),
    );
  },
});

export const unavailableLearningSurfacePort: LearningSurfacePort =
  Object.freeze({
    handoff() {
      throw new Error('LEARNING_SURFACE_UNAVAILABLE');
    },
  });

export function menuSnapshotFromText(
  selection: SelectionSnapshot,
  input: {
    bookId: string;
    sectionId: string;
    profile: LearningProfile;
    position: MenuPosition;
  },
): Readonly<LearningSelectionSnapshot> {
  return freezeSnapshot({
    ...input,
    anchor: { kind: 'text', selection: clone(selection.anchor) },
    selectedText: selection.text,
    contentKind: 'text_selection',
    origin: 'text',
    capture: null,
  });
}

export function menuSnapshotFromRegion(
  region: RegionSelectionResult,
  input: {
    bookId: string;
    sectionId: string;
    profile: LearningProfile;
    position: MenuPosition;
  },
): Readonly<LearningSelectionSnapshot> {
  const anchor = clone(region.anchor);
  if (anchor.kind === 'region' && anchor.region.locator.format === 'epub') {
    anchor.region.locator.sectionId = input.sectionId;
  }
  return freezeSnapshot({
    ...input,
    anchor,
    selectedText: region.text,
    contentKind: region.capture ? 'visual_region' : 'reliable_text_region',
    origin: 'region',
    capture: region.capture,
  });
}

export function preparationMetadata(
  snapshot: LearningSelectionSnapshot,
  action: Exclude<LearningMenuAction, 'note'>,
  value: string | null,
): Readonly<PrepareLearningRequestMetadata> {
  return Object.freeze({
    bookId: snapshot.bookId,
    sectionId: snapshot.sectionId,
    providerProfileId: snapshot.profile.id,
    modelId: snapshot.profile.modelId,
    action,
    contentKind: snapshot.contentKind,
    anchor: snapshot.anchor,
    selectedText: snapshot.selectedText,
    question: action === 'ask' ? value : null,
    targetLanguage: action === 'translate' ? value : null,
  });
}

export function clampMenuPosition(
  position: MenuPosition,
  menu: { width: number; height: number },
  viewport: Pick<
    VisualViewport,
    'width' | 'height' | 'offsetLeft' | 'offsetTop'
  > | null = window.visualViewport,
): MenuPosition {
  const left = viewport?.offsetLeft ?? 0;
  const top = viewport?.offsetTop ?? 0;
  const width = viewport?.width ?? window.innerWidth;
  const height = viewport?.height ?? window.innerHeight;
  return Object.freeze({
    x: Math.max(
      left,
      Math.min(position.x, left + Math.max(0, width - menu.width)),
    ),
    y: Math.max(
      top,
      Math.min(position.y, top + Math.max(0, height - menu.height)),
    ),
  });
}

export function releaseSnapshotCapture(
  snapshot: LearningSelectionSnapshot | null,
) {
  snapshot?.capture?.bytes.fill(0);
  snapshot?.capture?.release();
}

function freezeSnapshot(
  snapshot: LearningSelectionSnapshot,
): Readonly<LearningSelectionSnapshot> {
  // Typed arrays cannot be frozen in all supported engines.  They stay private
  // to the staging path and are released on every close/cancel path.
  return Object.freeze({
    ...snapshot,
    profile: Object.freeze({ ...snapshot.profile }),
    anchor: deepFreeze(snapshot.anchor),
    position: Object.freeze({ ...snapshot.position }),
  });
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value as object)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { NoteEditor, type NoteEditorLabels } from '../notes/NoteEditor';
import type { Note, NotesApi } from '../notes/api';
import type { LearningApi } from './api';
import {
  LearningConfirmationDialog,
  type LearningConfirmationLabels,
} from './LearningConfirmationDialog';
import type {
  PreparationSummary,
  RegionCaptureMetadata,
} from './learning-contract';
import {
  clampMenuPosition,
  preparationMetadata,
  releaseSnapshotCapture,
  type LearningMenuAction,
  type LearningSelectionSnapshot,
  type LearningSurfacePort,
} from './selection-state';
import { useLearningSurfacePort } from './LearningRequestProvider';

const MAX_INLINE_CODE_POINTS = 16_384;

export interface SelectionMenuLabels {
  menu: string;
  explain: string;
  example: string;
  derive: string;
  translate: string;
  ask: string;
  note: string;
  input: string;
  submit: string;
  unavailable: string;
  error: string;
  noteEditor: NoteEditorLabels;
  confirmation: LearningConfirmationLabels;
}

interface SelectionMenuProps {
  snapshot: Readonly<LearningSelectionSnapshot>;
  api: LearningApi;
  noteApi: NotesApi;
  surface: LearningSurfacePort;
  labels: SelectionMenuLabels;
  onClose(): void;
  onError(message: string): void;
  onNoteSaved?(note: Readonly<Note>): void;
}

export function SelectionMenu({
  snapshot,
  api,
  noteApi,
  surface,
  labels,
  onClose,
  onError,
  onNoteSaved,
}: SelectionMenuProps) {
  const globalSurface = useLearningSurfacePort();
  const learningSurface = globalSurface ?? surface;
  const menuRef = useRef<HTMLDivElement>(null);
  const buttonsRef = useRef<Array<HTMLButtonElement | null>>([]);
  const live = useRef(true);
  const inFlight = useRef(false);
  const [position, setPosition] = useState(snapshot.position);
  const [activeAction, setActiveAction] = useState<LearningMenuAction | null>(
    null,
  );
  const [value, setValue] = useState('');
  const [composing, setComposing] = useState(false);
  const [pending, setPending] = useState<{
    action: Exclude<LearningMenuAction, 'note'>;
    value: string | null;
    summary: Readonly<PreparationSummary>;
  } | null>(null);

  useEffect(() => {
    live.current = true;
    const reposition = () => {
      const element = menuRef.current;
      if (!element) return;
      setPosition(
        clampMenuPosition(snapshot.position, element.getBoundingClientRect()),
      );
    };
    reposition();
    buttonsRef.current[0]?.focus();
    window.visualViewport?.addEventListener('resize', reposition);
    window.addEventListener('resize', reposition);
    return () => {
      live.current = false;
      releaseSnapshotCapture(snapshot);
      window.visualViewport?.removeEventListener('resize', reposition);
      window.removeEventListener('resize', reposition);
    };
  }, [snapshot]);

  const close = () => {
    live.current = false;
    releaseSnapshotCapture(snapshot);
    onClose();
  };
  const fail = (summary?: Readonly<PreparationSummary>) => {
    live.current = false;
    if (summary) void api.discard(summary.preparationId).catch(() => {});
    releaseSnapshotCapture(snapshot);
    onError(labels.error);
    onClose();
  };
  const finishHandoff = async (
    action: Exclude<LearningMenuAction, 'note'>,
    summary: Readonly<PreparationSummary>,
    operationToken: string | null = null,
  ) => {
    try {
      if (operationToken) await stageCapture(summary, operationToken);
      if (!live.current) {
        await api.discard(summary.preparationId);
        releaseSnapshotCapture(snapshot);
        return;
      }
      await learningSurface.handoff(
        Object.freeze({
          preparationId: summary.preparationId,
          summary,
          action,
          selectionLabel: selectionLabel(snapshot),
        }),
      );
      releaseSnapshotCapture(snapshot);
      if (live.current) {
        live.current = false;
        onClose();
      }
    } catch {
      fail(summary);
    } finally {
      inFlight.current = false;
    }
  };
  const run = async (
    action: Exclude<LearningMenuAction, 'note'>,
    input: string | null,
  ) => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const summary = await api.prepare(
        preparationMetadata(snapshot, action, input),
      );
      if (!live.current) {
        await api.discard(summary.preparationId);
        releaseSnapshotCapture(snapshot);
        return;
      }
      if (summary.requiresBlockingConfirmation) {
        setPending({ action, value: input, summary });
        inFlight.current = false;
        return;
      }
      await finishHandoff(action, summary);
    } catch {
      fail();
      inFlight.current = false;
    }
  };
  const selectAction = (action: LearningMenuAction) => {
    if (action === 'translate' || action === 'ask' || action === 'note') {
      setActiveAction(action);
      setValue('');
      return;
    }
    void run(action, null);
  };
  const submitInline = () => {
    const trimmed = value.trim();
    if (!trimmed || [...trimmed].length > MAX_INLINE_CODE_POINTS) return;
    if (activeAction && activeAction !== 'note') {
      void run(activeAction, trimmed);
    }
  };
  const onMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const current = buttonsRef.current.findIndex(
      (button) => button === document.activeElement,
    );
    const count = buttonsRef.current.length;
    if (!count) return;
    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? count - 1
          : (current + (event.key === 'ArrowDown' ? 1 : count - 1)) % count;
    buttonsRef.current[next]?.focus();
  };
  const confirm = async (skipPrompt: boolean) => {
    if (!pending || inFlight.current) return;
    inFlight.current = true;
    try {
      const token = await api.authorize(pending.summary.preparationId, 'allow');
      if (skipPrompt) await setNoPrompt(snapshot.profile.id, pending.summary);
      if (!token && snapshot.capture) throw new Error('missing authorization');
      await finishHandoff(
        pending.action,
        pending.summary,
        snapshot.capture ? token : null,
      );
    } catch {
      fail(pending.summary);
      inFlight.current = false;
    }
  };
  const deny = () => {
    if (!pending) return;
    void api.authorize(pending.summary.preparationId, 'deny').catch(() => {});
    void api.discard(pending.summary.preparationId).catch(() => {});
    releaseSnapshotCapture(snapshot);
    live.current = false;
    onClose();
  };
  const actions: LearningMenuAction[] = [
    'explain',
    'example',
    'derive',
    'translate',
    'ask',
    'note',
  ];
  return (
    <>
      <div
        ref={menuRef}
        aria-label={labels.menu}
        className="learning-selection-menu"
        onKeyDown={onMenuKeyDown}
        role="menu"
        tabIndex={-1}
        style={{ left: position.x, position: 'fixed', top: position.y }}
      >
        {actions.map((action, index) => (
          <button
            key={action}
            ref={(element) => {
              buttonsRef.current[index] = element;
            }}
            type="button"
            role="menuitem"
            onClick={() => selectAction(action)}
          >
            {labels[action]}
          </button>
        ))}
        {activeAction === 'note' ? (
          <NoteEditor
            api={noteApi}
            anchor={snapshot.anchor}
            bookId={snapshot.bookId}
            labels={labels.noteEditor}
            sectionId={snapshot.sectionId}
            selectedText={snapshot.selectedText}
            onCancel={() => setActiveAction(null)}
            onSaved={(note) => {
              onNoteSaved?.(note);
              close();
            }}
          />
        ) : activeAction ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              submitInline();
            }}
          >
            <textarea
              aria-label={labels.input}
              maxLength={MAX_INLINE_CODE_POINTS}
              value={value}
              onCompositionEnd={() => setComposing(false)}
              onCompositionStart={() => setComposing(true)}
              onChange={(event) => setValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  event.preventDefault();
                  setActiveAction(null);
                }
                if (event.key === 'Enter' && !event.shiftKey && !composing) {
                  event.preventDefault();
                  submitInline();
                }
              }}
            />
            <button type="submit" disabled={!value.trim()}>
              {labels.submit}
            </button>
          </form>
        ) : null}
      </div>
      {pending && (
        <LearningConfirmationDialog
          labels={labels.confirmation}
          summary={pending.summary}
          onCancel={deny}
          onConfirm={(skipPrompt) => {
            void confirm(skipPrompt);
          }}
        />
      )}
    </>
  );

  async function stageCapture(
    summary: Readonly<PreparationSummary>,
    operationToken: string,
  ) {
    const capture = snapshot.capture;
    if (!capture || snapshot.anchor.kind !== 'region') return;
    const bytes = Uint8Array.from(capture.bytes);
    const hash = await digest(bytes);
    const metadata: RegionCaptureMetadata = {
      preparationId: summary.preparationId,
      operationToken,
      bookId: snapshot.bookId,
      providerProfileId: snapshot.profile.id,
      modelId: snapshot.profile.modelId,
      anchorContentSha256: snapshot.anchor.region.contentSha256,
      captureSha256: hash,
      schemaVersion: 1,
      mimeType: capture.mimeType,
      width: capture.width,
      height: capture.height,
      decodedPixelCount: capture.width * capture.height,
      encodedByteLength: bytes.byteLength,
    };
    await api.stageRegionCapture(metadata, bytes);
  }
}

function selectionLabel(snapshot: Readonly<LearningSelectionSnapshot>): string {
  return snapshot.origin === 'region' ? 'Selected region' : 'Selected text';
}

async function setNoPrompt(
  profileId: string,
  summary: Readonly<PreparationSummary>,
) {
  const categories = summary.riskFlags.map((risk) =>
    risk === 'image_send' ? 'image_send' : 'cost_risk',
  );
  await Promise.all(
    [...new Set(categories)].map((category) =>
      invoke('update_provider_operation_consent', {
        profileId,
        category,
        decision: 'skip_prompt',
      }),
    ),
  );
}

async function digest(bytes: Uint8Array) {
  const copy = Uint8Array.from(bytes);
  const hash = await crypto.subtle.digest(
    'SHA-256',
    copy.buffer as ArrayBuffer,
  );
  copy.fill(0);
  return [...new Uint8Array(hash)]
    .map((value) => value.toString(16).padStart(2, '0'))
    .join('');
}

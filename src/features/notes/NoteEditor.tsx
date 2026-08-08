import { useEffect, useRef, useState } from 'react';

import { useModalFocus } from '../../components/useModalFocus';
import type { ContentAnchor } from '../../lib/generated/document';
import {
  MAX_NOTE_CODE_POINTS,
  isNoteError,
  type Note,
  type NotesApi,
} from './api';

export interface NoteEditorLabels {
  input: string;
  save: string;
  cancel: string;
  empty: string;
  tooLong: string;
  error: string;
  conflict: string;
  reload: string;
  preserve: string;
  delete: string;
  deleteConfirm: string;
  confirmDelete: string;
  cancelDelete: string;
}

interface NoteEditorProps {
  api: NotesApi;
  bookId: string;
  sectionId: string;
  anchor: ContentAnchor;
  selectedText: string | null;
  labels: NoteEditorLabels;
  note?: Readonly<Note>;
  onSaved(note: Readonly<Note>): void;
  onDeleted?(): void;
  onCancel(): void;
}

export function NoteEditor({
  api,
  bookId,
  sectionId,
  anchor,
  selectedText,
  labels,
  note,
  onSaved,
  onDeleted,
  onCancel,
}: NoteEditorProps) {
  const [draft, setDraft] = useState(note?.noteText ?? '');
  const [revision, setRevision] = useState(note?.revision ?? null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const inFlight = useRef(false);
  const live = useRef(true);
  const draftRef = useRef(draft);
  const deleteTrigger = useRef<HTMLButtonElement>(null);
  const deleteDialogRef = useModalFocus<HTMLDivElement>(
    Boolean(note && confirmingDelete),
    () => setConfirmingDelete(false),
  );

  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      draftRef.current = '';
    };
  }, []);

  const updateDraft = (value: string) => {
    draftRef.current = value;
    setDraft(value);
    setFeedback(null);
  };
  const cancel = () => {
    draftRef.current = '';
    setDraft('');
    onCancel();
  };
  const submit = async () => {
    if (inFlight.current) return;
    const codePoints = [...draft].length;
    if (!draft.trim()) {
      setFeedback(labels.empty);
      return;
    }
    if (codePoints > MAX_NOTE_CODE_POINTS) {
      setFeedback(labels.tooLong);
      return;
    }
    inFlight.current = true;
    setFeedback(null);
    try {
      const saved = note
        ? await api.update({
            bookId,
            noteId: note.id,
            expectedRevision: revision ?? note.revision,
            noteText: draft,
          })
        : await api.create({
            bookId,
            sectionId,
            anchor,
            selectedText,
            noteText: draft,
          });
      if (!live.current) return;
      setRevision(saved.revision);
      setConflict(false);
      draftRef.current = '';
      onSaved(saved);
    } catch (error) {
      if (!live.current) return;
      if (isNoteError(error) && error.code === 'REQUEST_CONFLICT' && note) {
        setConflict(true);
        setFeedback(labels.conflict);
      } else {
        setFeedback(labels.error);
      }
    } finally {
      inFlight.current = false;
    }
  };
  const resolveConflict = async (preserveDraft: boolean) => {
    if (!note || inFlight.current) return;
    inFlight.current = true;
    try {
      const current = await api.get(bookId, note.id);
      if (!live.current) return;
      setRevision(current.revision);
      if (!preserveDraft) updateDraft(current.noteText);
      setConflict(false);
      setFeedback(null);
    } catch {
      if (live.current) setFeedback(labels.error);
    } finally {
      inFlight.current = false;
    }
  };
  const remove = async () => {
    if (!note || inFlight.current) return;
    inFlight.current = true;
    try {
      await api.delete({
        bookId,
        noteId: note.id,
        expectedRevision: revision ?? note.revision,
      });
      if (!live.current) return;
      draftRef.current = '';
      setDraft('');
      onDeleted?.();
    } catch (error) {
      if (!live.current) return;
      if (isNoteError(error) && error.code === 'REQUEST_CONFLICT') {
        setConflict(true);
        setFeedback(labels.conflict);
      } else {
        setFeedback(labels.error);
      }
    } finally {
      inFlight.current = false;
    }
  };

  return (
    <form
      aria-label={labels.input}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <textarea
        aria-label={labels.input}
        value={draft}
        onChange={(event) => updateDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            cancel();
          }
        }}
      />
      <button type="submit">{labels.save}</button>
      <button type="button" onClick={cancel}>
        {labels.cancel}
      </button>
      {note ? (
        <button
          ref={deleteTrigger}
          type="button"
          onClick={() => setConfirmingDelete(true)}
        >
          {labels.delete}
        </button>
      ) : null}
      {note && confirmingDelete ? (
        <div
          ref={deleteDialogRef}
          role="alertdialog"
          aria-modal="true"
          aria-label={labels.deleteConfirm}
        >
          <p>{labels.deleteConfirm}</p>
          <button type="button" onClick={() => void remove()}>
            {labels.confirmDelete}
          </button>
          <button type="button" onClick={() => setConfirmingDelete(false)}>
            {labels.cancelDelete}
          </button>
        </div>
      ) : null}
      {conflict ? (
        <div role="group" aria-label={labels.conflict}>
          <button type="button" onClick={() => void resolveConflict(false)}>
            {labels.reload}
          </button>
          <button type="button" onClick={() => void resolveConflict(true)}>
            {labels.preserve}
          </button>
        </div>
      ) : null}
      {feedback ? (
        <p aria-live="polite" role="status">
          {feedback}
        </p>
      ) : null}
    </form>
  );
}

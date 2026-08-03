import { useCallback, useEffect, useReducer, useState } from 'react';
import { useBlocker } from 'react-router-dom';

import { useMessage } from '../../app/LanguageProvider';
import { toUserError } from '../../lib/errors';
import { TeachingInstructionEditor } from './TeachingInstructionEditor';
import { TeachingPresetMenu } from './TeachingPresetMenu';
import { TeachingTestPanel } from './TeachingTestPanel';
import { TauriTeachingApi, type TeachingApi } from './api';
import { initialTeachingState, teachingReducer } from './teaching-state';

export function TeachingInstructionsPage({
  api: suppliedApi,
}: {
  api?: TeachingApi;
} = {}) {
  const message = useMessage();
  const [api] = useState<TeachingApi>(
    () => suppliedApi ?? new TauriTeachingApi(),
  );
  const [state, dispatch] = useReducer(teachingReducer, initialTeachingState);
  const [presetConfirm, setPresetConfirm] = useState<string | null>(null);
  const blocker = useBlocker(state.dirty);

  const load = useCallback(async () => {
    dispatch({ type: 'loading' });
    try {
      const [instruction, learningProfileAvailable] = await Promise.all([
        api.getInstruction(),
        api.hasAvailableLearningProfile(),
      ]);
      dispatch({ type: 'loaded', instruction, learningProfileAvailable });
    } catch (error) {
      dispatch({ type: 'error', error: toUserError(error) });
    }
  }, [api]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const warnBeforeUnload = (event: BeforeUnloadEvent) => {
      if (!state.dirty) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', warnBeforeUnload);
    return () => window.removeEventListener('beforeunload', warnBeforeUnload);
  }, [state.dirty]);

  const save = useCallback(async () => {
    if (state.stage === 'saving' || !state.persisted) return;
    dispatch({ type: 'saving' });
    try {
      const instruction = await api.updateInstruction({
        instruction: state.draft,
        expectedRevision: state.persisted.revision,
      });
      dispatch({ type: 'saved', instruction });
    } catch (error) {
      const userError = toUserError(error);
      if (userError.code === 'REQUEST_CONFLICT') {
        dispatch({ type: 'conflict', draft: state.draft });
      } else {
        dispatch({ type: 'error', error: userError });
      }
    }
  }, [api, state.draft, state.persisted, state.stage]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        void save();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [save]);

  const applyPreset = (instruction: string) => {
    if (state.dirty) {
      setPresetConfirm(instruction);
      return;
    }
    dispatch({ type: 'draftChanged', draft: instruction });
  };

  const current = state.persisted;
  return (
    <section
      aria-labelledby="teaching-instructions-title"
      className="phase-page"
    >
      <h1 id="teaching-instructions-title">{message('teaching.title')}</h1>
      <p>{message('teaching.description')}</p>
      {state.error ? (
        <div role="alert">
          <p>{state.error.message}</p>
          <p>{state.error.nextStep}</p>
        </div>
      ) : null}
      {state.stage === 'loading' || !current ? (
        <p aria-live="polite">{message('teaching.loading')}</p>
      ) : (
        <>
          <TeachingPresetMenu
            disabled={state.stage === 'saving'}
            onSelect={applyPreset}
          />
          <TeachingInstructionEditor
            draft={state.draft}
            dirty={state.dirty}
            stage={state.stage}
            onChange={(draft) => dispatch({ type: 'draftChanged', draft })}
            onSave={() => void save()}
            onClear={() => dispatch({ type: 'draftChanged', draft: '' })}
            onDefault={() => dispatch({ type: 'draftChanged', draft: '' })}
          />
          {state.stage === 'conflict' ? (
            <div role="alert">
              <p>{message('teaching.conflict')}</p>
              <button type="button" onClick={() => void load()}>
                {message('teaching.reload')}
              </button>
              <button
                type="button"
                onClick={() => dispatch({ type: 'preserveConflictDraft' })}
              >
                {message('teaching.copyDraft')}
              </button>
            </div>
          ) : null}
          <TeachingTestPanel
            api={api}
            instruction={state.draft}
            enabled={state.learningProfileAvailable}
          />
        </>
      )}
      {presetConfirm ? (
        <div
          aria-describedby="replace-draft-description"
          aria-modal="true"
          role="dialog"
        >
          <p id="replace-draft-description">
            {message('teaching.replaceDraft')}
          </p>
          <button
            type="button"
            onClick={() => {
              dispatch({ type: 'draftChanged', draft: presetConfirm });
              setPresetConfirm(null);
            }}
          >
            {message('teaching.replace')}
          </button>
          <button type="button" onClick={() => setPresetConfirm(null)}>
            {message('teaching.keepEditing')}
          </button>
        </div>
      ) : null}
      {blocker.state === 'blocked' ? (
        <div
          aria-describedby="leave-draft-description"
          aria-modal="true"
          role="dialog"
        >
          <p id="leave-draft-description">{message('teaching.leaveWarning')}</p>
          <button type="button" onClick={() => blocker.proceed?.()}>
            {message('teaching.leave')}
          </button>
          <button type="button" onClick={() => blocker.reset?.()}>
            {message('teaching.stay')}
          </button>
        </div>
      ) : null}
    </section>
  );
}

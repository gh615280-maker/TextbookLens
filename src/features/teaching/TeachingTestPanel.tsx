import { useState } from 'react';

import type { TeachingApi } from './api';

export function TeachingTestPanel({
  api,
  instruction,
  enabled,
}: {
  api: TeachingApi;
  instruction: string;
  enabled: boolean;
}) {
  void api;
  void instruction;
  const [open, setOpen] = useState(false);
  return (
    <section aria-labelledby="teaching-test-title">
      <h2 id="teaching-test-title">Temporary test</h2>
      <p>This preview is temporary and is not saved to learning history.</p>
      <button
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        {open ? 'Hide test' : 'Show test'}
      </button>
      {open ? (
        <fieldset disabled={!enabled}>
          <label htmlFor="teaching-test-question">Test question</label>
          <textarea id="teaching-test-question" maxLength={500} rows={4} />
          <button type="button">Run temporary test</button>
          {!enabled ? (
            <p>A default learning profile must be available.</p>
          ) : null}
        </fieldset>
      ) : null}
    </section>
  );
}

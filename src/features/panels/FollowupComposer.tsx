import { useState } from 'react';
import { useMessage } from '../../app/LanguageProvider';

const MAX_FOLLOWUP_CODE_POINTS = 16_384;

interface FollowupComposerProps {
  readonly disabled?: boolean;
  readonly providerChangeNotice: string | null;
  onSubmit(question: string): void;
}

export function FollowupComposer({
  disabled = false,
  providerChangeNotice,
  onSubmit,
}: FollowupComposerProps) {
  const message = useMessage();
  const [question, setQuestion] = useState('');
  const trimmed = question.trim();
  return (
    <form
      className="followup-composer"
      onSubmit={(event) => {
        event.preventDefault();
        if (!trimmed || disabled) return;
        onSubmit(trimmed);
        setQuestion('');
      }}
    >
      {providerChangeNotice ? <p role="note">{providerChangeNotice}</p> : null}
      <label>
        {message('panel.followup')}
        <textarea
          disabled={disabled}
          maxLength={MAX_FOLLOWUP_CODE_POINTS}
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
        />
      </label>
      <button disabled={disabled || !trimmed} type="submit">
        {message('panel.send')}
      </button>
    </form>
  );
}

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { PropsWithChildren } from 'react';

import { LanguageProvider, useLanguage } from '../../app/LanguageProvider';
import type { UiLanguage } from '../../lib/i18n';
import { TeachingInstructionEditor } from './TeachingInstructionEditor';

function settings(uiLanguage: UiLanguage) {
  return {
    onboardingCompleted: false,
    activeProviderProfileId: null,
    defaultLearningProfileId: null,
    defaultVisionProfileId: null,
    theme: 'system' as const,
    contextMode: 'standard' as const,
    uiLanguage,
    uiLanguageInitialized: true,
    firstReaderHintCompleted: false,
  };
}

const languageApi = {
  getAppSettings: async () => settings('en'),
  initializeUiLanguage: vi.fn(async (language: UiLanguage) =>
    settings(language),
  ),
  updateUiLanguage: vi.fn(async (language: UiLanguage) => settings(language)),
};

function EnglishLanguage({ children }: PropsWithChildren) {
  return <LanguageProvider api={languageApi}>{children}</LanguageProvider>;
}

describe('TeachingInstructionEditor', () => {
  it('counts Unicode scalar values and exposes draft and save states', async () => {
    const onChange = vi.fn();
    const onSave = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <TeachingInstructionEditor
        draft="A😀"
        dirty
        onChange={onChange}
        onClear={vi.fn()}
        onDefault={vi.fn()}
        onSave={onSave}
        stage="saved"
      />,
      { wrapper: EnglishLanguage },
    );

    expect(await screen.findByText('Unsaved changes')).toBeVisible();
    expect(screen.getByText('2 / 1000')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(onSave).toHaveBeenCalledOnce();
    await user.type(screen.getByLabelText('Instruction'), 'x');
    expect(onChange).toHaveBeenCalledWith('A😀x');

    rerender(
      <TeachingInstructionEditor
        draft=""
        dirty={false}
        onChange={onChange}
        onClear={vi.fn()}
        onDefault={vi.fn()}
        onSave={onSave}
        stage="saving"
      />,
    );
    expect(screen.getByText('Saving')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('updates visible editor text when the application language changes', async () => {
    const user = userEvent.setup();
    render(<LanguageSwitchFixture />, { wrapper: EnglishLanguage });

    expect(await screen.findByText('Teaching instruction')).toBeVisible();
    await user.click(screen.getByRole('button', { name: '切换为简体中文' }));

    expect(await screen.findByText('教学指令')).toBeVisible();
    expect(screen.getByLabelText('指令内容')).toBeVisible();
    expect(screen.getByRole('button', { name: '保存' })).toBeVisible();
  });
});

function LanguageSwitchFixture() {
  const { switchLanguage } = useLanguage();
  return (
    <>
      <button onClick={() => void switchLanguage('zh-CN')} type="button">
        切换为简体中文
      </button>
      <TeachingInstructionEditor
        draft=""
        dirty={false}
        onChange={vi.fn()}
        onClear={vi.fn()}
        onDefault={vi.fn()}
        onSave={vi.fn()}
        stage="saved"
      />
    </>
  );
}

import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { LearningConfirmationDialog } from './LearningConfirmationDialog';

describe('LearningConfirmationDialog', () => {
  it('renders only the safe preparation summary', () => {
    render(
      <LearningConfirmationDialog
        labels={{
          title: 'Confirm',
          details:
            '{provider}/{profile}/{model}/{tokens}/{sources}/{citations}',
          noPrompt: 'Never',
          cancel: 'Cancel',
          continue: 'Continue',
          imageRisk: 'Image',
          costRisk: 'Cost',
        }}
        summary={{
          preparationId: '44444444-4444-4444-8444-444444444444',
          providerDisplayName: 'Provider',
          profileDisplayName: 'Profile',
          modelDisplayName: 'Model',
          estimatedInputTokens: 12,
          sourceCount: 3,
          citationCount: 2,
          omittedSourceCount: 0,
          willSendImage: true,
          riskFlags: ['image_send', 'cost_risk'],
          requiresBlockingConfirmation: true,
          expiresAt: '2026-08-05T00:00:00Z',
          actionCategory: 'explain',
        }}
        onCancel={vi.fn()}
        onConfirm={vi.fn()}
      />,
    );
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'Provider/Profile/Model/12/3/2',
    );
    expect(screen.getByRole('dialog')).toHaveTextContent('Image');
    expect(screen.getByRole('dialog')).toHaveTextContent('Cost');
    expect(screen.getByRole('dialog')).not.toHaveTextContent(
      'PRIVATE_TEXTBOOK_BODY_SENTINEL',
    );
  });
});

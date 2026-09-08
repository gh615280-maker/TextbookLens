import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { toUserError, type UserFacingError } from '../../lib/errors';
import type {
  TeachingInstructionDto,
  UpdateTeachingInstruction,
} from '../../lib/generated/teaching';
import type { AppSettingsDto } from '../../lib/generated/settings';
import type { ProviderProfileSummary } from '../../lib/generated/provider';

export interface TeachingTestRequest {
  sessionId: string;
  requestId: string;
  instruction: string;
  question: string;
}

export interface TeachingTestEvent {
  requestId: string;
  type: 'text_delta' | 'usage' | 'completed' | 'error' | 'cancelled';
  text?: string;
  inputTokens?: number;
  outputTokens?: number;
  code?: UserFacingError['code'];
}

export interface TeachingApi {
  getInstruction(): Promise<TeachingInstructionDto>;
  updateInstruction(
    update: UpdateTeachingInstruction,
  ): Promise<TeachingInstructionDto>;
  hasAvailableLearningProfile(): Promise<boolean>;
  startTest?(request: TeachingTestRequest): Promise<void>;
  cancelTest?(sessionId: string, requestId: string): Promise<void>;
  listenTest?(handler: (event: TeachingTestEvent) => void): Promise<() => void>;
}

export class TauriTeachingApi implements TeachingApi {
  async getInstruction() {
    try {
      return await invoke<TeachingInstructionDto>('get_teaching_instruction');
    } catch (error) {
      throw toUserError(error);
    }
  }

  async updateInstruction(update: UpdateTeachingInstruction) {
    try {
      return await invoke<TeachingInstructionDto>(
        'update_teaching_instruction',
        {
          update,
        },
      );
    } catch (error) {
      throw toUserError(error);
    }
  }

  async hasAvailableLearningProfile() {
    try {
      const [settings, profiles] = await Promise.all([
        invoke<AppSettingsDto>('get_app_settings'),
        invoke<ProviderProfileSummary[]>('list_provider_profiles', {
          operation: 'text_learning',
        }),
      ]);
      return profiles.some(
        (profile) =>
          profile.id === settings.defaultLearningProfileId &&
          profile.credentialStatus !== 'missing',
      );
    } catch (error) {
      throw toUserError(error);
    }
  }

  async startTest(request: TeachingTestRequest) {
    try {
      await invoke('start_teaching_test', { request });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async cancelTest(sessionId: string, requestId: string) {
    try {
      await invoke('cancel_teaching_test', {
        request: { sessionId, requestId },
      });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async listenTest(handler: (event: TeachingTestEvent) => void) {
    return listen<TeachingTestEvent>('teaching-test-event', (event) => {
      handler(event.payload);
    });
  }
}

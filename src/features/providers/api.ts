import { invoke } from '@tauri-apps/api/core';

import { toUserError } from '../../lib/errors';
import type {
  AiOperation,
  LocalModelConnectResult,
  ProviderCapabilityRegistryDto,
  ProviderKind,
  ProviderOperationConsentCategory,
  ProviderOperationConsentDecision,
  ProviderProfileSummary,
} from '../../lib/generated/provider';
import type { AppSettingsDto } from '../../lib/generated/settings';

export interface SaveProviderProfileRequest {
  providerKind: ProviderKind;
  displayName: string;
  modelId?: string;
  credential: string;
}

export interface ProviderApi {
  connectLocal(): Promise<LocalModelConnectResult>;
  listCapabilities(): Promise<ProviderCapabilityRegistryDto>;
  listProfiles(operation?: AiOperation): Promise<ProviderProfileSummary[]>;
  getSettings(): Promise<AppSettingsDto>;
  validateAndSave(
    request: SaveProviderProfileRequest,
  ): Promise<ProviderProfileSummary>;
  replaceCredential(
    profileId: string,
    credential: string,
  ): Promise<ProviderProfileSummary>;
  deleteProfile(profileId: string): Promise<void>;
  setDefault(operation: AiOperation, profileId: string): Promise<void>;
  updateConsent(
    profileId: string,
    category: ProviderOperationConsentCategory,
    decision: ProviderOperationConsentDecision,
  ): Promise<void>;
  resetConsents(profileId?: string): Promise<void>;
}

export class TauriProviderApi implements ProviderApi {
  async connectLocal() {
    try {
      return await invoke<LocalModelConnectResult>('connect_local_models');
    } catch (error) {
      throw toUserError(error);
    }
  }
  async listCapabilities() {
    return invoke<ProviderCapabilityRegistryDto>('list_provider_capabilities');
  }
  async listProfiles(operation?: AiOperation) {
    try {
      return await invoke<ProviderProfileSummary[]>('list_provider_profiles', {
        operation: operation ?? null,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }
  async getSettings() {
    return invoke<AppSettingsDto>('get_app_settings');
  }
  async validateAndSave(request: SaveProviderProfileRequest) {
    try {
      return await invoke<ProviderProfileSummary>(
        'validate_and_save_provider_profile',
        { request },
      );
    } catch (error) {
      throw toUserError(error);
    }
  }
  async replaceCredential(profileId: string, credential: string) {
    try {
      return await invoke<ProviderProfileSummary>(
        'replace_provider_profile_credential',
        { profileId, credential },
      );
    } catch (error) {
      throw toUserError(error);
    }
  }
  async deleteProfile(profileId: string) {
    try {
      await invoke('delete_provider_profile', { profileId });
    } catch (error) {
      throw toUserError(error);
    }
  }
  async setDefault(operation: AiOperation, profileId: string) {
    try {
      await invoke('set_default_provider_profile', { operation, profileId });
    } catch (error) {
      throw toUserError(error);
    }
  }
  async updateConsent(
    profileId: string,
    category: ProviderOperationConsentCategory,
    decision: ProviderOperationConsentDecision,
  ) {
    try {
      await invoke('update_provider_operation_consent', {
        profileId,
        category,
        decision,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }
  async resetConsents(profileId?: string) {
    try {
      await invoke('reset_provider_operation_consents', {
        profileId: profileId ?? null,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }
}

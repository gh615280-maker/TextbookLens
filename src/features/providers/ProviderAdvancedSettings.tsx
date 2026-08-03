import type { ProviderCapability } from '../../lib/generated/provider';

interface Props {
  provider: ProviderCapability;
  modelId: string;
  onModelChange(modelId: string): void;
}
export function ProviderAdvancedSettings({
  provider,
  modelId,
  onModelChange,
}: Props) {
  return (
    <details>
      <summary>Advanced model settings</summary>
      <label>
        Model
        <select
          value={modelId}
          onChange={(event) => onModelChange(event.target.value)}
        >
          {provider.models.map((model) => (
            <option key={model.id} value={model.id}>
              {model.displayName}
            </option>
          ))}
        </select>
      </label>
    </details>
  );
}

import type { ProviderCapability } from '../../lib/generated/provider';
import { useMessage } from '../../app/LanguageProvider';

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
  const message = useMessage();
  return (
    <details>
      <summary>{message('aiServices.advanced')}</summary>
      <label>
        {message('aiServices.model')}
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

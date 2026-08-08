import { useMessage } from '../../app/LanguageProvider';
import { pickTextbook } from './picker';

interface ImportButtonProps {
  disabled?: boolean;
  onSelect(sourcePath: string): void | Promise<void>;
}

export function ImportButton({
  disabled = false,
  onSelect,
}: ImportButtonProps) {
  const message = useMessage();
  async function chooseTextbook() {
    const selected = await pickTextbook();
    if (selected) await onSelect(selected);
  }

  return (
    <button type="button" disabled={disabled} onClick={chooseTextbook}>
      {message('import.choose')}
    </button>
  );
}

import { pickTextbook } from './picker';

interface ImportButtonProps {
  disabled?: boolean;
  onSelect(sourcePath: string): void | Promise<void>;
}

export function ImportButton({
  disabled = false,
  onSelect,
}: ImportButtonProps) {
  async function chooseTextbook() {
    const selected = await pickTextbook();
    if (selected) await onSelect(selected);
  }

  return (
    <button type="button" disabled={disabled} onClick={chooseTextbook}>
      导入教材
    </button>
  );
}

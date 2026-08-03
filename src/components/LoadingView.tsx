import { useMessage } from '../app/LanguageProvider';

export function LoadingView() {
  const message = useMessage();
  return (
    <div aria-busy="true" role="status">
      {message('loading.label')}
    </div>
  );
}

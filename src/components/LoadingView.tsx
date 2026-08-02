import { formatMessage } from '../lib/i18n';

export function LoadingView() {
  return (
    <div aria-busy="true" role="status">
      {formatMessage('zh-CN', 'loading.label')}
    </div>
  );
}

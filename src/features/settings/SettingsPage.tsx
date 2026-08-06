import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';

import { useLanguage } from '../../app/LanguageProvider';
import type {
  MaintenanceErrorDto,
  MaintenanceStatusDto,
  StorageUsageDto,
} from '../../lib/generated/maintenance';
import { ReaderSettingsControls } from '../reader/ReaderSettingsControls';
import type { ReaderSettings } from '../reader/api';
import { tauriSettingsApi, type SettingsApi } from './api';
import { pickBackupDestination, pickBackupToRestore } from './picker';

const CLEAR_PHRASE = 'DELETE ALL TEXTBOOKLENS DATA';

const copy = {
  en: {
    title: 'Settings',
    reading: 'Reading experience',
    data: 'Data and privacy',
    about: 'About',
    readingHelp: 'These defaults are used when you open a textbook.',
    loading: 'Loading local data…',
    refresh: 'Refresh',
    openData: 'Open data directory',
    storage: 'Local storage',
    total: 'Total',
    source: 'Textbook copies',
    derived: 'Derived reading data',
    index: 'Local AI indexes',
    database: 'Database',
    cache: 'Cache',
    log: 'Logs',
    location:
      'TextbookLens keeps its app-managed data in its local Windows app-data location. The exact path is not shown here.',
    createBackup: 'Create backup',
    restoreBackup: 'Restore backup',
    clearAll: 'Clear all data',
    privacy: 'Privacy and sending boundary',
    privacyText:
      'Importing, reading, searching, annotations, language, backups, and restores work locally and do not require an AI network request. API keys do not enter SQLite, frontend state, logs, diagnostics, or backups.',
    backupTitle: 'Create local backup',
    backupWarning:
      'This backup includes app-owned textbook copies. Check copyright and sharing permissions before sharing it. It does not include API keys.',
    estimate: 'Estimated backup size: {size}.',
    chooseDestination: 'Choose .tlbackup location',
    cancel: 'Cancel',
    creating: 'Creating backup…',
    backupDone: 'Backup created safely.',
    backupFailed:
      'The backup could not be created safely. No details from your files were shown.',
    restoreTitle: 'Restore backup?',
    restoreWarning:
      'TextbookLens will verify the selected backup and replace current local data at a maintenance and restart boundary. If verification fails, current data is kept. Backup contents are not shown here.',
    restoreConfirm: 'Verify and prepare restore',
    restoring: 'Preparing restore…',
    restoreReady: 'Restore is ready. Restart TextbookLens to complete it.',
    credential:
      'AI credentials are device-local. Reconnect an AI service after restart if required.',
    restoreFailed:
      'The backup could not be verified or prepared. Current data was kept.',
    clearTitle: 'Clear all TextbookLens data?',
    clearWarning:
      'This cannot be undone. Every clear requires this exact confirmation.',
    clearItems: [
      'Database, settings, and teaching instructions',
      'App-owned textbook copies and derived reading files',
      'Local indexes and corrections',
      'Completed history, annotations, and markers',
      'Cache, logs, staging, trash, and remote temporary resource records',
      'TextbookLens credentials',
    ],
    clearPreserved:
      'Your original files and separately saved .tlbackup files are not deleted.',
    typePhrase: 'Type {phrase} to continue.',
    clearConfirm: 'Clear all data',
    clearing: 'Clearing local data…',
    clearReady: 'Local data was cleared. Restart TextbookLens to start again.',
    credentialRetry:
      'Local data was cleared, but credential cleanup still needs a retry. Do not treat this as complete.',
    clearFailed: 'Data was not fully cleared. Nothing is reported as complete.',
    restart: 'Restart now',
    restarting: 'Restarting…',
    busy: 'A maintenance operation cannot start while these local tasks are active: {operations}. Stop or finish them, then try again.',
    status: 'Maintenance status: {status}.',
    aboutText: 'TextbookLens is a local-first AI textbook reader.',
    project: 'Project and licenses are available with this installation.',
    unknown: 'A safe local operation could not be completed. Try again.',
  },
  'zh-CN': {
    title: '设置',
    reading: '阅读体验',
    data: '数据与隐私',
    about: '关于',
    readingHelp: '打开教材时将使用这些默认设置。',
    loading: '正在加载本地数据…',
    refresh: '刷新',
    openData: '打开数据目录',
    storage: '本地存储',
    total: '合计',
    source: '教材副本',
    derived: '派生阅读数据',
    index: '本地 AI 索引',
    database: '数据库',
    cache: '缓存',
    log: '日志',
    location:
      'TextbookLens 将应用管理的数据保存在本机 Windows 应用数据位置；此处不会显示确切路径。',
    createBackup: '创建备份',
    restoreBackup: '恢复备份',
    clearAll: '清除全部数据',
    privacy: '隐私与发送边界',
    privacyText:
      '导入、阅读、搜索、批注、语言、备份和恢复均在本地完成，不需要 AI 网络请求。API Key 不会进入 SQLite、前端状态、日志、诊断或备份。',
    backupTitle: '创建本地备份',
    backupWarning:
      '备份包含应用拥有的教材副本。共享前请确认版权和共享权限；备份不包含 API Key。',
    estimate: '预计备份大小：{size}。',
    chooseDestination: '选择 .tlbackup 保存位置',
    cancel: '取消',
    creating: '正在创建备份…',
    backupDone: '备份已安全创建。',
    backupFailed: '无法安全创建备份；未显示任何文件详情。',
    restoreTitle: '恢复备份？',
    restoreWarning:
      'TextbookLens 会验证所选备份，并在维护和重启边界替换当前本地数据。验证失败时会保留当前数据；此处不会显示备份内容。',
    restoreConfirm: '验证并准备恢复',
    restoring: '正在准备恢复…',
    restoreReady: '恢复已准备就绪。请重启 TextbookLens 以完成恢复。',
    credential: 'AI 凭据只保存在本机；如有需要，请在重启后重新连接 AI 服务。',
    restoreFailed: '无法验证或准备此备份，当前数据已保留。',
    clearTitle: '清除所有 TextbookLens 数据？',
    clearWarning: '此操作不可撤销，每次清除都必须输入以下精确确认语。',
    clearItems: [
      '数据库、设置和教学指令',
      '应用拥有的教材副本和派生阅读文件',
      '本地索引和修正',
      '已完成的历史、批注和标记',
      '缓存、日志、暂存区、回收区和远程临时资源记录',
      'TextbookLens 凭据',
    ],
    clearPreserved: '不会删除您的原始文件或单独保存的 .tlbackup 文件。',
    typePhrase: '输入 {phrase} 以继续。',
    clearConfirm: '清除全部数据',
    clearing: '正在清除本地数据…',
    clearReady: '本地数据已清除。请重启 TextbookLens 重新开始。',
    credentialRetry: '本地数据已清除，但凭据清理仍需重试；这不能视为已完成。',
    clearFailed: '数据未被完全清除；不会报告为完成。',
    restart: '立即重启',
    restarting: '正在重启…',
    busy: '以下本地任务正在运行，无法开始维护操作：{operations}。请先停止或完成相关任务后重试。',
    status: '维护状态：{status}。',
    aboutText: 'TextbookLens 是本地优先的 AI 教材阅读器。',
    project: '项目和许可证信息随此安装提供。',
    unknown: '无法完成安全的本地操作，请重试。',
  },
  'zh-TW': {
    title: '設定',
    reading: '閱讀體驗',
    data: '資料與隱私',
    about: '關於',
    readingHelp: '開啟教材時會使用這些預設設定。',
    loading: '正在載入本機資料…',
    refresh: '重新整理',
    openData: '開啟資料目錄',
    storage: '本機儲存空間',
    total: '合計',
    source: '教材副本',
    derived: '衍生閱讀資料',
    index: '本機 AI 索引',
    database: '資料庫',
    cache: '快取',
    log: '記錄',
    location:
      'TextbookLens 將應用程式管理的資料保存在本機 Windows 應用程式資料位置；此處不會顯示確切路徑。',
    createBackup: '建立備份',
    restoreBackup: '還原備份',
    clearAll: '清除所有資料',
    privacy: '隱私與傳送界線',
    privacyText:
      '匯入、閱讀、搜尋、註解、語言、備份及還原都在本機完成，不需要 AI 網路請求。API Key 不會進入 SQLite、前端狀態、記錄、診斷或備份。',
    backupTitle: '建立本機備份',
    backupWarning:
      '備份包含應用程式擁有的教材副本。分享前請確認版權與分享權限；備份不包含 API Key。',
    estimate: '預估備份大小：{size}。',
    chooseDestination: '選擇 .tlbackup 儲存位置',
    cancel: '取消',
    creating: '正在建立備份…',
    backupDone: '備份已安全建立。',
    backupFailed: '無法安全建立備份；未顯示任何檔案細節。',
    restoreTitle: '還原備份？',
    restoreWarning:
      'TextbookLens 會驗證所選備份，並在維護與重新啟動界線替換目前本機資料。驗證失敗時會保留目前資料；此處不會顯示備份內容。',
    restoreConfirm: '驗證並準備還原',
    restoring: '正在準備還原…',
    restoreReady: '還原已準備完成。請重新啟動 TextbookLens 以完成還原。',
    credential:
      'AI 憑據只保存在本機；如有需要，請在重新啟動後重新連接 AI 服務。',
    restoreFailed: '無法驗證或準備此備份，目前資料已保留。',
    clearTitle: '清除所有 TextbookLens 資料？',
    clearWarning: '此操作無法復原，每次清除都必須輸入下列精確確認語。',
    clearItems: [
      '資料庫、設定和教學指令',
      '應用程式擁有的教材副本和衍生閱讀檔案',
      '本機索引和修正',
      '已完成的歷程、註解和標記',
      '快取、記錄、暫存區、回收區和遠端臨時資源記錄',
      'TextbookLens 憑據',
    ],
    clearPreserved: '不會刪除您的原始檔或另行儲存的 .tlbackup 檔案。',
    typePhrase: '輸入 {phrase} 以繼續。',
    clearConfirm: '清除所有資料',
    clearing: '正在清除本機資料…',
    clearReady: '本機資料已清除。請重新啟動 TextbookLens 重新開始。',
    credentialRetry: '本機資料已清除，但憑據清理仍需重試；這不能視為已完成。',
    clearFailed: '資料未完全清除；不會回報為完成。',
    restart: '立即重新啟動',
    restarting: '正在重新啟動…',
    busy: '下列本機工作正在執行，無法開始維護操作：{operations}。請先停止或完成相關工作後再試。',
    status: '維護狀態：{status}。',
    aboutText: 'TextbookLens 是本機優先的 AI 教材閱讀器。',
    project: '專案與授權資訊隨此安裝提供。',
    unknown: '無法完成安全的本機操作，請再試一次。',
  },
} as const;

type Copy = (typeof copy)[keyof typeof copy];
type DialogKind = 'backup' | 'restore' | 'clear' | null;
type SettingsPicker = {
  pickBackupDestination(): Promise<string | null>;
  pickBackupToRestore(): Promise<string | null>;
};

export function SettingsPage({
  api = tauriSettingsApi,
  picker = { pickBackupDestination, pickBackupToRestore },
}: {
  api?: SettingsApi;
  picker?: SettingsPicker;
}) {
  const { uiLanguage } = useLanguage();
  const text = copy[uiLanguage] as Copy;
  const [usage, setUsage] = useState<StorageUsageDto | null>(null);
  const [maintenance, setMaintenance] = useState<MaintenanceStatusDto | null>(
    null,
  );
  const [readerSettings, setReaderSettings] = useState<ReaderSettings | null>(
    null,
  );
  const [dialog, setDialog] = useState<DialogKind>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [restartRequired, setRestartRequired] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clearValue, setClearValue] = useState('');

  const refresh = useCallback(async () => {
    setError(null);
    const [status, nextUsage] = await Promise.all([
      api.getMaintenanceStatus(),
      api.getStorageUsage(),
    ]);
    setMaintenance(status);
    setUsage(nextUsage);
  }, [api]);
  useEffect(() => {
    void Promise.resolve()
      .then(refresh)
      .catch(() => setError(text.unknown));
    void api
      .getReaderSettings()
      .then(setReaderSettings)
      .catch(() => undefined);
  }, [api, refresh, text.unknown]);
  const reportError = (value: unknown, fallback: string) => {
    const safe = value as Partial<MaintenanceErrorDto>;
    if (
      safe?.code === 'MAINTENANCE_BUSY' ||
      safe?.code === 'MAINTENANCE_SHUTTING_DOWN'
    ) {
      const operations =
        safe.activeOperations
          ?.map((item) => `${item.kind} (${item.count})`)
          .join(', ') || 'local work';
      setError(text.busy.replace('{operations}', operations));
    } else setError(fallback);
  };
  const guarded = async (work: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await work();
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const beginBackup = () =>
    guarded(async () => {
      setNotice(null);
      try {
        await refresh();
        setDialog('backup');
      } catch (value) {
        reportError(value, text.unknown);
      }
    });
  const createBackup = () =>
    guarded(async () => {
      const destination = await picker.pickBackupDestination();
      if (!destination) return;
      try {
        await api.createLocalBackup(destination);
        setDialog(null);
        setNotice(text.backupDone);
        await refresh();
      } catch (value) {
        reportError(value, text.backupFailed);
      }
    });
  const beginRestore = () =>
    guarded(async () => {
      const archive = await picker.pickBackupToRestore();
      if (archive) {
        restoreArchiveRef.current = archive;
        setDialog('restore');
      }
    });
  const restoreArchiveRef = useRef<string | null>(null);
  const restore = () =>
    guarded(async () => {
      const archive = restoreArchiveRef.current;
      if (!archive) return;
      try {
        const result = await api.restoreLocalBackup(archive);
        setDialog(null);
        setRestartRequired(result.restartRequired);
        setNotice(
          result.aiConfigurationRequired
            ? `${text.restoreReady} ${text.credential}`
            : text.restoreReady,
        );
      } catch (value) {
        reportError(value, text.restoreFailed);
      }
    });
  const clear = () =>
    guarded(async () => {
      try {
        const result = await api.clearAllTextbookLensData(clearValue);
        setDialog(null);
        setRestartRequired(
          result.status === 'CLEAR_READY_TO_RESTART' && result.restartRequired,
        );
        setNotice(
          result.status === 'CLEAR_CREDENTIAL_CLEANUP_REQUIRED'
            ? text.credentialRetry
            : text.clearReady,
        );
      } catch (value) {
        reportError(value, text.clearFailed);
      }
    });
  const restart = () =>
    guarded(async () => {
      setNotice(text.restarting);
      await api.restartApplication();
    });

  return (
    <section aria-labelledby="settings-title" className="settings-page">
      <h1 id="settings-title">{text.title}</h1>
      <section aria-labelledby="reading-title" className="settings-card">
        <h2 id="reading-title">{text.reading}</h2>
        <p>{text.readingHelp}</p>
        {readerSettings ? (
          <ReaderSettingsControls
            settings={readerSettings}
            format="defaults"
            language={uiLanguage}
            onChange={(next) =>
              void api
                .updateReaderSettings(next)
                .then(setReaderSettings)
                .catch(() => setError(text.unknown))
            }
          />
        ) : (
          <p>{text.loading}</p>
        )}
      </section>
      <section aria-labelledby="data-title" className="settings-card">
        <h2 id="data-title">{text.data}</h2>
        <p>{text.location}</p>
        <div className="button-row">
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void guarded(async () => {
                try {
                  await api.openAppDataDirectory();
                } catch (value) {
                  reportError(value, text.unknown);
                }
              })
            }
          >
            {text.openData}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void guarded(async () => {
                try {
                  await refresh();
                } catch (value) {
                  reportError(value, text.unknown);
                }
              })
            }
          >
            {text.refresh}
          </button>
        </div>
        <Storage usage={usage} text={text} />
        <p className="settings-maintenance">
          {maintenance
            ? text.status.replace('{status}', maintenance.code)
            : text.loading}
        </p>
        {maintenance && maintenance.activeOperations.length > 0 ? (
          <p className="settings-maintenance">
            {text.busy.replace(
              '{operations}',
              maintenance.activeOperations
                .map((item) => `${item.kind} (${item.count})`)
                .join(', '),
            )}
          </p>
        ) : null}
        <div className="button-row">
          <button
            type="button"
            disabled={busy}
            onClick={() => void beginBackup()}
          >
            {text.createBackup}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void beginRestore()}
          >
            {text.restoreBackup}
          </button>
          <button
            className="settings-danger"
            type="button"
            disabled={busy}
            onClick={() => {
              setClearValue('');
              setNotice(null);
              setError(null);
              setDialog('clear');
            }}
          >
            {text.clearAll}
          </button>
        </div>
        <h3>{text.privacy}</h3>
        <p>{text.privacyText}</p>
      </section>
      <section aria-labelledby="about-title" className="settings-card">
        <h2 id="about-title">{text.about}</h2>
        <p>{text.aboutText}</p>
        <p>{text.project}</p>
      </section>
      {notice ? (
        <div
          className="settings-notice"
          role="status"
          aria-label="Settings operation status"
          aria-live="polite"
        >
          <p>{notice}</p>
          {restartRequired && (
            <button
              type="button"
              disabled={busy}
              onClick={() => void restart()}
            >
              {text.restart}
            </button>
          )}
        </div>
      ) : null}
      {error ? (
        <p className="inline-error" role="alert">
          {error}
        </p>
      ) : null}
      {dialog === 'backup' && (
        <Dialog
          title={text.backupTitle}
          onClose={() => !busy && setDialog(null)}
        >
          <p>
            {text.estimate.replace(
              '{size}',
              formatBytes(usage?.totalBytes ?? 0n),
            )}
          </p>
          <p>{text.backupWarning}</p>
          <div className="button-row">
            <button
              type="button"
              disabled={busy}
              onClick={() => setDialog(null)}
            >
              {text.cancel}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void createBackup()}
            >
              {busy ? text.creating : text.chooseDestination}
            </button>
          </div>
        </Dialog>
      )}
      {dialog === 'restore' && (
        <Dialog
          title={text.restoreTitle}
          onClose={() => !busy && setDialog(null)}
        >
          <p>{text.restoreWarning}</p>
          <div className="button-row">
            <button
              type="button"
              disabled={busy}
              onClick={() => setDialog(null)}
            >
              {text.cancel}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void restore()}
            >
              {busy ? text.restoring : text.restoreConfirm}
            </button>
          </div>
        </Dialog>
      )}
      {dialog === 'clear' && (
        <Dialog
          title={text.clearTitle}
          onClose={() => !busy && setDialog(null)}
        >
          <p>{text.clearWarning}</p>
          <ul>
            {text.clearItems.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <p>{text.clearPreserved}</p>
          <label>
            {text.typePhrase.replace('{phrase}', CLEAR_PHRASE)}
            <input
              value={clearValue}
              disabled={busy}
              onChange={(event) => setClearValue(event.currentTarget.value)}
            />
          </label>
          <div className="button-row">
            <button
              type="button"
              disabled={busy}
              onClick={() => setDialog(null)}
            >
              {text.cancel}
            </button>
            <button
              className="settings-danger"
              type="button"
              disabled={busy || clearValue !== CLEAR_PHRASE}
              onClick={() => void clear()}
            >
              {busy ? text.clearing : text.clearConfirm}
            </button>
          </div>
        </Dialog>
      )}
    </section>
  );
}

function Storage({
  usage,
  text,
}: {
  usage: StorageUsageDto | null;
  text: Copy;
}) {
  const labels: Record<string, string> = {
    source: text.source,
    derived: text.derived,
    index: text.index,
    database: text.database,
    cache: text.cache,
    log: text.log,
  };
  return (
    <section aria-labelledby="storage-title">
      <h3 id="storage-title">{text.storage}</h3>
      {usage ? (
        <dl className="settings-storage">
          <div>
            <dt>{text.total}</dt>
            <dd>{formatBytes(usage.totalBytes)}</dd>
          </div>
          {usage.categories.map((item) => (
            <div key={item.category}>
              <dt>{labels[item.category]}</dt>
              <dd>{formatBytes(item.bytes)}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <p>{text.loading}</p>
      )}
    </section>
  );
}

function Dialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose(): void;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);
  useEffect(() => {
    previousFocus.current = document.activeElement as HTMLElement | null;
    dialogRef.current
      ?.querySelector<HTMLElement>('button, input, select, textarea')
      ?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const focusable = Array.from(
        dialogRef.current?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href]',
        ) ?? [],
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      previousFocus.current?.focus();
    };
  }, []);
  return (
    <div className="settings-dialog-backdrop">
      <div
        aria-modal="true"
        aria-label={title}
        className="settings-dialog"
        ref={dialogRef}
        role="dialog"
      >
        <h2>{title}</h2>
        {children}
      </div>
    </div>
  );
}

function formatBytes(value: bigint) {
  const bytes = Number(value);
  if (!Number.isFinite(bytes) || bytes < 1024) return `${Math.max(0, bytes)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let current = bytes / 1024;
  let index = 0;
  while (current >= 1024 && index < units.length - 1) {
    current /= 1024;
    index += 1;
  }
  return `${current.toFixed(current >= 10 || Number.isInteger(current) ? 0 : 1)} ${units[index]}`;
}

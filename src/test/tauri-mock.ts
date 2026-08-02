import type { InvokeArgs } from '@tauri-apps/api/core';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';

export interface RecordedIpcCall {
  command: string;
  payload: InvokeArgs | undefined;
}

export function installTauriMock(
  handler: (command: string, payload?: InvokeArgs) => unknown,
): RecordedIpcCall[] {
  const calls: RecordedIpcCall[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return handler(command, payload);
  });
  return calls;
}

export { clearMocks };

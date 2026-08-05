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
    calls.push({ command, payload: safeRecordedPayload(command, payload) });
    return handler(command, payload);
  });
  return calls;
}

function safeRecordedPayload(
  command: string,
  payload: InvokeArgs | undefined,
): InvokeArgs | undefined {
  if (command === 'stage_region_capture') {
    return {
      encodedByteLength:
        payload instanceof Uint8Array ? payload.byteLength : undefined,
    };
  }
  if (command === 'prepare_learning_request') {
    const metadata = asRecord(asRecord(payload)?.metadata);
    return {
      metadata: {
        action: metadata?.action,
        contentKind: metadata?.contentKind,
        hasSelectedText: typeof metadata?.selectedText === 'string',
        hasQuestion: typeof metadata?.question === 'string',
        hasTargetLanguage: typeof metadata?.targetLanguage === 'string',
      },
    };
  }
  if (command === 'authorize_learning_request') {
    const args = asRecord(payload);
    return { decision: args?.decision };
  }
  if (command === 'discard_learning_preparation') {
    return { preparationId: asRecord(payload)?.preparationId };
  }
  if (command === 'invalidate_learning_preparations') {
    const request = asRecord(asRecord(payload)?.request);
    return { request: { reason: request?.reason } };
  }
  if (
    command === 'start_learning_request' ||
    command === 'subscribe_learning_request' ||
    command === 'cancel_learning_request'
  ) {
    const args = asRecord(payload);
    return {
      requestId: args?.requestId,
      preparationId: args?.preparationId,
      afterSeq: args?.afterSeq,
    };
  }
  if (command === 'start_conversation_followup') {
    const args = asRecord(payload);
    return {
      conversationId: args?.conversationId,
      hasQuestion: typeof args?.question === 'string',
    };
  }
  if (
    command === 'get_learning_conversation' ||
    command === 'delete_learning_conversation'
  ) {
    const args = asRecord(payload);
    return {
      bookId: args?.bookId,
      conversationId: args?.conversationId,
      annotationId: args?.annotationId,
    };
  }
  if (command === 'create_note' || command === 'update_note') {
    const args = asRecord(payload);
    return {
      bookId: args?.bookId,
      noteId: args?.noteId,
      expectedRevision: args?.expectedRevision,
      hasNoteText: typeof args?.noteText === 'string',
      hasSelectedText: typeof args?.selectedText === 'string',
      anchorKind: asRecord(args?.anchor)?.kind,
    };
  }
  return payload;
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

export { clearMocks };

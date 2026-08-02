import { z } from 'zod';

const appErrorCodeSchema = z.enum([
  'INVALID_API_KEY',
  'MODEL_NOT_FOUND',
  'PROVIDER_PERMISSION_DENIED',
  'PROVIDER_REGION_RESTRICTED',
  'RATE_LIMITED',
  'INSUFFICIENT_QUOTA',
  'CONTEXT_TOO_LARGE',
  'NETWORK_OFFLINE',
  'PROVIDER_UNAVAILABLE',
  'PROVIDER_REFUSED',
  'UNSUPPORTED_FILE_TYPE',
  'FILE_CORRUPTED',
  'FILE_ENCRYPTED_OR_DRM',
  'NO_EXTRACTABLE_TEXT',
  'IMPORT_CANCELLED',
  'DATABASE_ERROR',
  'CREDENTIAL_STORE_ERROR',
  'ANCHOR_NOT_FOUND',
  'INVALID_INPUT',
  'NOT_FOUND',
  'BOOK_NOT_READY',
  'REQUEST_CONFLICT',
  'LOCAL_IO_ERROR',
]);

const appErrorDtoSchema = z.object({
  code: appErrorCodeSchema,
  message: z.string().min(1).max(1024),
  nextStep: z.string().min(1).max(1024),
  diagnosticId: z.uuid().nullable(),
});

export type UserFacingError = z.infer<typeof appErrorDtoSchema>;

const FALLBACK_ERROR: UserFacingError = {
  code: 'DATABASE_ERROR',
  message: '操作未能安全完成。',
  nextStep: '重启应用后重试；若问题持续，请导出诊断信息。',
  diagnosticId: null,
};

export function toUserError(error: unknown): UserFacingError {
  const parsed = appErrorDtoSchema.safeParse(error);
  return parsed.success ? parsed.data : { ...FALLBACK_ERROR };
}

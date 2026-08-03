# Phase 6 Windows 11 manual evidence

Captured on 2026-08-03 using synthetic TextbookLens fixtures only on Windows 11 Pro 23H2
(build 22631). The supported V1 and release-verification baseline is Windows 11 x64 only; this
evidence makes no Windows 10 compatibility claim.

## Live readings

The existing debug app was the only `textbooklens.exe` process. Its exact nonzero window was process
31572, HWND 2099878. Read-only Win32 probes reported DPI awareness value 2 (per-monitor aware),
awareness context 34, monitor handle 65537, and a successful monitor-scale query (`HRESULT` 0) at
every checkpoint.

| Checkpoint             | Settings UI | `GetDpiForWindow` | Monitor scale | `SPI_GETHIGHCONTRAST` | Flags |
| ---------------------- | ----------: | ----------------: | ------------: | --------------------- | ----: |
| Original baseline      |        150% |               144 |           150 | false                 |   126 |
| Real 200% scaling      |        200% |               192 |           200 | false                 |   126 |
| Aquatic contrast theme |        200% |               192 |           200 | true                  |   127 |
| Contrast restored      |        200% |               192 |           200 | false                 |   126 |
| Final restored state   |        150% |               144 |           150 | false                 |   126 |

The login-era `AppliedDPI` registry value remained 144 while Settings and the live window APIs
reported 200%; it is recorded as stale and was not used as the live acceptance source. No UAC,
sign-out, authentication, install, VM, browser zoom, emulation, or registry write was used.

## Artifacts

- `200-percent-narrow-reader.png` - real-200% narrow reader with all primary toolbar actions retained
  and synthetic DOCX text readable.
- `200-percent-fullscreen-reader.png` - real-200% document fullscreen. This is a target-only crop of
  the left 900 pixels of the 1280 x 800 fullscreen capture, excluding unrelated desktop
  notifications; the missing Windows title bar and fullscreen reader surface are visible.
- `200-percent-escape-focus-return.png` - the narrow reader after Escape exits fullscreen, with the
  focus ring returned to the fullscreen trigger.
- `high-contrast-opaque-reader.png` - real-200% Aquatic contrast theme with an opaque, readable,
  operational reader fallback.

Only the TextbookLens target window and synthetic fixture content appear in the PNGs. Contrast was
restored to off and display scaling was restored to the original 150%, with the final live readings
matching the original baseline.

## Verification contracts

- [GetDpiForWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow)
- [GetWindowDpiAwarenessContext](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdpiawarenesscontext)
- [GetAwarenessFromDpiAwarenessContext](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getawarenessfromdpiawarenesscontext)
- [GetScaleFactorForMonitor](https://learn.microsoft.com/en-us/windows/win32/api/shellscalingapi/nf-shellscalingapi-getscalefactorformonitor)
- [SystemParametersInfo and HIGHCONTRAST](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow)

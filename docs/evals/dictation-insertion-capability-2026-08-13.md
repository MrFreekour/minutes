# Dictation insertion capability matrix — 2026-08-13

Minutes treats clipboard preservation and active-app insertion as different
capabilities. A successful clipboard write never proves that text reached the
focused app.

| Platform/session | Clipboard | Active-app path | Honest success states | Fallback |
|---|---|---|---|---|
| macOS | `pbcopy` / `pbpaste` | Direct focused-control AX insertion when supported; otherwise System Events `Cmd-V` | `typed` only when AX verification observes the text; otherwise `pasted` | `copied` or `blocked`, with text preserved |
| Windows desktop | native `arboard` text clipboard | Not implemented | Never claims `typed` or `pasted` | `blocked` after a verified clipboard copy |
| Linux, pure X11 with `xdotool` | `xclip` or `xsel` | `xdotool key --clearmodifiers ctrl+v` | `pasted`, explicitly unverified | `copied` if injection fails |
| Linux, Wayland (including XWayland present) | `wl-copy` / `wl-paste` | No universal compositor-independent path | Never claims `typed` or `pasted` | `copied`, with manual-paste guidance |
| Linux desktop without matching clipboard tools | unavailable | unavailable | `failed` | Actionable package guidance |
| Headless/other | not claimed | not claimed | Never claims active-app insertion | `failed` if clipboard is unavailable |

The platform-neutral `TextInsertionCapability` classifier is exercised on every
build host so Windows, X11, Wayland, and headless truth does not depend on
running their conditional branches on macOS. Windows also retains a native-only
clipboard round-trip test for its runner.

Cross-app focus and permissions remain separate runtime gates. On macOS the
operation itself is authoritative because a direct Accessibility trust preflight
does not predict whether the System Events child can paste. A target bundle
mismatch blocks automation and keeps the text on the clipboard.

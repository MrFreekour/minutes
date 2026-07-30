# UI screenshot helper (`ui_shot`)

A narrow, purpose-built binary that captures a Minutes window on request, so
automated UI verification does not depend on a human looking at a screen.

## Why this exists

Desktop UI changes are the one class of change CI cannot verify. Type checks and
Rust tests do not catch render bugs, so the pre-commit checklist requires
building the dev app and click-testing it. That routes every UI change through
the maintainer, and when it is skipped, regressions ship: #605 fixed a title
truncation caused by #596 adding a fifth action button, which no automated check
could have caught.

`screencapture` over SSH does not work, because the Screen Recording TCC grant
would have to go to `sshd`. That is the tempting shortcut and it is a bad trade:
it grants continuous screen access to anything that ever runs over SSH, forever,
on a machine that has client financial data and password manager windows on
screen. The capability is the same; the blast radius is enormous.

TCC grants attach to an executable. So the fix is a binary narrow enough that
holding Screen Recording is acceptable.

## The security boundary

**The boundary is the target, not the trigger.** `ui_shot` will only capture
windows owned by an allowlisted Minutes bundle:

- `com.useminutes.desktop`
- `com.useminutes.desktop.dev`
- `com.useminutes.desktop.uitest`

The allowlist is a compile-time literal, not an argument or a prefix match, so
the set of capturable windows cannot be widened by whoever invokes the binary.
Asked for anything else it refuses and writes no file:

```
$ ui_shot com.1password.1password /tmp/out.png
ui_shot: refusing to capture 'com.1password.1password': not in the allowlist.
This helper only captures Minutes windows by design.
$ echo $?
3
```

Even fully compromised, this helper cannot photograph a password manager, a
banking session, or private messages. That is what makes the grant defensible
where granting `sshd` is not.

It also only ever reads: one capture per invocation, no continuous recording, no
persistence, and it writes exactly the output path it is given.

## Setup

One-time, and it needs a GUI session because TCC prompts and the keychain are
not reachable over SSH.

1. Build and install the helper. `cargo build -p minutes-app` compiles it to
   `tauri/src-tauri/bin/ui_shot` as part of the normal build; copy it somewhere
   stable:

   ```bash
   mkdir -p ~/.minutes/bin
   cp tauri/src-tauri/bin/ui_shot ~/.minutes/bin/ui_shot
   chmod 755 ~/.minutes/bin/ui_shot
   ```

2. **Sign it with a stable identity.** This step is what makes the grant
   survive rebuilds, and it must run in a Terminal on the machine (the login
   keychain is not available over SSH):

   ```bash
   codesign --force --options runtime \
     --sign "Developer ID Application: Your Name (TEAMID)" \
     ~/.minutes/bin/ui_shot
   ```

   Without it the binary is ad-hoc signed, which means TCC identifies it by
   content hash and **every rebuild invalidates the grant**. That is the same
   failure mode as a dev app whose Accessibility grant silently stops working
   after a local rebuild: the entry still appears in System Settings, but the
   running binary no longer matches it.

3. Grant Screen Recording. Run it once to trigger the prompt, or add it
   manually: System Settings, Privacy and Security, Screen and System Audio
   Recording, then add `~/.minutes/bin/ui_shot`.

   If you had granted an earlier build and it stopped working, remove the entry
   with the minus button and re-add it. Toggling it off and on re-enables the
   same stale record and does not help.

## Usage

```bash
ui_shot <bundle-id> <output.png> [window-index]
```

Windows are sorted largest first, so index 0 is the main window rather than a
tooltip or status item. Minutes is a menu bar app, so its window must be open;
with no window the helper exits 2 and says so rather than writing an empty file.

| Exit | Meaning |
|---|---|
| 0 | captured |
| 1 | usage error |
| 2 | no capturable window for that bundle |
| 3 | bundle not allowlisted |
| 4 | capture failed, usually a missing Screen Recording grant |

A capture that returns zero pixels is treated as failure rather than written
out, because that is what macOS hands back when the grant is missing but the
call itself succeeds.

## Scope

This is the first phase of the agent-driven QA harness designed in #577. It
provides the capture primitive. The remaining phases, accessibility-tree
snapshots with stable fingerprints and deterministic replay, are what turn
captures into assertions. A screenshot proves a window rendered; a fingerprint
proves it rendered the same as last time.

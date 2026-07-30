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

Run this in a Terminal **on the machine**, not over SSH:

```bash
./scripts/install-ui-shot.sh
```

It builds, installs to `~/.minutes/bin/ui_shot`, signs with the first stable
identity it finds (override with `MINUTES_DEV_SIGNING_IDENTITY`), and then tells
you whether the Screen Recording grant is already held.

Two constraints make this a script rather than a copy-paste:

- **`codesign` needs the login keychain**, which an SSH session cannot reach; it
  fails with `errSecInternalComponent`. So this step cannot be automated
  remotely, which also means an agent cannot silently re-grant itself capture
  access.
- **Signing must happen on every rebuild.** Without a stable identity the binary
  is ad-hoc signed, TCC identifies it by content hash, and each rebuild
  invalidates the grant. That is the same failure mode as a locally rebuilt dev
  app losing Accessibility: the entry stays listed in System Settings while the
  running binary no longer matches it.

Then grant Screen Recording if the script says it is missing: System Settings,
Privacy and Security, Screen and System Audio Recording, add
`~/.minutes/bin/ui_shot`.

If an entry is already listed but `ui_shot --check` still reports not granted,
remove it with the minus button and re-add it. Toggling it off and on re-enables
the same stale record.

Verify any time with:

```bash
~/.minutes/bin/ui_shot --check
```

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

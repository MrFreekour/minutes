# Minutes Archive 0.2.0

This is the first DMG release of the private Minutes Archive pilot.

- Adds a visible, one-time update check when the app opens, before any folder
  is approved.
- Downloads nothing unless the user chooses **Install update**.
- Verifies every in-app update against the Minutes signing key before replacing
  the installed app.
- Closes the update network window before the folder picker opens and keeps all
  archive reading, indexing, and search local to the Mac.
- Keeps a signed, notarized DMG available as the manual recovery path.

The Archive pilot remains separate from the normal Minutes desktop release
channel. Its updater reads only the dedicated `archive-stable` manifest.

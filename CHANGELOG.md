# Changelog

## 0.1.0 – 2026-11-08

First release.

- Tray icon with a panel listing all saved Claude Desktop accounts (name, e-mail, 5h and weekly limits with reset times); one click switches.
- `switch.ps1`: swaps only the login data of Claude Desktop, refuses when an unexpected account is signed in, logs to `switch.log`.
- Claude Code mod: records each account's rate limits, band above the prompt, `/accounts` panel.
- `install.ps1`: icon from the installed Claude logo, Autostart and Start menu entry.

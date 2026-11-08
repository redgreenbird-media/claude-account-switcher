# Changelog

## 0.2.1 – 2026-11-08

- **Fixed**: a switch closed Claude but did not start it again, and the app was gone afterwards. Started from a Claude Code session (or an installer run from one), the app ran inside Claude's job object and Windows ended it together with Claude, before the login was swapped. The app now leaves such a job at start.
- **Fixed**: Claude hung on a white window for a while after a switch. Closing its window only hides it in the tray, so after five seconds it was ended outright and had to recover on the next start. It is now asked to quit the way Windows does before an update (Restart Manager), saves and ends its sessions; only what is still running after 30 seconds is ended.

## 0.2.0 – 2026-11-08

A real app instead of PowerShell scripts.

- **Claude Accounts app** (Tauri): tray icon and panel, starts with Windows, installs per user, published through the update server so Roost installs and updates it. Replaces `scripts/tray.ps1`, `switch.ps1`, `install.ps1` and `make-icon.ps1`.
- **Limits for every account**: the app also reads Claude Desktop's own usage samples (`plan-usage-history.json`), so accounts without a recent Code session show their figures too. Without a reset time the panel shows the latest possible reset. Before, accounts other than the last active one showed nothing.
- **Fixed**: the mod wrote all accounts into one `accounts.json` from a per-account store, so each account overwrote the others' limits. It now writes `limits\<uuid>.json` per account.
- **Fixed**: the mod's band and panel stayed off because it derived Claude's data folder from `CLAUDE_CODE_EXECPATH`, which mods do not see that way. The app finds the folder itself; the mod no longer needs it.
- **Terminal per account**: Claude Code with that account's own login in Windows Terminal, in parallel to Desktop (`~/.claude-accounts/<uuid>`). A second Claude Desktop instance is not possible: signed builds ignore `CLAUDE_USER_DATA_DIR`.
- Own icon instead of Claude's logo.
- Mod switches through the app (`claude-accounts.exe --switch <uuid>` / `--add`); without the app it shows the limits only.
## 0.1.0 – 2026-11-08

First release.

- Tray icon with a panel listing all saved Claude Desktop accounts (name, e-mail, 5h and weekly limits with reset times); one click switches.
- `switch.ps1`: swaps only the login data of Claude Desktop, refuses when an unexpected account is signed in, logs to `switch.log`.
- Claude Code mod: records each account's rate limits, band above the prompt, `/accounts` panel.
- `install.ps1`: icon from the installed Claude logo, Autostart and Start menu entry.

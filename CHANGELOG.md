# Changelog

## 0.3.0-beta.2 – 2026-11-26

- **Daily pace on the weekly bar**: a mark shows how much of the weekly limit may be used by the end of today, one seventh per day of the window (day 1: 14 %, day 2: 29 %, …), so that the limit lasts until the reset. *Soll heute* under the bar gives the figure; the mark turns red once usage is past it.
- **Click the account to show its Claude Desktop**: the whole card brings the account's Desktop to the front, not only the *Fenster*/*Zeigen* button. The active account shows the main Desktop, an account with a window of its own shows that one, any other opens a window from its saved login.
- **Releases build on GitHub again**: this repository is public, and our self-hosted runners never take jobs of public repositories, so no release since the move to the VM got built (0.2.x stayed unpublished at the publish step too). The release workflow now builds on a GitHub-hosted Windows runner and ends with a GitHub release; the update server picks it up within ten minutes (`claude-accounts.yml` there).

## 0.3.0-beta.1 – 2026-11-26

- **Claude Desktop per account in its own window**: *Fenster* opens an account in a further Claude Desktop next to the main one, with its own data folder (`desktop\<uuid>`, started with `--user-data-dir`, which Claude keeps). The saved login goes in before the first start, so there is no sign-in. *Wechseln* to an account whose window is open closes that window first and takes its login along. An account signed in to the main Desktop gets no window. Before, a second Desktop was thought impossible: Claude ignores `CLAUDE_USER_DATA_DIR`, but not the `--user-data-dir` switch.
- **Live limits for every account**: the app asks Anthropic's usage endpoint (`/api/oauth/usage`, the one Claude Desktop asks itself) every minute and when the panel opens, with each account's own token from the logins Claude keeps (main Desktop, own windows, saved profiles). Exact reset times, also for accounts not in use. Read only: tokens are decrypted in memory (DPAPI-wrapped key in `Local State`), never written, refreshed or sent anywhere but `*.anthropic.com`; an expired one is skipped until Claude renews it. Before, figures changed only after a reply or when Desktop sampled them.
- **Switching only touches the main Desktop**: Claude's processes are told apart by their data folder, so closing Claude for a switch leaves the other windows running.
- **Limits of accounts in their own window** come from that window's own usage samples, without the org guesswork.
- **Reset times without a live reading**: *Reset spätestens* is now one window length after the window was first seen in Desktop's samples (a drop, a 0 % or a gap ends a window), not after the latest sample, which pushed the time forward with every sample.
- Mod: in an account's own window no switch buttons (switching changes the main Desktop); shows *Reset spätestens* where the app has no exact time.
- The explanation under the account list is gone.

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

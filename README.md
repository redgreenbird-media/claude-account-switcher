# Claude Account Switcher

Switch Claude Desktop (Windows) between several accounts from a tray icon, and see each account's 5-hour and weekly usage limits.

> Unofficial. Not affiliated with or endorsed by Anthropic. It swaps Claude Desktop's local login files; use it at your own risk.

The UI is in German.

## What it does

- **Tray icon** (the Claude logo with a switch badge): click it to open a panel listing every saved account, with name, e-mail and its last known limits plus reset times. Click another account to switch. *Account hinzufügen* signs out so another account can sign in.
- **Switching** closes Claude Desktop, saves the current login, restores the other one and starts Claude again. Only the login is swapped: session cookies, the web storage of the claude.ai views and two keys of `config.json`. Sessions and settings are kept. The script stops without changing anything if a different account than expected is signed in, and logs to `%LOCALAPPDATA%\ClaudeAccountSwitcher\switch.log`.
- **Claude Code mod** (optional): in the Code tab it records each account's rate limits after every reply, which the tray panel shows. It also adds a band above the prompt and an `/accounts` panel.

Saved logins stay on your machine in `%LOCALAPPDATA%\ClaudeAccountSwitcher\profiles`, encrypted the same way Claude stores them (bound to your Windows user).

## Install

Requirements: Windows 10/11, Claude Desktop from the Microsoft Store / MSIX installer, Windows PowerShell 5.1.

1. Clone the repository somewhere permanent:

   ```bash
   git clone https://github.com/redgreenbird-media/claude-account-switcher.git
   ```

2. Install the tray icon. This builds the icon, adds *Claude Accounts* to Autostart and the Start menu, and starts it:

   ```bash
   powershell -ExecutionPolicy Bypass -File claude-account-switcher\scripts\install.ps1
   ```

3. Optional, for the limits: load the folder as a Claude Code mod by adding it to `~/.claude/settings.json`:

   ```json
   "env": { "CLAUDE_CODE_PLUGIN_DIRS": "C:\\path\\to\\claude-account-switcher" }
   ```

   Or install it as a plugin from a terminal:

   ```bash
   claude plugin marketplace add redgreenbird-media/claude-account-switcher
   claude plugin install account-switcher@claude-account-switcher
   ```

## First use

An account becomes selectable once you have switched away from it once. Start with *Account hinzufügen*: Claude restarts on its sign-in page, sign in with the second account, and both accounts show in the panel from then on.

Limits of inactive accounts are the values from when they were last active. A window whose reset time has passed shows 0 %.

## Files

| File | Purpose |
| --- | --- |
| `scripts/tray.ps1` | Tray icon and panel (WPF). `-DryRun` prints the accounts, `-Snapshot x.png` renders the panel. |
| `scripts/switch.ps1` | Closes Claude, swaps the login, starts Claude. |
| `scripts/make-icon.ps1` | Builds the tray icon from the installed Claude logo. |
| `scripts/install.ps1` | Icon, Autostart and Start menu entry. Safe to run again. |
| `hooks/register.tsx` | The Claude Code mod. |

## Uninstall

Right-click the tray icon, *Beenden*. Delete *Claude Accounts* from `shell:startup` and the Start menu, and `%LOCALAPPDATA%\ClaudeAccountSwitcher`.

# Claude Accounts

Switch Claude Desktop (Windows) between several accounts from the tray, open further accounts in Claude Desktop windows of their own, see every account's 5-hour and weekly limit, and run Claude Code with another account in parallel.

> Unofficial. Not affiliated with or endorsed by Anthropic. It swaps Claude Desktop's local login files; use it at your own risk.

The UI is in German.

## Install

In [Roost](https://updates.redgreenbird.com/get): **Claude Accounts** (the app) and, optionally, **Account Switcher (Claude Code)** (the mod). Roost keeps both up to date.

Without Roost: run the setup from the [latest release](https://github.com/redgreenbird-media/claude-account-switcher/releases). It installs for the current user, no admin rights needed.

The app starts with Windows and sits in the tray (right-click › *Mit Windows starten* turns that off).

## What it does

- **Tray panel** (click the icon): every saved account with name, e-mail, 5-hour and weekly limit, reset times and when the figures were taken. *Wechseln* switches, *Account hinzufügen* signs out so another account can sign in.
- **Switching** closes Claude Desktop, saves the current login, puts the other one back and starts Claude again. Only the login moves: the session cookies, the web storage of the claude.ai views and two keys of `config.json`. Sessions and settings stay. If a different account than expected is signed in, nothing changes. Log: `%LOCALAPPDATA%\ClaudeAccountSwitcher\switch.log`.
- **Own window** (*Fenster*, per account): a further Claude Desktop with that account, next to the main one. Each account gets its own data folder `desktop\<uuid>`, started with `--user-data-dir`; Claude keeps such a folder and holds its single-instance lock per folder. The saved login is put in before the first start, so there is no sign-in. *Wechseln* to an account whose window is open closes that window first and takes its login along; the main Desktop's account gets no window. A window downloads its own copy of the Code engine on first use.
- **Limits** come from three places and the newest one wins:
  - Live from Anthropic, every minute and when the panel opens: the usage endpoint Claude Desktop asks itself (`/api/oauth/usage`), with each account's own token from the logins Claude keeps (main Desktop, own windows, saved profiles). Exact percentages and reset times, for accounts not in use too. Read only: the tokens are decrypted in memory with the DPAPI-wrapped key in Claude's `Local State`, never written or refreshed, and sent only to `*.anthropic.com`. An expired token is skipped until Claude renews it; the account then shows its last reading.
  - Claude Desktop's own usage samples (`plan-usage-history.json`, the figures of *Settings › Usage*), of the main Desktop and of each account's own window. They carry no reset time: the panel shows *Reset spätestens …*, one window length after the window was first seen in the samples, and a window past that reads 0 %.
  - The mod (below), after every reply in a Code session: the windows with their exact reset times.
- **Terminal** (per account): opens Windows Terminal in a folder you pick and starts Claude Code with that account's own login, next to Desktop and the other accounts. Each account gets its own config folder `~/.claude-accounts/<uuid>`; sign in there once with `/login`. Skills, agents, commands, plugins, hooks and projects (sessions and memory) are those of `~/.claude`; `settings.json`, the `*.md` files and the MCP servers are copied over at every start. A ✓ on the button means the terminal login matches the account.

### How the windows work

Claude Desktop allows one instance per data folder. Signed builds ignore `CLAUDE_USER_DATA_DIR`, but Claude honours the `--user-data-dir` switch and keeps such a folder as a "chosen folder". The app starts the packaged app with that switch through Windows' activation manager (as the Start menu does, with a command line). Its processes are told apart by their folder, so a switch closes only the main Desktop.

## The mod (optional)

The Claude Code mod in this repository records each account's rate limits after every reply (`%LOCALAPPDATA%\ClaudeAccountSwitcher\limits\<uuid>.json`), shows the account and its limits in a band above the prompt with a button per other account, and adds an `/accounts` panel. Switching from there asks the app to do it.

From a clone instead of Roost, add the folder to `~/.claude/settings.json`:

```json
"env": { "CLAUDE_CODE_PLUGIN_DIRS": "C:\\path\\to\\claude-account-switcher" }
```

## First use

An account becomes switchable once you have switched away from it once. Start with *Account hinzufügen*: Claude restarts on its sign-in page, sign in with the second account, and both show in the panel from then on.

## Files

Everything the app keeps is in `%LOCALAPPDATA%\ClaudeAccountSwitcher`:

| File | What |
| --- | --- |
| `profiles\<uuid>\` | saved logins, encrypted the way Claude stores them (bound to your Windows user) |
| `desktop\<uuid>\` | data folder of the account's own Claude Desktop window |
| `limits\<uuid>.json` | the newest live or mod reading |
| `names.json` | names and e-mails read from Claude's cached account data |
| `view.json` | what the panel shows, for the mod |
| `app-settings.json` | autostart, last terminal folder |
| `switch.log` | every switch |

## Development

```bash
npm install
npx tauri dev
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

```bash
claude plugin test .
```

| Path | What |
| --- | --- |
| `src-tauri/src/claude.rs` | Claude Desktop's data folder, signed-in account, closing and starting it |
| `src-tauri/src/profiles.rs` | saving and restoring logins |
| `src-tauri/src/instances.rs` | an account's own Desktop window: its folder, login and start |
| `src-tauri/src/usage.rs` | merging Desktop's samples and the readings in `limits\` |
| `src-tauri/src/live.rs` | live limits from Anthropic's usage endpoint |
| `src-tauri/src/names.rs` | names, e-mails and organizations from Claude's IndexedDB blobs |
| `src-tauri/src/parallel.rs` | per-account Claude Code config folders and the terminal |
| `src/` | the panel |
| `hooks/register.tsx` | the Claude Code mod |
| `assets/make-icon.ps1` | draws `assets/icon.png`; `npx tauri icon assets/icon.png -o src-tauri/icons` makes the sizes |

Releases: bump `VERSION`, `src-tauri/Cargo.toml` and `.claude-plugin/plugin.json`, then tag `vX.Y.Z` (see the update server's `docs/RELEASING.md`).

## Uninstall

Windows Settings › Apps › *Claude Accounts*. Delete `%LOCALAPPDATA%\ClaudeAccountSwitcher` (saved logins) and `~/.claude-accounts` (terminal logins) if you want them gone too.
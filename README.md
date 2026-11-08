# Claude Accounts

Switch Claude Desktop (Windows) between several accounts from the tray, see every account's 5-hour and weekly limit, and run Claude Code with another account in parallel.

> Unofficial. Not affiliated with or endorsed by Anthropic. It swaps Claude Desktop's local login files; use it at your own risk.

The UI is in German.

## Install

In [Roost](https://updates.redgreenbird.com/get): **Claude Accounts** (the app) and, optionally, **Account Switcher (Claude Code)** (the mod). Roost keeps both up to date.

Without Roost: run the setup from the [latest release](https://github.com/redgreenbird-media/claude-account-switcher/releases). It installs for the current user, no admin rights needed.

The app starts with Windows and sits in the tray (right-click › *Mit Windows starten* turns that off).

## What it does

- **Tray panel** (click the icon): every saved account with name, e-mail, 5-hour and weekly limit, reset times and when the figures were taken. *Wechseln* switches, *Account hinzufügen* signs out so another account can sign in.
- **Switching** closes Claude Desktop, saves the current login, puts the other one back and starts Claude again. Only the login moves: the session cookies, the web storage of the claude.ai views and two keys of `config.json`. Sessions and settings stay. If a different account than expected is signed in, nothing changes. Log: `%LOCALAPPDATA%\ClaudeAccountSwitcher\switch.log`.
- **Limits** come from two places and the newer one wins:
  - Claude Desktop's own usage samples (`plan-usage-history.json`, the figures of *Settings › Usage*), taken every few minutes while Desktop runs with an account. These cover inactive accounts too, from when they were last active, but carry no reset time; the panel then shows *Reset spätestens …* (the sample time plus the window length), and a window older than its length reads 0 %.
  - The mod (below), after every reply in a Code session: the windows with their exact reset times.
- **Terminal** (per account): opens Windows Terminal in a folder you pick and starts Claude Code with that account's own login, next to Desktop and the other accounts. Each account gets its own config folder `~/.claude-accounts/<uuid>`; sign in there once with `/login`. Skills, agents, commands, plugins, hooks and projects (sessions and memory) are those of `~/.claude`; `settings.json`, the `*.md` files and the MCP servers are copied over at every start. A ✓ on the button means the terminal login matches the account.

### Why not two Claude Desktop windows?

Claude Desktop allows one instance per data folder. Its only switch for another data folder, `CLAUDE_USER_DATA_DIR`, is removed on start by signed builds ("signed builds ignore it"), so a second Desktop with another account is not possible without modifying the app. Parallel work therefore runs in the terminal.

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
| `limits\<uuid>.json` | limits the mod recorded |
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
| `src-tauri/src/usage.rs` | merging Desktop's samples and the mod's readings |
| `src-tauri/src/names.rs` | names, e-mails and organizations from Claude's IndexedDB blobs |
| `src-tauri/src/parallel.rs` | per-account Claude Code config folders and the terminal |
| `src/` | the panel |
| `hooks/register.tsx` | the Claude Code mod |
| `assets/make-icon.ps1` | draws `assets/icon.png`; `npx tauri icon assets/icon.png -o src-tauri/icons` makes the sizes |

Releases: bump `VERSION`, `src-tauri/Cargo.toml` and `.claude-plugin/plugin.json`, then tag `vX.Y.Z` (see the update server's `docs/RELEASING.md`).

## Uninstall

Windows Settings › Apps › *Claude Accounts*. Delete `%LOCALAPPDATA%\ClaudeAccountSwitcher` (saved logins) and `~/.claude-accounts` (terminal logins) if you want them gone too.
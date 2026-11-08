import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Account, Limit, View, Window } from '../types'

const PANE = 'account-switcher'
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

const view = atom(
  { plugin: 'account-switcher', key: 'view' } as const,
  { isDesktop: false, accounts: [], now: 0 } as View,
  { shape: 'view-2' },
)

type Dollar = EngineInterface

/** Where this session runs and who it is signed in as. */
type Context = {
  isDesktop: boolean
  /** %LOCALAPPDATA%\ClaudeAccountSwitcher, shared with the Claude Accounts app. */
  root: string
  current?: { id: string; email: string; orgId?: string }
  /** This account's windows as the last reply reported them. */
  live: Window[]
  liveAt?: number
}

// ---------- formatting ----------

const DAYS = ['So', 'Mo', 'Di', 'Mi', 'Do', 'Fr', 'Sa']
const pad = (n: number) => String(n).padStart(2, '0')

function clockTime(at: number, now: number): string {
  const d = new Date(at)
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  if (d.toDateString() === new Date(now).toDateString()) return `heute ${hm}`
  if (d.toDateString() === new Date(now + 86_400_000).toDateString()) return `morgen ${hm}`

  return `${DAYS[d.getDay()]} ${d.getDate()}.${d.getMonth() + 1}. ${hm}`
}

function span(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000))
  if (minutes < 60) return `${minutes} min`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} h ${minutes % 60} min`

  return `${Math.floor(hours / 24)} T ${hours % 24} h`
}

function bar(percent: number): string {
  const filled = Math.round(Math.min(100, Math.max(0, percent)) / 10)

  return '█'.repeat(filled) + '░'.repeat(10 - filled)
}

const LABELS: Record<string, string> = { five_hour: '5h', seven_day: 'Woche' }
const KINDS = ['five_hour', 'seven_day']

/** A window's figures as of `now`: a window whose reset passed reads 0 %. */
function figures(w: Window | undefined, kind: string, now: number) {
  const label = LABELS[kind] ?? kind
  if (!w || w.percent === null || w.percent === undefined) return { label, percent: NaN, reset: '' }
  const isReset = w.isReset || (w.resetsAt !== undefined && w.resetsAt !== null && w.resetsAt <= now)

  return {
    label,
    percent: isReset ? 0 : Math.round(w.percent),
    reset: isReset
      ? 'zurückgesetzt'
      : w.resetsAt
        ? `Reset in ${span(w.resetsAt - now)} (${clockTime(w.resetsAt, now)})`
        : 'Reset unbekannt',
  }
}

function shortName(email: string): string {
  return email.split('@')[0] || email
}

// ---------- data ----------

function toWindows(limits: readonly Limit[], at: number): Window[] {
  return limits
    .filter(l => KINDS.includes(l.kind))
    .map(l => {
      const resetsAt = l.resetsAt ? Date.parse(l.resetsAt) : NaN

      return {
        kind: l.kind,
        percent: l.percentUsed,
        resetsAt: Number.isFinite(resetsAt) ? resetsAt : undefined,
        isReset: false,
        at,
      }
    })
}

function parseAccounts(raw: unknown): Account[] {
  if (!Array.isArray(raw)) return []

  return raw.flatMap(a => {
    const v = a as Partial<Account> | null
    if (!v || typeof v.id !== 'string' || !UUID.test(v.id) || typeof v.email !== 'string') return []

    return [
      {
        id: v.id,
        email: v.email,
        name: typeof v.name === 'string' ? v.name : '',
        hasProfile: v.hasProfile === true,
        windows: Array.isArray(v.windows) ? v.windows.filter(w => w && typeof w.kind === 'string') : [],
        measuredAt: typeof v.measuredAt === 'number' ? v.measuredAt : undefined,
      },
    ]
  })
}

/** What the Claude Accounts app last published: its exe and the accounts it knows. */
async function readApp($: Dollar, ctx: Context): Promise<{ app?: string; accounts: Account[] }> {
  try {
    const raw = JSON.parse(await $.fs.read(`${ctx.root}\\view.json`)) as { app?: unknown; accounts?: unknown }

    return {
      app: typeof raw.app === 'string' && raw.app.toLowerCase().endsWith('.exe') ? raw.app : undefined,
      accounts: parseAccounts(raw.accounts),
    }
  } catch {
    return { accounts: [] }
  }
}

async function refresh($: Dollar, ctx: Context, notice?: string) {
  const { app, accounts } = await readApp($, ctx)
  const now = await $.clock.now()
  const me = ctx.current
  if (me) {
    const known = accounts.find(a => a.id === me.id)
    const mine: Account = known ?? { id: me.id, email: me.email, name: '', hasProfile: false, windows: [] }
    // This session's own reading is the freshest for the signed-in account.
    if (ctx.live.length > 0 && (mine.measuredAt ?? 0) <= (ctx.liveAt ?? 0)) {
      mine.windows = ctx.live
      mine.measuredAt = ctx.liveAt
    }
    if (!known) accounts.unshift(mine)
  }
  accounts.sort((a, b) => (a.id === me?.id ? -1 : b.id === me?.id ? 1 : a.email.localeCompare(b.email)))
  await update($, view, () => ({ isDesktop: ctx.isDesktop, app, currentId: me?.id, accounts, now, notice }))
}

/** Writes this account's windows to limits\<uuid>.json, which the app merges into its view. */
async function recordLimits($: Dollar, ctx: Context, limits: readonly Limit[]) {
  if (!ctx.current) return
  const now = await $.clock.now()
  const windows = limits.filter(l => KINDS.includes(l.kind))
  if (windows.length === 0) return
  ctx.live = toWindows(windows, now)
  ctx.liveAt = now
  try {
    await $.fs.write(
      `${ctx.root}\\limits\\${ctx.current.id}.json`,
      JSON.stringify({
        id: ctx.current.id,
        email: ctx.current.email,
        orgId: ctx.current.orgId,
        limits: windows.map(l => ({ kind: l.kind, percentUsed: l.percentUsed, resetsAt: l.resetsAt })),
        measuredAt: now,
      }),
    )
  } catch {
    // The app then shows Desktop's own figures; nothing here blocks the session.
  }
}

/**
 * Asks the Claude Accounts app to switch. Started through WMI, outside this
 * session's process tree, so it outlives Claude closing during the switch.
 */
async function launch($: Dollar, app: string, args: string[]) {
  const line = [`"${app}"`, ...args].join(' ')
  const quoted = `'${line.replaceAll("'", "''")}'`
  const command = [
    `$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = ${quoted} }`,
    'exit $r.ReturnValue',
  ].join('; ')
  const ran = await $.process.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', command], {
    timeoutMs: 30_000,
  })
  if (ran.exitCode !== 0) {
    throw new Error(ran.stderr.trim() || `Win32_Process.Create returned ${ran.exitCode}`)
  }
}

// ---------- module ----------

// Set at session.start (a reload runs it again), read by the press handlers.
let ctx: Context = { isDesktop: false, root: '', live: [] }

async function start($: Dollar, to?: Account) {
  const v = await read($, view)
  if (!ctx.isDesktop) return
  if (!v.app) {
    await refresh($, ctx, 'Die App „Claude Accounts" ist nicht installiert – in Roost installieren.')

    return
  }
  const what = to ? `Wechsle zu ${to.email}` : 'Melde ab für neuen Account'
  await refresh($, ctx, `${what} – Claude startet neu …`)
  $.ui.toast(`${what} – Claude startet neu …`)
  try {
    await launch($, v.app, to ? ['--switch', to.id] : ['--add'])
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    await refresh($, ctx, `Wechsel fehlgeschlagen: ${message}`)
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    const entry = await $.env.get('CLAUDE_CODE_ENTRYPOINT')
    const local = (await $.env.get('LOCALAPPDATA')) ?? ''
    const id = (await $.env.get('CLAUDE_CODE_ACCOUNT_UUID')) ?? ''
    const email = (await $.env.get('CLAUDE_CODE_USER_EMAIL')) ?? ''
    const orgId = await $.env.get('CLAUDE_CODE_ORGANIZATION_UUID')

    ctx = {
      isDesktop: entry === 'claude-desktop' && local !== '',
      root: `${local}\\ClaudeAccountSwitcher`,
      current: local !== '' && UUID.test(id) && email ? { id, email, orgId } : undefined,
      live: [],
    }

    await $.command.register({
      name: 'accounts',
      description: 'Claude-Desktop-Accounts mit 5h-/Wochenlimit anzeigen und wechseln',
    })

    if (ctx.current) {
      try {
        await recordLimits($, ctx, (await $.session.usage()).rateLimits)
      } catch {
        // No reading before the first reply.
      }
    }
    if (ctx.isDesktop) {
      $.clock.every(60_000, () => {
        void refresh($, ctx)
      })
    }
    await refresh($, ctx)

    return started
  })

  on('session.measure', async ($, e, next) => {
    if (ctx.current && e.changed.includes('rateLimits') && e.rateLimits.length > 0) {
      await recordLimits($, ctx, e.rateLimits)
      await refresh($, ctx)
    }

    return next(e)
  })

  on('command.run', { command: 'accounts' }, async $ => {
    if (!ctx.isDesktop) {
      return { text: 'Der Account-Wechsel funktioniert nur in Claude Desktop.' }
    }
    await $.ui.open({ id: PANE, title: 'Accounts' })

    return { text: 'Accounts geöffnet.' }
  })

  // Compact band above the prompt: active account, its limits, one button per
  // other saved account.
  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const v = await read($, view)
    const me = v.accounts.find(a => a.id === v.currentId)
    if (!v.isDesktop || e.props.hasSurvey || !me) return next(e)

    const { Box, Button, Text } = $.ui.resolve(e)
    const line = KINDS.map(k => figures(me.windows.find(w => w.kind === k), k, v.now))
      .map(l => `${l.label} ${Number.isNaN(l.percent) ? '–' : `${l.percent}%`}`)
      .join(' · ')
    const others = v.app ? v.accounts.filter(a => a.id !== me.id && a.hasProfile) : []

    return (
      <Box flexDirection="row" flexWrap="wrap" columnGap={1} alignItems="center">
        <Text>
          {shortName(me.email)} <Text dimColor>{line}</Text>
        </Text>
        {v.notice ? (
          <Text color="warning">{v.notice}</Text>
        ) : (
          others.map(a => (
            <Button key={`band-${a.id}`} onPress={() => start($, a)}>
              ⇄ {shortName(a.email)}
            </Button>
          ))
        )}
        <Button key="band-open" onPress={() => $.ui.open({ id: PANE, title: 'Accounts' })}>
          Accounts
        </Button>
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Button, Text } = $.ui.resolve(e)
    const v = await read($, view)

    if (!v.isDesktop) {
      return <Text dimColor>Der Account-Wechsel funktioniert nur in Claude Desktop.</Text>
    }

    return (
      <Box flexDirection="column" rowGap={1}>
        {v.notice && <Text color="warning">{v.notice}</Text>}
        {!v.app && (
          <Text dimColor>
            Für den Wechsel braucht es die App „Claude Accounts" (in Roost). Ohne sie zeigt das Panel nur die Limits.
          </Text>
        )}
        {v.accounts.map(a => {
          const isCurrent = a.id === v.currentId
          const age =
            a.measuredAt === undefined
              ? 'noch nicht gemessen'
              : v.now - a.measuredAt < 120_000
                ? 'Stand: gerade eben'
                : `Stand: vor ${span(v.now - a.measuredAt)}`

          return (
            <Box key={`acc-${a.id}`} flexDirection="column">
              <Text bold>
                {isCurrent ? '● ' : '○ '}
                {a.email}
                {isCurrent ? <Text color="success"> aktiv</Text> : ''}
              </Text>
              {KINDS.map(kind => {
                const l = figures(a.windows.find(w => w.kind === kind), kind, v.now)
                if (Number.isNaN(l.percent)) {
                  return <Text dimColor>{`  ${l.label.padEnd(6)} keine Daten`}</Text>
                }

                return (
                  <Text>
                    {`  ${l.label.padEnd(6)} ${bar(l.percent)} ${String(l.percent).padStart(3)}%  `}
                    <Text dimColor>{l.reset}</Text>
                  </Text>
                )
              })}
              <Text dimColor>{`  ${age}`}</Text>
              {!isCurrent &&
                v.app &&
                (a.hasProfile ? (
                  <Button key={`switch-${a.id}`} variant="primary" onPress={() => start($, a)}>
                    Zu diesem Account wechseln
                  </Button>
                ) : (
                  <Text dimColor>
                    {'  Kein gespeicherter Login – einmal normal anmelden, dann hier wegwechseln.'}
                  </Text>
                ))}
            </Box>
          )
        })}
        {v.app && (
          <Button key="add" onPress={() => start($)}>
            + Account hinzufügen
          </Button>
        )}
        <Text dimColor>
          Wechseln schliesst Claude kurz und startet es mit dem anderen Login neu. Laufende Sessions
          bleiben erhalten. Limits anderer Accounts zeigen den letzten Stand.
        </Text>
      </Box>
    )
  })
}
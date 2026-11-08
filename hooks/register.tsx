import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Account, Limit, View } from '../types'

const PANE = 'account-switcher'
const STORE_KEY = 'accounts'
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

const view = atom(
  { plugin: 'account-switcher', key: 'view' } as const,
  { isDesktop: false, accounts: [], now: 0 } as View,
  { shape: 'view-1' },
)

type Stored = Omit<Account, 'hasProfile'>

/** Where this session runs. */
type Context = {
  isDesktop: boolean
  userData: string
  profiles: string
  script: string
  current?: { id: string; email: string; orgId?: string }
}

type Dollar = EngineInterface

// ---------- formatting ----------

const DAYS = ['So', 'Mo', 'Di', 'Mi', 'Do', 'Fr', 'Sa']
const pad = (n: number) => String(n).padStart(2, '0')

function clockTime(at: number, now: number): string {
  const d = new Date(at)
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  const today = new Date(now)
  const tomorrow = new Date(now + 86_400_000)
  if (d.toDateString() === today.toDateString()) return `heute ${hm}`
  if (d.toDateString() === tomorrow.toDateString()) return `morgen ${hm}`

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

/** A window's figures as of `now`: a window whose reset passed reads 0 %. */
function current(limit: Limit, now: number) {
  const resetAt = limit.resetsAt ? Date.parse(limit.resetsAt) : NaN
  const isReset = Number.isFinite(resetAt) && resetAt <= now

  return {
    label: LABELS[limit.kind] ?? limit.kind,
    percent: isReset ? 0 : Math.round(limit.percentUsed),
    reset: isReset
      ? 'zurückgesetzt'
      : Number.isFinite(resetAt)
        ? `Reset in ${span(resetAt - now)} (${clockTime(resetAt, now)})`
        : 'Reset unbekannt',
  }
}

function windows(account: Account): Limit[] {
  return ['five_hour', 'seven_day'].map(
    kind =>
      account.limits.find(l => l.kind === kind) ?? { kind, percentUsed: NaN },
  )
}

function shortName(email: string): string {
  return email.split('@')[0] || email
}

// ---------- data ----------

function parseStored(raw: unknown): Record<string, Stored> {
  const out: Record<string, Stored> = {}
  if (!raw || typeof raw !== 'object') return out
  for (const [id, value] of Object.entries(raw as Record<string, unknown>)) {
    const v = value as Partial<Stored> | null
    if (!UUID.test(id) || !v || typeof v.email !== 'string') continue
    out[id] = {
      id,
      email: v.email,
      orgId: typeof v.orgId === 'string' ? v.orgId : undefined,
      limits: Array.isArray(v.limits)
        ? v.limits.filter(
            (l): l is Limit =>
              !!l && typeof l.kind === 'string' && typeof l.percentUsed === 'number',
          )
        : [],
      measuredAt: typeof v.measuredAt === 'number' ? v.measuredAt : undefined,
    }
  }

  return out
}

async function loadStored($: Dollar) {
  return parseStored(await $.store.get(STORE_KEY))
}

async function hasProfile($: Dollar, ctx: Context, id: string) {
  try {
    return await $.fs.exists(`${ctx.profiles}\\${id}\\auth.json`)
  } catch {
    return false
  }
}

async function refresh($: Dollar, ctx: Context, notice?: string) {
  const stored = await loadStored($)
  const now = await $.clock.now()
  const accounts: Account[] = []
  for (const s of Object.values(stored)) {
    accounts.push({ ...s, hasProfile: await hasProfile($, ctx, s.id) })
  }
  accounts.sort((a, b) =>
    a.id === ctx.current?.id ? -1 : b.id === ctx.current?.id ? 1 : a.email.localeCompare(b.email),
  )
  await update($, view, () => ({
    isDesktop: ctx.isDesktop,
    currentId: ctx.current?.id,
    accounts,
    now,
    notice,
  }))
}

async function recordLimits($: Dollar, ctx: Context, limits: readonly Limit[]) {
  if (!ctx.current) return
  const windowsOnly = limits
    .filter(l => l.kind === 'five_hour' || l.kind === 'seven_day')
    .map(l => ({ kind: l.kind, percentUsed: l.percentUsed, resetsAt: l.resetsAt }))
  const stored = await loadStored($)
  const me = stored[ctx.current.id] ?? { ...ctx.current, limits: [] }
  stored[ctx.current.id] = {
    ...me,
    email: ctx.current.email,
    orgId: ctx.current.orgId,
    ...(windowsOnly.length > 0
      ? { limits: windowsOnly, measuredAt: await $.clock.now() }
      : {}),
  }
  await $.store.set(STORE_KEY, stored)
  await exportAccounts($, ctx, stored)
}

/** Last time the mod ran for an account, and the last error, for the tray app. */
async function writeStatus($: Dollar, ctx: Context, error?: string) {
  try {
    await $.fs.write(
      `${ctx.profiles}\\..\\mod-status.json`,
      JSON.stringify({ at: await $.clock.now(), accountId: ctx.current?.id, error }),
    )
  } catch {
    // Diagnostics only.
  }
}

/**
 * Mirrors the known accounts (names, limits) and the app's data folder into
 * accounts.json, which the taskbar picker (scripts/picker.ps1) reads outside Claude.
 */
async function exportAccounts($: Dollar, ctx: Context, stored: Record<string, Stored>) {
  try {
    await $.fs.write(
      `${ctx.profiles}\\..\\accounts.json`,
      JSON.stringify({ userData: ctx.userData, accounts: stored }, null, 2),
    )
  } catch {
    // The picker falls back to short ids; nothing here blocks the session.
  }
}

/** Starts switch.ps1 outside Claude's process tree, so it outlives Claude's exit. */
async function launch($: Dollar, ctx: Context, mode: 'switch' | 'add', to?: string) {
  const args = [
    `-Mode ${mode}`,
    ctx.current ? `-From ${ctx.current.id}` : '',
    to ? `-To ${to}` : '',
    `-UserData "${ctx.userData}"`,
    `-Profiles "${ctx.profiles}"`,
  ].filter(Boolean)
  const line = `powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "${ctx.script}" ${args.join(' ')}`
  const quoted = `'${line.replaceAll("'", "''")}'`
  const command = [
    '$si = New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ ShowWindow = [uint16]0 }',
    `$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = ${quoted}; ProcessStartupInformation = $si }`,
    'exit $r.ReturnValue',
  ].join('; ')
  const ran = await $.process.run(
    ['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', command],
    { timeoutMs: 30_000 },
  )
  if (ran.exitCode !== 0) {
    throw new Error(ran.stderr.trim() || `Win32_Process.Create returned ${ran.exitCode}`)
  }
}

// ---------- module ----------

// Set at session.start (a reload runs it again), read by the press handlers.
let ctx: Context = { isDesktop: false, userData: '', profiles: '', script: '' }

async function start($: Dollar, mode: 'switch' | 'add', to?: Account) {
  if (!ctx.isDesktop) return
  const what = mode === 'switch' && to ? `Wechsle zu ${to.email}` : 'Melde ab für neuen Account'
  await refresh($, ctx, `${what} – Claude startet neu …`)
  $.ui.toast(`${what} – Claude startet neu …`)
  try {
    await launch($, ctx, mode, to?.id)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    await refresh($, ctx, `Wechsel fehlgeschlagen: ${message}`)
  }
}

export const register: Register = on => {

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    const execPath = (await $.env.get('CLAUDE_CODE_EXECPATH')) ?? ''
    const entry = await $.env.get('CLAUDE_CODE_ENTRYPOINT')
    const local = (await $.env.get('LOCALAPPDATA')) ?? ''
    const id = (await $.env.get('CLAUDE_CODE_ACCOUNT_UUID')) ?? ''
    const email = (await $.env.get('CLAUDE_CODE_USER_EMAIL')) ?? ''
    const orgId = await $.env.get('CLAUDE_CODE_ORGANIZATION_UUID')
    const cut = execPath.toLowerCase().indexOf('\\claude-code\\')
    const userData = cut > 0 ? execPath.slice(0, cut) : ''

    ctx = {
      isDesktop: entry === 'claude-desktop' && userData !== '' && local !== '',
      userData,
      profiles: `${local}\\ClaudeAccountSwitcher\\profiles`,
      script: `${$.plugin.root}\\scripts\\switch.ps1`,
      current: UUID.test(id) && email ? { id, email, orgId } : undefined,
    }

    await $.command.register({
      name: 'accounts',
      description: 'Claude-Desktop-Accounts mit 5h-/Wochenlimit anzeigen und wechseln',
    })

    // Recorded wherever the account is known: the tray app reads accounts.json.
    if (ctx.current) {
      let limits: readonly Limit[] = []
      try {
        limits = (await $.session.usage()).rateLimits
      } catch (error) {
        await writeStatus($, ctx, `session.usage: ${String(error)}`)
      }
      await recordLimits($, ctx, limits)
      await writeStatus($, ctx)
    }
    if (ctx.isDesktop) {
      $.clock.every(60_000, () => {
        void update($, view, v => ({ ...v, now: Date.now() }))
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
    const figures = windows(me)
      .map(l => current(l, v.now))
      .map(l => `${l.label} ${Number.isNaN(l.percent) ? '–' : `${l.percent}%`}`)
      .join(' · ')
    const others = v.accounts.filter(a => a.id !== me.id && a.hasProfile)

    return (
      <Box flexDirection="row" flexWrap="wrap" columnGap={1} alignItems="center">
        <Text>
          {shortName(me.email)} <Text dimColor>{figures}</Text>
        </Text>
        {v.notice ? (
          <Text color="warning">{v.notice}</Text>
        ) : (
          others.map(a => (
            <Button key={`band-${a.id}`} onPress={() => start($, 'switch', a)}>
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
        {v.accounts.map(a => {
          const isCurrent = a.id === v.currentId
          const age =
            a.measuredAt === undefined
              ? 'noch nicht gemessen'
              : isCurrent && v.now - a.measuredAt < 120_000
                ? 'Stand: live'
                : `Stand: vor ${span(v.now - a.measuredAt)}`

          return (
            <Box key={`acc-${a.id}`} flexDirection="column">
              <Text bold>
                {isCurrent ? '● ' : '○ '}
                {a.email}
                {isCurrent ? <Text color="success"> aktiv</Text> : ''}
              </Text>
              {windows(a).map(limit => {
                const l = current(limit, v.now)
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
                (a.hasProfile ? (
                  <Button
                    key={`switch-${a.id}`}
                    variant="primary"
                    onPress={() => start($, 'switch', a)}
                  >
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
        <Button key="add" onPress={() => start($, 'add')}>
          + Account hinzufügen
        </Button>
        <Text dimColor>
          Wechseln schliesst Claude kurz und startet es mit dem anderen Login neu. Laufende Sessions
          bleiben erhalten. Limits anderer Accounts zeigen den letzten Stand.
        </Text>
      </Box>
    )
  })
}

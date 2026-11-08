import { expect, test } from 'claude-code/testing'

const A = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
const B = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
const NOW = Date.parse('2026-11-08T10:00:00')
const HOUR = 3_600_000
const ROOT = 'C:\\Users\\me\\AppData\\Local\\ClaudeAccountSwitcher'
const APP = 'C:\\Users\\me\\AppData\\Local\\Claude Accounts\\claude-accounts.exe'

const ENV: Record<string, string> = {
  CLAUDE_CODE_ENTRYPOINT: 'claude-desktop',
  LOCALAPPDATA: 'C:\\Users\\me\\AppData\\Local',
  CLAUDE_CODE_ACCOUNT_UUID: A,
  CLAUDE_CODE_USER_EMAIL: 'alice@example.com',
  CLAUDE_CODE_ORGANIZATION_UUID: 'org-a',
}

// What the app published: Alice with an older reading, Bob with a reset five-hour window.
const VIEW = {
  app: APP,
  accounts: [
    {
      id: A,
      email: 'alice@example.com',
      name: 'Alice',
      hasProfile: false,
      windows: [{ kind: 'five_hour', percent: 5, resetsAt: NOW + HOUR, isReset: false, at: NOW - HOUR }],
      measuredAt: NOW - HOUR,
    },
    {
      id: B,
      email: 'bob@example.com',
      name: 'Bob',
      hasProfile: true,
      windows: [
        { kind: 'five_hour', percent: 80, resetsAt: NOW - HOUR, isReset: false, at: NOW - 3 * HOUR },
        { kind: 'seven_day', percent: 41, resetsAt: NOW + 30 * HOUR, isReset: false, at: NOW - 3 * HOUR },
      ],
      measuredAt: NOW - 3 * HOUR,
    },
  ],
}

const PANE = {
  plugin: 'account-switcher',
  component: 'Pane',
  requestId: 'account-switcher',
  props: {
    title: 'Accounts',
    isFocused: false,
    bodyColumns: 60,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 30 },
    view: {},
  },
} as const

test('records its own limits, lists both accounts and switches through the app', async ($, on) => {
  const runs: string[][] = []
  const written: Record<string, string> = {}
  on('env.get', (_, e) => ({ value: ENV[e.name] }))
  on('fs.read', (_, e) => {
    if (e.path === `${ROOT}\\view.json`) return { value: JSON.stringify(VIEW) as never }
    throw new Error('ENOENT')
  })
  on('fs.write', (_, e) => {
    written[e.path] = e.text

    return { value: undefined }
  })
  on('clock.now', () => ({ value: NOW }))
  on('session.usage', () => ({
    value: {
      startedAt: NOW,
      context: {} as never,
      rateLimits: [
        { kind: 'five_hour', percentUsed: 23, resetsAt: new Date(NOW + 2 * HOUR + 30 * 60_000).toISOString() },
        { kind: 'seven_day', percentUsed: 54.5, resetsAt: new Date(NOW + 50 * HOUR).toISOString() },
      ],
    },
  }))
  on('process.run', (_, e) => {
    runs.push([...e.argv])

    return { value: { exitCode: 0, stdout: '', stderr: '' } as never }
  })
  on('command.register', () => ({ value: { command: 'accounts' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))

  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })

  const record = JSON.parse(written[`${ROOT}\\limits\\${A}.json`] ?? '{}')
  expect(record.email).toBe('alice@example.com')
  expect(record.orgId).toBe('org-a')
  expect(record.limits.length).toBe(2)
  expect(record.measuredAt).toBe(NOW)

  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  expect((await ui.find({ type: 'Text', text: /alice@example\.com/ }))?.text).toContain('aktiv')
  // The session's own reading beats the app's older one.
  expect((await ui.find({ type: 'Text', text: /5h .*23%/ }))?.text).toContain('Reset in 2 h 30 min')
  expect((await ui.find({ type: 'Text', text: /Woche .*55%/ }))?.text).toBeTruthy()
  // Bob's five-hour window passed its reset: shown as 0 %.
  expect((await ui.find({ type: 'Text', text: /zurückgesetzt/ }))?.text).toContain('0%')
  expect((await ui.find({ type: 'Text', text: /vor 3 h/ }))?.text).toBeTruthy()

  await ui.press({ key: `switch-${B}` })
  expect(runs.length).toBe(1)
  const command = runs[0]?.join(' ') ?? ''
  expect(command).toContain('Win32_Process')
  expect(command).toContain(`"${APP}" --switch ${B}`)
  await ui.unmount()
})

test('without the app: limits only, no switch buttons', async ($, on) => {
  on('env.get', (_, e) => ({ value: ENV[e.name] }))
  on('fs.read', () => {
    throw new Error('ENOENT')
  })
  on('fs.write', () => ({ value: undefined }))
  on('clock.now', () => ({ value: NOW }))
  on('session.usage', () => ({ value: { startedAt: NOW, context: {} as never, rateLimits: [] } }))
  on('command.register', () => ({ value: { command: 'accounts' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))

  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })

  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  expect((await ui.find({ type: 'Text', text: /Claude Accounts/ }))?.text).toContain('Roost')
  expect((await ui.find({ type: 'Text', text: /alice@example\.com/ }))?.text).toContain('aktiv')
  expect(await ui.find({ type: 'Button', key: 'add' })).toBeFalsy()
  await ui.unmount()
})
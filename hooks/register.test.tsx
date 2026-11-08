import { expect, test } from 'claude-code/testing'

const A = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
const B = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
const NOW = Date.parse('2026-11-08T10:00:00')
const HOUR = 3_600_000

const ENV: Record<string, string> = {
  CLAUDE_CODE_EXECPATH:
    'C:\\Users\\me\\AppData\\Local\\Claude-Data\\claude_x\\claude-code\\2.1.317\\abc\\claude.exe',
  CLAUDE_CODE_ENTRYPOINT: 'claude-desktop',
  LOCALAPPDATA: 'C:\\Users\\me\\AppData\\Local',
  CLAUDE_CODE_ACCOUNT_UUID: A,
  CLAUDE_CODE_USER_EMAIL: 'alice@example.com',
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

test('lists both accounts with limits, reset times and a switch button', async ($, on) => {
  const runs: string[][] = []
  let stored: unknown = {
    [B]: {
      id: B,
      email: 'bob@example.com',
      // Five-hour window already reset; weekly still running.
      limits: [
        { kind: 'five_hour', percentUsed: 80, resetsAt: new Date(NOW - HOUR).toISOString() },
        { kind: 'seven_day', percentUsed: 41, resetsAt: new Date(NOW + 30 * HOUR).toISOString() },
      ],
      measuredAt: NOW - 3 * HOUR,
    },
  }
  on('env.get', (_, e) => ({ value: ENV[e.name] }))
  on('store.get', () => ({ value: stored }))
  on('store.set', (_, e) => {
    stored = e.value

    return { value: undefined }
  })
  on('fs.exists', (_, e) => ({ value: e.path.includes(B) }))
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

  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  const alice = await ui.find({ type: 'Text', text: /alice@example\.com/ })
  expect(alice?.text).toContain('aktiv')
  expect((await ui.find({ type: 'Text', text: /5h .*23%/ }))?.text).toContain('Reset in 2 h 30 min')
  expect((await ui.find({ type: 'Text', text: /Woche .*55%/ }))?.text).toBeTruthy()
  // Bob's five-hour window passed its reset: shown as 0 %.
  expect((await ui.find({ type: 'Text', text: /zurückgesetzt/ }))?.text).toContain('0%')
  expect((await ui.find({ type: 'Text', text: /vor 3 h/ }))?.text).toBeTruthy()

  await ui.press({ key: `switch-${B}` })
  expect(runs.length).toBe(1)
  const command = runs[0]?.join(' ') ?? ''
  expect(command).toContain('Win32_Process')
  expect(command).toContain(`-Mode switch -From ${A} -To ${B}`)
  expect(command).toContain('-UserData "C:\\Users\\me\\AppData\\Local\\Claude-Data\\claude_x"')
  await ui.unmount()
})

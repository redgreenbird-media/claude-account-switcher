/** One rate-limit window as the session reports it. */
export type Limit = { kind: string; percentUsed: number; resetsAt?: string }

/** A window as the Claude Accounts app publishes it (times in ms since epoch). */
export type Window = {
  kind: string
  percent?: number | null
  resetsAt?: number | null
  /** The window has reset since the reading. */
  isReset: boolean
  at?: number | null
}

/** An account the Claude Accounts app knows. */
export type Account = {
  id: string
  email: string
  name: string
  /** Whether a saved login exists that the app can restore. */
  hasProfile: boolean
  windows: Window[]
  /** When the newest reading was taken (ms since epoch). */
  measuredAt?: number
}

export type View = {
  isDesktop: boolean
  /** The app's exe, from its view.json; absent when the app is not installed. */
  app?: string
  currentId?: string
  accounts: Account[]
  /** Clock used for relative reset times; bumped every minute. */
  now: number
  /** Text shown while a switch is starting, or the last error. */
  notice?: string
}

declare module 'claude-code' {
  interface PluginState {
    'account-switcher': { view: Shaped<View> }
  }
}
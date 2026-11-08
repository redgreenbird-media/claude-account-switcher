/** One rate-limit window as last measured for an account. */
export type Limit = { kind: string; percentUsed: number; resetsAt?: string }

/** An account the switcher has seen signed in to Claude Desktop. */
export type Account = {
  id: string
  email: string
  orgId?: string
  limits: Limit[]
  /** When `limits` were measured (ms since epoch). */
  measuredAt?: number
  /** Whether a saved login exists that the switcher can restore. */
  hasProfile: boolean
}

export type View = {
  isDesktop: boolean
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

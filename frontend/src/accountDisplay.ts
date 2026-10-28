export interface AccountDisplayInput {
  enabled?: boolean
  name: string
  email?: string | null
}

export function accountDisplayName(account: AccountDisplayInput): string {
  const email = account.email?.trim()
  const name = email || account.name.trim()
  return account.enabled === false ? name + '（已停止调度）' : name
}

// 请求候选不等于检测基线，也不保证每个账号均能使用。
export function requestModels(baselineModels: string[]): string[] {
  return [...new Set(['gpt-6.1-sol', 'gpt-6.1-terra', ...baselineModels])]
}

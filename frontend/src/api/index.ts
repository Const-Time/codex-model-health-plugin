export interface BridgeInput { path: string; method: string; contentType?: string; body?: string }
export interface PluginHost { version: number; request(input: BridgeInput): Promise<{status: number; contentType: string; body: ArrayBuffer}> }
declare global { interface Window { codexProxyPlugin?: PluginHost } }

export async function api<T>(path: string, data?: object): Promise<T> {
  const host = window.codexProxyPlugin
  if (!host || host.version !== 2) throw new Error('请从 Codex Proxy 的插件菜单打开此页')
  const reply = await host.request({ path: `api/${path}`, method: data === undefined ? 'GET' : 'POST', ...(data === undefined ? {} : { contentType: 'application/json', body: JSON.stringify(data) }) })
  if (!reply.contentType.toLowerCase().startsWith('application/json')) throw new Error('插件响应类型错误，请检查安装包资源')
  const body = JSON.parse(new TextDecoder().decode(reply.body))
  if (reply.status < 200 || reply.status >= 300) throw new Error(body.message || body.error?.message || `请求失败（${reply.status}）`)
  return body as T
}

export type Method = 'meow' | 'modeltrace'
export interface MethodCatalog { method: Method; label: string; engine_version: string; id: string; version: string; hash: string; models: {id: string; label?: string}[]; tiers: {id: string; count: number}[]; family_count?: number }
export interface Catalog { methods: MethodCatalog[] }
export interface UpstreamSource { method: Method; label: string; url: string; bundled_commit: string }
export interface UpstreamCheck { method: Method; status: 'identical' | 'ahead' | 'behind' | 'diverged'; checked_at: number; latest_commit: string; ahead_by: number; behind_by: number; compare_url: string }
export interface Bootstrap { background_supported: boolean; catalog: Catalog; sources: UpstreamSource[]; accounts: {id: string; name: string; email?: string | null; plan: string | null}[]; keys: {id: string; name: string; enabled: boolean}[] }
interface ReportBase { valid_samples: number; planned_samples: number; reasons: string[] }
export interface MeowReport extends ReportBase { method?: 'meow'; verdict: 'match' | 'mismatch' | 'insufficient'; model: string | null; matches: Record<string, number>; thresholds: Record<string, number>; quality_status: string }
export interface ModelTraceReport extends ReportBase { method: 'modeltrace'; verdict: 'attributed' | 'insufficient'; analysis: null | { prediction_name: string; family_prediction_name: string; results: {model: string; display_name: string; probability: number}[]; family_probabilities: {family: string; display_name: string; probability: number}[] } }
export type Report = MeowReport | ModelTraceReport
export interface Attempt { job_id: string; status: string; error?: string | null; category?: string; answer?: string; answer_truncated?: boolean; request_id?: string | null; response_id?: string | null; usage?: {input_tokens?: number | null; output_tokens?: number | null} }
export type Status = 'ready' | 'running' | 'paused' | 'completed' | 'stopped'
export interface Run { id: string; created_at: number; updated_at: number; account_id: string; account_name: string; key_name: string; plan: {method?: Method; engine_version: string; id: string; hash: string; version: string; tier: string; claimed_model: string; request_model: string; jobs: {job_id: string}[]; retry_budget: number}; status: Status; attempts: Attempt[]; inflight: string | null; report: Report }
export interface History { id: string; method?: Method; created_at: number; account_name: string; account_id: string; key_name: string; request_model: string; claimed_model: string; tier: string; baseline_hash: string; status: Status; attempts: number; planned: number; report: Report }

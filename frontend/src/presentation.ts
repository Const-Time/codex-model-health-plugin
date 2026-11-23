import type { Method, Report, Status } from './api'
export function methodText(method?: Method) { return method === 'modeltrace' ? 'ModelTrace' : 'Meow' }
export const statusText: Record<Status, string> = {ready: '等待继续', running: '检测中', paused: '已暂停', completed: '已完成', stopped: '已结束'}
export function verdict(report: Report, status: Status) {
  if (status !== 'completed' && status !== 'stopped') return '检测尚未结束'
  if (report.method === 'modeltrace') return report.verdict === 'attributed' && report.analysis ? `更接近 ${report.analysis.prediction_name}` : '证据不足'
  return {match: '行为与基线匹配', mismatch: '行为偏离所选基线', insufficient: '证据不足'}[report.verdict]
}
export function tone(report: Report, status: Status): 'success' | 'danger' | 'warning' | 'primary' {
  if (status !== 'completed' && status !== 'stopped') return 'primary'
  if (report.method === 'modeltrace') return report.verdict === 'attributed' ? 'primary' : 'warning'
  return {match: 'success', mismatch: 'danger', insufficient: 'warning'}[report.verdict] as 'success' | 'danger' | 'warning'
}
export const reasonText: Record<string, string> = {samples_incomplete: '各题组有效样本未达到基线要求', no_valid_samples: '还没有有效样本', no_threshold: '没有候选超过判定阈值', multiple_thresholds: '候选结果存在歧义', uncalibrated: '基线校准不匹配', unknown_claimed_model: '所选基线不包含该模型', baseline_cell_missing: '基线缺少题组', samples_exceed_plan: '样本数量超过计划'}
export function formatTime(value: number) { return new Date(value).toLocaleString('zh-CN', {hour12: false}) }

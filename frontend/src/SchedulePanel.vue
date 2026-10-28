<script setup lang="ts">
import { computed, onMounted, onScopeDispose, ref, watch } from 'vue'
import { BaseButton, BaseCard, BaseCheckbox, BaseInput, BaseNumberInput, BaseSegmented, BaseSelect, BaseSwitch, BaseTag } from '@codex-proxy/ui'
import { api } from './api'
import type { ScheduleTiming, ScheduleDetection, MethodCatalog } from './api'
import { accountDisplayName } from './accountDisplay'
import { formatTime } from './presentation'
import ModelSelect from './ModelSelect.vue'

const props = defineProps<{ accounts: { id: string; name: string; email?: string | null }[]; models: string[]; methods: MethodCatalog[]; scheduledDetectionSupported?: boolean; maximumConcurrentAccounts?: number }>()
const emit = defineEmits<{ 'view-report': [id: string] }>()
type Entry = { account_id: string; name: string; next_at: number; status: string; checked_at: number | null; message: string; pending: string | null; run_id?: string | null }
type Plan = { concurrency: number; enabled: boolean; model: string; interval_minutes: number; timing: ScheduleTiming; detection: ScheduleDetection; accounts: Entry[] }
const legacyDetection = (): ScheduleDetection => ({ method: 'short', tier: 'low', claimed_model: '' })
const detectionMethods = [{ value: 'meow', label: 'Meow' }, { value: 'modeltrace', label: 'ModelTrace' }, { value: 'short', label: '仅短探测' }]
const meowCatalog = computed(() => props.methods.find(method => method.method === 'meow'))
const baselines = computed(() => meowCatalog.value?.models.map(model => ({ value: model.id, label: model.label || model.id })) || [])
const tiers = computed(() => meowCatalog.value?.tiers.map(tier => ({ value: tier.id, label: `${tier.count} 题` })) || [])
const browserTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
const defaultTiming = (): ScheduleTiming => ({ mode: 'interval', daily_time: '09:00', cron: '0 9 * * *', timezone: browserTimezone })
const plan = ref<Plan>({ concurrency: 2, enabled: false, model: '', interval_minutes: 360, timing: defaultTiming(), detection: legacyDetection(), accounts: [] })
const concurrency = computed({ get: () => String(plan.value.concurrency), set: (value: string) => { plan.value.concurrency = Number(value) } })
const busy = ref(false), loaded = ref(false), error = ref(''), resume = ref<string[]>([]), saved = ref('')
const timingModes = [{ value: 'interval', label: '按间隔' }, { value: 'daily', label: '每天指定时间' }, { value: 'cron', label: 'Cron 表达式' }]
const timezones = computed(() => [...new Set(['UTC', browserTimezone, plan.value.timing.timezone, ...Intl.supportedValuesOf('timeZone')])].map(value => ({ value, label: value === 'Asia/Shanghai' ? 'Asia/Shanghai（北京时间）' : value })))
const displayTimezone = computed(() => plan.value.timing.mode === 'interval' ? browserTimezone : plan.value.timing.timezone)
const nextTimes = ref<number[]>([]), previewError = ref(''), previewBusy = ref(false)
let previewTimer: ReturnType<typeof setTimeout> | undefined
let previewSequence = 0
watch(() => [plan.value.timing, plan.value.interval_minutes, loaded.value], () => {
  clearTimeout(previewTimer)
  const sequence = ++previewSequence
  nextTimes.value = []; previewError.value = ''; previewBusy.value = loaded.value
  if (!loaded.value) return
  previewTimer = setTimeout(async () => {
    try {
      const result = await api<{ next_at: number[] }>('schedule/preview', { timing: plan.value.timing, interval_minutes: plan.value.interval_minutes })
      if (sequence === previewSequence) nextTimes.value = result.next_at
    } catch (cause) { if (sequence === previewSequence) previewError.value = String(cause) }
    finally { if (sequence === previewSequence) previewBusy.value = false }
  }, 350)
}, { deep: true, flush: 'sync' })
onScopeDispose(() => { clearTimeout(previewTimer); previewSequence++ })
function scheduleTime(value: number) {
  try { return new Intl.DateTimeFormat('zh-CN', { timeZone: displayTimezone.value, dateStyle: 'medium', timeStyle: 'short', hourCycle: 'h23' }).format(value) }
  catch { return formatTime(value) }
}
const selectedIds = computed({
  get: () => plan.value.accounts.map(entry => entry.account_id),
  set: (ids: string[]) => {
    const pending = plan.value.accounts.filter(entry => entry.pending || entry.status === 'running').map(entry => entry.account_id)
    const next = [...new Set([...ids, ...pending])]
    if (next.length > 50) { error.value = '最多选择 50 个账号'; return }
    plan.value.accounts = next.flatMap(id => {
      const existing = plan.value.accounts.find(entry => entry.account_id === id)
      if (existing) return [existing]
      const account = props.accounts.find(account => account.id === id)
      return account ? [{ account_id: id, name: accountDisplayName(account), next_at: 0, status: 'waiting', checked_at: null, message: '', pending: null }] : []
    })
    resume.value = resume.value.filter(id => next.includes(id))
  },
})
const accountOptions = computed(() => {
  const options = props.accounts.map(account => ({
    value: account.id,
    label: accountDisplayName(account),
    disabled: plan.value.accounts.some(entry => entry.account_id === account.id && (!!entry.pending || entry.status === 'running')) || (selectedIds.value.length >= 50 && !selectedIds.value.includes(account.id)),
  }))
  for (const entry of plan.value.accounts) {
    if (!options.some(option => option.value === entry.account_id)) options.push({ value: entry.account_id, label: `${entry.name}（当前不可用）`, disabled: (!!entry.pending || entry.status === 'running') })
  }
  return options
})
const hasTargets = computed(() => !plan.value.enabled || (plan.value.model.trim() && selectedIds.value.length > 0))
const detectionValid = computed(() => !plan.value.enabled || plan.value.detection.method === 'short' || (props.scheduledDetectionSupported && (plan.value.detection.method !== 'meow' || !!plan.value.detection.claimed_model)))
const valid = computed(() => hasTargets.value && detectionValid.value && !previewBusy.value && !previewError.value && nextTimes.value.length > 0)
watch(plan, () => { saved.value = '' }, { deep: true, flush: 'sync' })
function toggleResume(id: string, checked: boolean) {
  resume.value = checked ? [...new Set([...resume.value, id])] : resume.value.filter(value => value !== id)
  saved.value = ''
}
async function load() {
  busy.value = true; error.value = ''; saved.value = ''
  try { const result = await api<Plan>('schedule'); plan.value = { ...result, concurrency: result.concurrency ?? 1, timing: { ...defaultTiming(), ...result.timing }, detection: { ...legacyDetection(), ...result.detection } }; resume.value = []; loaded.value = true }
  catch (cause) { error.value = String(cause) }
  finally { busy.value = false }
}
async function save() {
  if (!loaded.value || busy.value || !valid.value) return
  busy.value = true; error.value = ''; saved.value = ''
  try {
    const result = await api<Plan>('schedule', { concurrency: plan.value.concurrency, enabled: plan.value.enabled, model: plan.value.model.trim(), interval_minutes: plan.value.interval_minutes, timing: plan.value.timing, detection: plan.value.detection, account_ids: selectedIds.value, resume_accounts: resume.value })
    plan.value = { ...result, concurrency: result.concurrency ?? 1, timing: { ...defaultTiming(), ...result.timing }, detection: { ...legacyDetection(), ...result.detection } }
    resume.value = []; saved.value = '已保存。按下方账号的下次执行时间运行，关闭页面也会执行。'
  } catch (cause) { error.value = String(cause) }
  finally { busy.value = false }
}
onMounted(load)
</script>

<template>
  <BaseCard>
    <template #header><h2>定时账号探测</h2></template>
    <template #body>
      <p class="muted text-cp-sm">仅对所选账号执行所选检测，不同账号可并行，同一账号逐题串行。消耗真实上游额度，不扣用户 Key 额度；额度窗口计时由上游决定，成功不代表账号一定没有风控。</p>
      <div class="schedule-toggle">
        <div><strong>启用定时任务</strong><p class="muted text-cp-xs">保存后生效，关闭页面也会按计划执行</p></div>
        <BaseSwitch v-model="plan.enabled" label="启用定时任务" :disabled="busy || !loaded" />
      </div>
      <div class="form-grid">
        <div class="field col-span-full">
          <label>检测方法</label>
          <BaseSegmented v-model="plan.detection.method" label="定时检测方法" :options="detectionMethods" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">Meow / ModelTrace 会完成多题检测并生成报告；仅短探测只检查请求是否成功，不生成模型指纹报告。</p>
          <p v-if="plan.detection.method !== 'short' && !scheduledDetectionSupported" role="alert" class="text-cp-sm">请先升级配套主程序，当前宿主不支持定时完整检测。</p>
        </div>
        <template v-if="plan.detection.method === 'meow'">
          <div class="field"><label>对照基线</label><BaseSelect v-model="plan.detection.claimed_model" aria-label="定时检测对照基线" :options="baselines" :disabled="busy || !loaded" placeholder="选择要核对的模型" /></div>
          <div class="field"><label>检测档位</label><BaseSelect v-model="plan.detection.tier" aria-label="定时检测档位" :options="tiers" :disabled="busy || !loaded" /></div>
        </template>
        <p v-if="plan.detection.method === 'modeltrace'" class="muted text-cp-sm col-span-full">统一候选库，三轮数字检测；完成后可在检测历史查看归属报告。</p>
        <div class="field col-span-full">
          <label id="schedule-accounts-label">探测账号 <span class="muted">{{ selectedIds.length }} / 50</span></label>
          <BaseSelect v-model="selectedIds" aria-label="探测账号" :options="accountOptions" multiple filterable collapse-tags :max-collapse-tags="3" :disabled="busy || !loaded" placeholder="搜索并多选账号" empty-text="没有可选账号" />
          <p class="muted text-cp-xs m-0">可搜索账号并连续勾选，最多 50 个；正在执行的账号暂不可移除。</p>
        </div>
        <div class="field">
          <label>并行账号数</label>
          <BaseSelect v-model="concurrency" aria-label="并行账号数" :options="[{value:'1',label:'1（串行）'},{value:'2',label:'2（推荐）'},{value:'3',label:'3'}]" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">最多 3 个账号同时请求，共用一个检测引擎。调整不重置已有进度；手动检测与定时批次互斥。</p>
          <p v-if="plan.concurrency > (maximumConcurrentAccounts ?? 1)" class="text-cp-sm">当前宿主最多支持 {{ maximumConcurrentAccounts ?? 1 }} 个账号并行；升级配套主程序后按此设置执行。</p>
        </div>
        <div class="field col-span-full">
          <label>请求模型</label>
          <ModelSelect v-model="plan.model" label="定时探测请求模型" :models="models" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">候选来自检测基线；所有所选账号使用同一模型，请确认均支持。列表外可选择自定义模型。</p>
        </div>
        <div class="field col-span-full">
          <label>调度方式</label>
          <BaseSegmented v-model="plan.timing.mode" label="调度方式" :options="timingModes" :disabled="busy || !loaded" />
        </div>
        <div v-if="plan.timing.mode === 'interval'" class="field">
          <label>间隔（分钟）</label>
          <BaseNumberInput v-model="plan.interval_minutes" label="探测间隔（分钟）" :min="60" :max="10080" size="md" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">60–10080 分钟；新增或重新启用后等待一个间隔。</p>
        </div>
        <div v-else-if="plan.timing.mode === 'daily'" class="field">
          <label>每天执行时间</label>
          <BaseInput v-model="plan.timing.daily_time" type="time" aria-label="每天执行时间" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">按所选时区，每天从这个时刻开始逐个探测账号。</p>
        </div>
        <div v-else class="field">
          <label>Cron 表达式</label>
          <BaseInput v-model="plan.timing.cron" aria-label="Cron 表达式" placeholder="0 9 * * *" maxlength="128" :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">分 时 日 月 周（五段，不含秒）。例如 0 9,18 * * 1-5：工作日 09:00 和 18:00。</p>
        </div>
        <div v-if="plan.timing.mode !== 'interval'" class="field">
          <label>执行时区</label>
          <BaseSelect v-model="plan.timing.timezone" aria-label="执行时区" :options="timezones" filterable :disabled="busy || !loaded" />
          <p class="muted text-cp-xs m-0">保存所选时区，不随服务器或浏览器时区变化。</p>
        </div>
        <div class="schedule-preview col-span-full" aria-live="polite">
          <strong>后续执行时刻 · {{ displayTimezone }}</strong>
          <p v-if="previewBusy" class="muted text-cp-sm">正在校验并计算…</p>
          <p v-else-if="previewError" role="alert" class="text-cp-sm">{{ previewError }}</p>
          <ol v-else class="text-cp-sm"><li v-for="at in nextTimes" :key="at">{{ scheduleTime(at) }}</li></ol>
          <p class="muted text-cp-xs">按当前表单预览，保存后生效。到点后由宿主分批调度；每批最多并行所选数量的账号，各完成一道题后进入下一轮，轮次之间约间隔 30 秒。</p>
        </div>
      </div>
      <p v-if="plan.enabled && !hasTargets" role="status" class="muted text-cp-sm">请选择至少一个账号和请求模型，再保存启用。</p>
      <div v-if="plan.accounts.length" class="schedule-entries">
        <div v-for="entry in plan.accounts" :key="entry.account_id" class="schedule-entry">
          <div class="schedule-entry-heading"><strong>{{ entry.name }}</strong><BaseTag :type="entry.status === 'paused' ? 'warning' : entry.status === 'ok' ? 'success' : 'neutral'">{{ entry.pending || entry.status === 'running' ? '执行中' : entry.status === 'paused' ? '已暂停' : entry.status === 'ok' ? '上次成功' : '等待执行' }}</BaseTag><BaseButton size="sm" :disabled="busy || (!!entry.pending || entry.status === 'running')" :aria-label="`移除 ${entry.name}`" @click="selectedIds = selectedIds.filter(id => id !== entry.account_id)">移除</BaseButton></div>
          <p class="muted text-cp-xs">上次：{{ entry.checked_at ? scheduleTime(entry.checked_at) : '尚未执行' }} · 下次：{{ entry.status === 'paused' || !plan.enabled ? '已暂停' : entry.next_at ? scheduleTime(entry.next_at) : '保存后计算' }} · {{ displayTimezone }}</p>
          <p v-if="entry.message" class="text-cp-sm">{{ entry.message }}</p>
          <BaseButton v-if="entry.run_id" size="sm" @click="emit('view-report', entry.run_id)">查看检测报告</BaseButton>
          <BaseCheckbox v-if="entry.status === 'paused'" :model-value="resume.includes(entry.account_id)" :disabled="busy" :label="`已核对，保存后恢复 ${entry.name}`" show-label @update:model-value="toggleResume(entry.account_id, $event)" />
        </div>
      </div>
      <p v-if="error" role="alert" class="error-panel">{{ error }}</p><p v-if="saved" role="status" class="muted text-cp-sm">{{ saved }}</p>
      <div class="flex flex-wrap gap-3 mt-4"><BaseButton variant="primary" :loading="busy" :disabled="!loaded || !valid" @click="save">保存计划</BaseButton><BaseButton :disabled="busy" @click="load">刷新状态</BaseButton></div>
      <p class="muted text-cp-xs">失败的当前题最多额外重试 2 次，仍失败则暂停对应账号，成功的题不重跑；模型不匹配属于有效结果。每次后台调度每个账号最多执行一道题，错过的周期不补跑。恢复时沿用历史中配置一致的检测进度。</p>
    </template>
  </BaseCard>
</template>

<style scoped>
.schedule-toggle { display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: 1rem; margin: 1.25rem 0; border-radius: var(--cp-border-radius); background: var(--cp-color-fill-quaternary); }
.schedule-toggle p { margin: .4rem 0 0; }
.schedule-preview { padding: 1rem; border-radius: var(--cp-border-radius); background: var(--cp-color-fill-quaternary); }
.schedule-preview ol { margin: .5rem 0; padding-left: 1.25rem; line-height: 1.8; }
.schedule-entries { margin-top: 1.25rem; }
.schedule-entry { padding: 1rem 0; border-bottom: 1px solid var(--cp-color-border); overflow-wrap: anywhere; }
.schedule-entry-heading { display: flex; flex-wrap: wrap; align-items: center; gap: .75rem; }
.schedule-entry-heading strong { flex: 1; min-width: 0; }
</style>

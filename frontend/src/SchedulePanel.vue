<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { BaseButton, BaseCard } from '@codex-proxy/ui'
import { api } from './api'
import { formatTime } from './presentation'
const props = defineProps<{accounts:{id:string;name:string;email?:string|null}[]}>()
type Entry = { account_id:string;name:string;next_at:number;status:string;checked_at:number|null;message:string;pending:string|null }
type Plan = {enabled:boolean;model:string;interval_minutes:number;accounts:Entry[]}
const plan = ref<Plan>({enabled:false,model:'',interval_minutes:360,accounts:[]})
const selected = ref(''), busy = ref(false), error = ref(''), resume = ref<string[]>([]), saved = ref('')
async function load() { busy.value=true; error.value=''; try { plan.value=await api<Plan>('schedule'); resume.value=[] } catch(e) { error.value=String(e) } finally {busy.value=false} }
function add() { const account=props.accounts.find(a=>a.id===selected.value); if (!account || plan.value.accounts.some(a=>a.account_id===account.id) || plan.value.accounts.length>=50) return; plan.value.accounts.push({account_id:account.id,name:account.email||account.name,next_at:0,status:'waiting',checked_at:null,message:'',pending:null}); selected.value='' }
async function save() { busy.value=true; error.value=''; saved.value=''; try { plan.value=await api<Plan>('schedule',{enabled:plan.value.enabled,model:plan.value.model,interval_minutes:Number(plan.value.interval_minutes),account_ids:plan.value.accounts.map(a=>a.account_id),resume_accounts:resume.value}); resume.value=[]; saved.value='已保存。新增或重新启用的计划从一个间隔后开始，关闭页面也会执行。' } catch(e) {error.value=String(e)} finally {busy.value=false} }
onMounted(load)
</script>
<template>
  <BaseCard>
    <template #header><h2>定时账号探测</h2></template>
    <template #body>
      <p class="muted">仅对手动选择的账号发送短请求，全局串行。消耗真实上游额度，不扣用户 Key 额度。成功调用可能触发额度窗口计时，实际规则由上游决定；成功不代表账号一定没有风控。</p>
      <fieldset :disabled="busy" class="schedule-fields">
        <label><input v-model="plan.enabled" type="checkbox"> 启用定时任务</label>
        <label>请求模型 <input v-model="plan.model" maxlength="128" placeholder="填写所有所选账号支持的模型"></label>
        <label>间隔（分钟） <input v-model="plan.interval_minutes" type="number" min="60" max="10080"></label>
        <label>添加账号 <select v-model="selected"><option value="">请选择账号</option><option v-for="a in accounts" :key="a.id" :value="a.id">{{ a.email || a.name }}</option></select></label>
        <BaseButton :disabled="!selected || plan.accounts.length>=50" @click="add">添加</BaseButton>
      </fieldset>
      <div v-for="entry in plan.accounts" :key="entry.account_id" class="schedule-entry">
        <strong>{{ entry.name }}</strong>
        <span>{{ entry.pending ? '执行中' : entry.status === 'paused' ? '已暂停' : entry.status === 'ok' ? '上次成功' : '等待执行' }}</span>
        <p class="muted">上次：{{ entry.checked_at ? formatTime(entry.checked_at) : '尚未执行' }} · 下次：{{ entry.status === 'paused' || !plan.enabled ? '已暂停' : entry.next_at ? formatTime(entry.next_at) : '保存后计算' }}</p>
        <p>{{ entry.message }}</p>
        <label v-if="entry.status === 'paused'"><input v-model="resume" type="checkbox" :value="entry.account_id"> 已核对，保存后恢复此账号</label>
        <BaseButton size="sm" :disabled="busy || !!entry.pending" @click="plan.accounts=plan.accounts.filter(a=>a.account_id!==entry.account_id)">移除</BaseButton>
      </div>
      <p v-if="error" role="alert">{{ error }}</p><p v-if="saved" role="status">{{ saved }}</p>
      <div class="flex gap-3 mt-4"><BaseButton variant="primary" :loading="busy" @click="save">保存计划</BaseButton><BaseButton :disabled="busy" @click="load">刷新状态</BaseButton></div>
      <p class="muted text-cp-xs">失败或结果未知时暂停对应账号，不自动重试；错过的执行周期不会补发。每次后台检查最多执行一个账号。</p>
    </template>
  </BaseCard>
</template>
<style scoped>
.schedule-fields {display:flex;flex-wrap:wrap;gap:1rem;border:0;padding:0}.schedule-fields label{display:flex;align-items:center;gap:.5rem}.schedule-fields input,.schedule-fields select{max-width:22rem;padding:.5rem;border:1px solid var(--cp-border,#d6dae0);border-radius:.4rem;background:transparent;color:inherit}.schedule-entry{padding:1rem 0;border-bottom:1px solid var(--cp-border,#d6dae0)}.schedule-entry>span{margin-left:1rem}
</style>

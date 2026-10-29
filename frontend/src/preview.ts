import { applyResolvedTheme, resolveTheme, DEFAULT_THEME_COLOR, DEFAULT_CUSTOM_THEME_COLOR } from '@codex-proxy/ui/theme'
import type { Catalog, MethodCatalog, ModelTraceReport, Report, Run, History, UpstreamSource } from './api'
import { accountDisplayName } from './accountDisplay'

const params = new URLSearchParams(location.search)
const sources: UpstreamSource[] = [
  {method:'meow',label:'Meow',url:'https://github.com/chen-006/meow-llm-detector',bundled_commit:'dc608d5c097abf96575d5b34f49952ddbd27c281'},
  {method:'modeltrace',label:'ModelTrace',url:'https://github.com/xqy2006/ModelTrace',bundled_commit:'6cd4a8ff0f3f4e49d6d6969dd2fbadfa1569fde2'},
]
const catalog: MethodCatalog = {"method": "meow", "label": "Meow", "engine_version": "4.5.4", "id": "meow-gpt-other-cap98-efficient", "version": "4.5.4-predictive.20261003.1", "hash": "ff04ec844b5bd63c189748cff4caaf039be9cb908f398cb6c279efab9e8c4aec", "models": [{"id": "gpt-6.1-sol"}, {"id": "gpt-6-astra"}, {"id": "gpt-5.6-terra"}, {"id": "gpt-6-luna"}], "tiers": [{"id": "low", "count": 32}, {"id": "medium", "count": 64}, {"id": "high", "count": 128}]}
const trace: MethodCatalog = {"method": "modeltrace", "label": "ModelTrace", "id": "modeltrace-unified", "engine_version": "6cd4a8f", "version": "6cd4a8f", "hash": "eeab6ff54d6ffb2b294ebfd6cce25bce387f7a268b66759144f64c16f6c66b9f", "models": [{"id": "gpt-5.4", "label": "gpt-5.4"}, {"id": "gpt-5.5", "label": "gpt-5.5"}, {"id": "gpt-5.6-sol", "label": "gpt-5.6-sol"}, {"id": "gpt-5.6-terra", "label": "gpt-5.6-terra"}, {"id": "gpt-5.6-luna", "label": "gpt-5.6-luna"}, {"id": "gpt-6-astra", "label": "gpt-6-astra"}, {"id": "gpt-6-sol", "label": "gpt-6-sol"}, {"id": "gpt-6-luna", "label": "gpt-6-luna"}, {"id": "claude-haiku-4-5-20251001", "label": "claude-haiku-4-5-20251001"}, {"id": "claude-sonnet-4-6", "label": "claude-sonnet-4-6"}, {"id": "claude-sonnet-5", "label": "claude-sonnet-5"}, {"id": "claude-opus-4-6", "label": "claude-opus-4-6"}, {"id": "claude-opus-4-7", "label": "claude-opus-4-7"}, {"id": "claude-opus-4-8", "label": "claude-opus-4-8"}, {"id": "claude-opus-5", "label": "claude-opus-5"}, {"id": "claude-opus-5-5", "label": "claude-opus-5-5"}, {"id": "claude-sonnet-5-5", "label": "claude-sonnet-5-5"}, {"id": "claude-haiku-5-5", "label": "claude-haiku-5-5"}], "family_count": 2, "tiers": [{"id": "fixed", "count": 3}]}
const methods: Catalog = {methods:[catalog,trace]}
const previewAccounts = [
  {id:'demo-a',name:'OPENAI账号',email:'primary@example.com',plan:'pro'},
  {id:'demo-b',name:'OPENAI账号',email:'backup@example.com',plan:'plus'},
  {id:'demo-c',name:'无邮箱账号',email:'   ',plan:'free'},
]
function traceReport(valid: number): ModelTraceReport {return {method:'modeltrace',verdict:valid===3?'attributed':'insufficient',valid_samples:valid,planned_samples:3,reasons:valid===3?[]:['samples_incomplete'],analysis:valid?{prediction_name:'gpt-6-astra',family_prediction_name:'GPT',results:['gpt-6-astra','gpt-6-sol','gpt-5.6-sol','claude-opus-5','gpt-5.5'].map((model,i)=>({model,display_name:model,probability:[.84,.08,.03,.025,.025][i]!})),family_probabilities:[{family:'gpt',display_name:'GPT',probability:.975},{family:'claude',display_name:'Claude',probability:.025}]}:null}}
function report(valid: number, total: number): Report { return {verdict:valid >= total*.6 ? 'match' : 'insufficient',model:valid>=total*.6 ? 'gpt-6-astra':null,valid_samples:valid,planned_samples:total,matches:{'gpt-6-astra':valid ? .987 : .333,'gpt-5.6-sol':valid ? .011 : .333,'other_known_external':valid ? .002 : .334},thresholds:{'gpt-6-astra':.92,'gpt-5.6-sol':.94,'other_known_external':.98},reasons:valid>=total*.6?[]:['samples_incomplete'],quality_status:valid>=total*.6?'sufficient':'insufficient_valid_samples'} }
function example(id: string, total = 32): Run {return {id,created_at:Date.now()-3600000,updated_at:Date.now()-3500000,account_id:'demo-a',account_name:'主账号 A · 固定代理',key_name:'模型体检专用',status:'completed',inflight:null,plan:{engine_version:'4.5.4',id:catalog.id,version:catalog.version,hash:catalog.hash,tier:'low',claimed_model:'gpt-6-astra',request_model:'gpt-6-astra',jobs:Array.from({length:total},(_,i)=>({job_id:`probe-${i%6}:${Math.floor(i/6)}`})),retry_budget:Math.ceil(total/2)},attempts:Array.from({length:total},(_,i)=>({job_id:`probe-${i%6}:${Math.floor(i/6)}`,status:'ok',category:'cat',answer:'cat',request_id:`preview-request-${i+1}`})),report:report(total,total)}}
function summary(r: Run): History {return {id:r.id,method:r.plan.method,created_at:r.created_at,account_name:r.account_name,account_id:r.account_id,key_name:r.key_name,request_model:r.plan.request_model,claimed_model:r.plan.claimed_model,tier:r.plan.tier,baseline_hash:r.plan.hash,status:r.status,attempts:r.attempts.length,planned:r.plan.jobs.length,report:r.report}}

export function installPreview() {
  applyResolvedTheme(document.documentElement,resolveTheme(params.get('theme') === 'dark' ? 'dark' : 'light',DEFAULT_THEME_COLOR,DEFAULT_CUSTOM_THEME_COLOR))
  document.body.style.background='var(--cp-color-bg-layout)'
  document.body.style.padding='24px'
  const runs: Run[] = params.get('scenario') === 'empty' ? [] : [example('0-1')]
  const active = new Set<string>()
  let schedule = { concurrency:2, detection:{method:"short",tier:"low",claimed_model:""}, enabled:false, model:'', interval_minutes:360, timing:{mode:'interval',daily_time:'09:00',cron:'0 9 * * *',timezone:Intl.DateTimeFormat().resolvedOptions().timeZone}, accounts:[] as {account_id:string;name:string;next_at:number;status:string;checked_at:null;message:string;pending:null}[] }
  // Development fixture only. Production previews use the Rust scheduler, including DST rules.
  function scheduleDates(timing: typeof schedule.timing, interval: number) {
    if (timing.mode === 'interval') return [1,2,3].map(i => Date.now()+i*interval*60000)
    if (timing.mode === 'cron' && !['0 9 * * *','0 9,18 * * 1-5'].includes(timing.cron)) throw new Error('独立预览仅支持示例表达式；实际插件由后端校验')
    const [hour, minute] = timing.daily_time.split(':').map(Number)
    const hours = timing.mode === 'daily' ? [hour] : timing.cron.includes('9,18') ? [9,18] : [9]
    const formatter = new Intl.DateTimeFormat('en-US', {timeZone:timing.timezone,hour:'numeric',minute:'numeric',weekday:'short',hourCycle:'h23'})
    const dates: number[] = []
    const start = Math.floor(Date.now()/60000)*60000+60000
    for (let at=start; at<start+8*86400000 && dates.length<3; at+=60000) {
      const parts = Object.fromEntries(formatter.formatToParts(at).map(part=>[part.type,part.value]))
      if (hours.includes(Number(parts.hour)) && Number(parts.minute)===(timing.mode==='daily'?minute:0) && !(timing.mode==='cron' && timing.cron.endsWith('1-5') && ['Sat','Sun'].includes(parts.weekday!))) dates.push(at)
    }
    return dates
  }
  async function drive(run: Run) {
    if (active.has(run.id)) return
    active.add(run.id)
    try {
      while (run.status === 'ready') {
        await window.codexProxyPlugin!.request({path:'api/runs/step',method:'POST',contentType:'application/json',body:JSON.stringify({id:run.id})})
      }
    } finally { active.delete(run.id) }
  }
  window.codexProxyPlugin = {version:2,async request(input) {
    await new Promise(resolve => setTimeout(resolve,params.get('scenario') === 'loading' ? 12000 : 80))
    const data = input.body ? JSON.parse(input.body) : {}
    const respond=(value: unknown,status=200)=>({status,contentType:'application/json',body:new TextEncoder().encode(JSON.stringify(value)).buffer})
    if (params.get('scenario') === 'error') return respond({message:'模拟：宿主账号目录读取失败，请重试'},503)
    if (input.path === 'api/bootstrap') return respond({maximum_concurrent_accounts:params.get("scenario")==="old-host"?1:3,scheduled_detection_supported:params.get("scenario") !== "old-host",background_supported:true,catalog:methods,sources,accounts:params.get('scenario')==='empty'?[]:previewAccounts,keys:[]})
    if (input.path === 'api/upstreams/check') {
      const source = sources.find(s=>s.method===data.method)!
      const scenario = params.get('updates')
      if (scenario === 'slow') await new Promise(resolve=>setTimeout(resolve,2000))
      if (scenario === 'error' && source.method === 'meow') return respond({message:'GitHub 暂时拒绝检查（HTTP 403，可能触发限流），请稍后重试'},502)
      const status = scenario === 'ahead' || scenario === 'behind' || scenario === 'diverged' ? scenario : 'identical'
      const latest = status === 'identical' ? source.bundled_commit : 'a'.repeat(40)
      return respond({check:{method:source.method,status,checked_at:Date.now(),latest_commit:latest,ahead_by:status==='ahead'||status==='diverged'?2:0,behind_by:status==='behind'||status==='diverged'?1:0,compare_url:`${source.url}/compare/${source.bundled_commit}...${latest}`}})
    }
    if (input.path === 'api/models') return respond({models:['gpt-6-astra','gpt-5.6-sol']})
    if (input.path === 'api/schedule/preview') {
      if (data.timing.mode === 'cron' && (data.timing.cron.trim().split(/\s+/).length !== 5 || data.timing.cron.includes('invalid'))) return respond({message:'Cron 需要五段：分 时 日 月 周，不包含秒或年份'},400)
      try { return respond({next_at:scheduleDates(data.timing,data.interval_minutes),timezone:data.timing.timezone}) }
      catch (cause) { return respond({message:String(cause)},400) }
    }
    if (input.path === 'api/schedule') {
      if (params.get('scenario') === 'schedule-error') return respond({message:'计划读取失败'},503)
      if (input.body) schedule = {concurrency:data.concurrency ?? 1,detection:data.detection || schedule.detection,enabled:data.enabled,model:data.model,interval_minutes:data.interval_minutes,timing:data.timing || schedule.timing,accounts:data.account_ids.map((id:string)=>({account_id:id,name:previewAccounts.find(a=>a.id===id)?.email || id,next_at:scheduleDates(data.timing || schedule.timing,data.interval_minutes)[0]!,status:'waiting',checked_at:null,message:'',pending:null}))}
      return respond(schedule)
    }
    if (input.path === 'api/runs') return respond({runs:runs.map(summary)})
    if (input.path === 'api/runs/start') {
      const selected = methods.methods.find(m=>m.method===data.method)!
      const selectedAccount = previewAccounts.find(account=>account.id===data.account_id) ?? previewAccounts[0]!
      const r=example(`1-${Date.now()}`,selected.tiers.find(t=>t.id===data.tier)!.count)
      Object.assign(r,{created_at:Date.now(),updated_at:Date.now(),status:'ready',attempts:[],report:data.method==='modeltrace'?traceReport(0):report(0,r.plan.jobs.length)})
      r.account_id = selectedAccount.id
      r.account_name = accountDisplayName(selectedAccount)
      Object.assign(r.plan,{method:data.method,engine_version:selected.engine_version,id:selected.id,hash:selected.hash,version:selected.version,tier:data.tier,claimed_model:data.claimed_model,request_model:data.request_model})
      runs.unshift(r);void drive(r);return respond({run:r,background_running:true})
    }
    const r=runs.find(r=>r.id===data.id)
    if (!r) return respond({message:'模拟记录不存在'},404)
    if (input.path === 'api/runs/get') return respond({run:r,busy:active.has(r.id),background_running:active.has(r.id)})
    if (input.path === 'api/runs/control') {r.status=data.action==='pause'?'paused':data.action==='stop'?'stopped':'ready';if (data.action==='resume') void drive(r);return respond({run:r,background_running:active.has(r.id)})}
    if (input.path === 'api/runs/step') {
      const i=r.attempts.length
      await new Promise(resolve=>setTimeout(resolve,700))
      const numeric = r.plan.method==='modeltrace'
      const failed = numeric && params.get('scenario')==='short-answer' && i===0
      r.attempts.push({job_id:r.plan.jobs[i]!.job_id,status:failed?'error':'ok',error:failed?'insufficient_numbers':null,answer:numeric?'147, 23, 311, 88, 204, 17, 265, 134…':'cat',answer_truncated:numeric,request_id:`preview-request-${i+1}`})
      const valid=r.attempts.filter(a=>a.status==='ok').length
      r.report=numeric?traceReport(valid):report(valid,r.plan.jobs.length);r.updated_at=Date.now()
      if (failed) r.status='paused'
      if (r.status==='ready') r.status=i+1===r.plan.jobs.length?'completed':'ready'
      return respond({run:r})
    }
    return respond({message:'未实现的模拟接口'},404)
  }}
}

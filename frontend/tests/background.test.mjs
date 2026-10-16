import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import ts from 'typescript'

function view() {
  let dispose, scheduled, cleared = false
  const calls = []
  const document = { visibilityState: 'visible' }
  const activeRun = { id: '0-1', status: 'ready', attempts: [], plan: {}, report: {} }
  const api = async (path, data) => {
    calls.push({path, data})
    if (path === 'runs') return {runs: []}
    assert.notEqual(path, 'runs/step', 'the page must never drive paid requests')
    return {run: activeRun, background_running: path !== 'runs/control'}
  }
  const modules = {
    vue: { ref: value => ({value}), computed: read => ({get value() {return read()}}), watch: () => {}, onMounted: () => {}, onScopeDispose: fn => {dispose = fn} },
    './api': {api}, '@codex-proxy/ui': {}, '@lucide/vue': {}, './accountDisplay': {}, './presentation': {}, './SourceUpdates.vue': {},
  }
  let source = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8').split('<script setup lang="ts">')[1].split('</script>')[0]
  source = source.replace('import.meta.env.DEV', 'false') + '\nexport {poll, start, control, openRun, run, loading, pumping}'
  const context = { exports: {}, document, require: name => {assert(name in modules, name);return modules[name]}, setTimeout: fn => {scheduled = fn; return 1}, clearTimeout: () => {cleared = true} }
  runInNewContext(ts.transpileModule(source, {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context)
  return { ...context.exports, calls, document, activeRun, dispose: () => dispose(), scheduled: () => scheduled, cleared: () => cleared }
}

test('start enqueues once; polling and disposal never issue step/pause requests', async () => {
  const v = view()
  v.loading.value = false
  await v.start()
  assert.equal(v.calls.filter(c => c.path === 'runs/start').length, 1)
  assert.equal(v.pumping.value, true)
  await v.poll()
  assert(v.calls.some(c => c.path === 'runs/get'))
  const count = v.calls.length
  v.dispose()
  await v.poll()
  assert.equal(v.calls.length, count)
  assert(v.cleared())
  assert(!v.calls.some(c => c.path === 'runs/control'))
})

test('hidden pages stop reads, not jobs; pause is explicit and never retried', async () => {
  const v = view()
  v.loading.value = false
  await v.openRun('0-1')
  v.document.visibilityState = 'hidden'
  const count = v.calls.length
  await v.poll()
  assert.equal(v.calls.length, count)
  await v.control('pause')
  assert.equal(v.calls.filter(c => c.path === 'runs/control').length, 1)
  assert.equal(v.calls.find(c => c.path === 'runs/control').data.action, 'pause')
  v.dispose()
})

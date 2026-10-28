import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'

const source = readFileSync(new URL('../src/accountDisplay.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2024 },
})
const { accountDisplayName } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`)

test('same name accounts use different emails as display names', () => {
  const name = 'OPENAI账号'
  assert.equal(accountDisplayName({ name, email: 'a@example.com' }), 'a@example.com')
  assert.equal(accountDisplayName({ name, email: 'b@example.com' }), 'b@example.com')
})

test('missing email falls back to trimmed name', () => {
  assert.equal(accountDisplayName({ name: '  备用账号  ', email: null }), '备用账号')
  assert.equal(accountDisplayName({ name: '主账号', email: '   ' }), '主账号')
})

test('stopped scheduling accounts are clearly labelled without hiding their identity', () => {
  assert.equal(accountDisplayName({ name: '备用账号', email: 'a@example.com', enabled: false }), 'a@example.com（已停止调度）')
  assert.equal(accountDisplayName({ name: '备用账号', enabled: false }), '备用账号（已停止调度）')
  assert.equal(accountDisplayName({ name: '主账号', enabled: true }), '主账号')
})

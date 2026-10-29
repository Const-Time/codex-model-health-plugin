import assert from 'node:assert/strict'
import test from 'node:test'
import { requestModels } from '../src/requestModels.ts'

test('request choices include 6.1 without changing or duplicating detector baselines', () => {
  const baselines = ['gpt-6-astra', 'gpt-6.1-sol']
  assert.deepEqual(requestModels(baselines), ['gpt-6.1-sol', 'gpt-6.1-terra', 'gpt-6-astra'])
  assert.deepEqual(baselines, ['gpt-6-astra', 'gpt-6.1-sol'])
})

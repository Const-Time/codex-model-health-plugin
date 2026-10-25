<script setup lang="ts">
import { computed, ref } from 'vue'
import { BaseInput, BaseSelect } from '@codex-proxy/ui'
const props = defineProps<{ models: string[]; label: string; disabled?: boolean }>()
const model = defineModel<string>({ required: true })
const custom = ref(false)
const customValue = '__custom_model__'
const options = computed(() => [
  ...[...new Set(props.models)].map(value => ({ value, label: value })),
  { value: customValue, label: '自定义模型…' },
])
const selection = computed({
  get: () => (!model.value && custom.value) || (model.value && !props.models.includes(model.value)) ? customValue : model.value,
  set: (value: string) => { custom.value = value === customValue; model.value = custom.value ? '' : value },
})
</script>
<template>
  <div class="field">
    <BaseSelect v-model="selection" :aria-label="label" :options="options" :disabled="disabled" filterable placeholder="选择请求模型" />
    <BaseInput v-if="selection === customValue" v-model="model" :aria-label="`${label}（自定义）`" :disabled="disabled" maxlength="128" placeholder="填写完整模型 ID" />
  </div>
</template>

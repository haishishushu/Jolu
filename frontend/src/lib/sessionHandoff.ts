export interface HandoffMeta {
  version: number; covered_to: number; covered_range: [number, number] | null; focus: string; target: string
  draft_rounds: number; path_warnings: string[]; executor_platform: string | null; executor_connection_id: string | null
  model: string | null; input_tokens: number | null; output_tokens: number | null; calls: number | null; created_ms: number
}
export interface HandoffState {
  readable: boolean; total: number; latest: HandoffMeta | null; digest_covered_to: number
  digest_range: [number, number] | null; new_range: [number, number] | null; reusable: boolean; estimate_tokens: number
}
export interface Draft { round: number; narrative_md: string; path_warnings: string[]; input_tokens: number | null; output_tokens: number | null }
export type ChatEventDto = { type: 'step'; text: string; done: number; total: number } | ({ type: 'draft' } & Draft) | { type: 'error'; text: string; retryable: boolean } | { type: 'digest_saved' }
export interface ChatItem { kind: 'step' | 'draft' | 'user' | 'error'; text: string; round?: number; warnings?: string[]; retryable?: boolean; done?: number; total?: number }
export interface HandoffResult { version: number; markdown: string; file_name: string; path_warnings: string[]; covered_range: [number, number] | null }
const safePart = (s: string) => s.trim().replace(/[<>:"/\\|?*\x00-\x1f]/g, '-').replace(/[. ]+$/, '') || '交接'
export function handoffFileName(project: string, topic: string, target: string, at: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${safePart(project)}_${safePart(topic || '继续当前开发')}_交接-${safePart(target)}_${at.getFullYear()}${pad(at.getMonth()+1)}${pad(at.getDate())}-${pad(at.getHours())}${pad(at.getMinutes())}.md`
}
export function dedupeFileName(name: string, exists: (name: string) => boolean): string {
  if (!exists(name)) return name
  const stem = name.replace(/\.md$/i, '')
  let n = 2
  while (exists(`${stem}-${n}.md`)) n++
  return `${stem}-${n}.md`
}
export function fileNameError(name: string): string | null {
  if (!name.trim() || name.trim() === '.md') return '请输入文件名'
  if (/[<>:"/\\|?*\x00-\x1f]/.test(name) || /[. ]$/.test(name) || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name)) return '文件名包含非法字符或系统保留名称'
  if ([...name].length > 120) return '文件名不能超过 120 个字符'
  if (!name.toLowerCase().endsWith('.md')) return '扩展名必须为 .md'
  return null
}
export function dialogMode(state: Pick<HandoffState, 'readable' | 'reusable' | 'latest'>): 'first' | 'incremental' | 'reuse' | 'unreadable' {
  return state.reusable ? 'reuse' : !state.readable ? 'unreadable' : state.latest ? 'incremental' : 'first'
}
export function reduceChat(items: ChatItem[], event: ChatEventDto): ChatItem[] {
  if (event.type === 'digest_saved') return items
  if (event.type === 'draft') return [...items, { kind:'draft', text:event.narrative_md, round:event.round, warnings:event.path_warnings }]
  const item: ChatItem = event.type === 'step' ? {kind:'step', text:event.text, done:event.done, total:event.total} : {kind:'error',text:event.text,retryable:event.retryable}
  const last = items.at(-1)
  return event.type === 'step' && last?.kind === 'step' && last.text === event.text ? [...items.slice(0,-1),item] : [...items,item]
}
export const formatTokens = (n: number) => n < 1000 ? String(n) : `${(n / 1000).toFixed(1)}k`
export const HANDOFF_PROMPT = '请先阅读这份交接文档，核对项目当前文件与验证状态。简述你理解的进度，然后从“下一步建议”继续；不要重复已完成的工作。'
export const HANDOFF_TARGETS = ['Claude Code','Codex','Gemini','Grok','OpenCode','Qoder','Trae','ZCode','WorkBuddy','其他 AI']

import { invoke } from './api'
import type { HandoffState, HandoffResult } from './sessionHandoff'
export const handoffApi = {
  state: (platform: string, sessionId: string) => invoke<HandoffState>('handoff_state', {platform, sessionId, focus:null}),
  start: (platform: string, sessionId: string, connectionId: string, model: string | null, focus: string, target: string) => invoke<string>('handoff_start', {platform, sessionId, connectionId, model, focus, target}),
  refine: (jobId: string, instruction: string) => invoke<void>('handoff_refine', {jobId, instruction}),
  estimate: (jobId: string, instruction: string) => invoke<number>('handoff_estimate_refine', {jobId, instruction}),
  finish: (jobId: string, round: number, project: string) => invoke<HandoffResult>('handoff_finish', {jobId, round, project}),
  cancel: (jobId: string) => invoke<void>('handoff_cancel', {jobId}),
  render: (platform: string, sessionId: string, project: string, topic: string, target: string) => invoke<string>('handoff_render', {platform, sessionId, project, topic, target}),
  save: (markdown: string, fileName: string, preferProjectDir: string | null) => invoke<string | null>('handoff_save', {markdown, fileName, preferProjectDir}),
}

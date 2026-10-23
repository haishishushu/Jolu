import { useState } from 'react'
import { HANDOFF_PROMPT, fileNameError, type HandoffResult, type HandoffState } from '@/lib/sessionHandoff'
import { handoffApi } from '@/lib/handoffApi'
import { isTauri, type SessionItemDto } from '@/lib/api'
import { MarkdownPreview } from '@/components/skills/skillParts'
import { useToast } from '@/components/ui/Toast'
import { HandoffNotice, handoffButton, handoffInput, handoffPrimary } from './handoffParts'
export function HandoffPreview({result, fileName, target, session, state, onClose, onUpdate}: {result:HandoffResult;fileName:string;target:string;session:SessionItemDto;state:HandoffState|null;onClose:()=>void;onUpdate:()=>void}) {
  const [markdown,setMarkdown]=useState(result.markdown)
  const [name,setName]=useState(fileName)
  const [source,setSource]=useState(false)
  const [projectDir,setProjectDir]=useState(false)
  const [busy,setBusy]=useState(false)
  const [error,setError]=useState('')
  const toast=useToast()
  const copy=(text:string)=>void navigator.clipboard.writeText(text).then(()=>toast.success('已复制'),()=>toast.danger('复制失败'))
  const save=async()=>{setBusy(true);setError('');try{if(!isTauri)throw new Error('浏览器预览不保存文件');const path=await handoffApi.save(markdown,name,projectDir?session.cwd:null);if(path)toast.success('已保存',path)}catch(e){setError(String(e))}finally{setBusy(false)}}
  const extra=state?Math.max(0,state.total-(result.covered_range?.[1]??0)):0
  return <section className="absolute inset-0 z-20 flex min-h-0 flex-col gap-4 overflow-auto bg-jolu-bg p-5">
    <header className="flex items-center justify-between"><div><h2 className="text-lg font-semibold text-jolu-text">交接文档已生成</h2><p className="text-xs text-jolu-muted">{session.title??session.session_id}</p></div><button className={handoffButton} onClick={onClose}>返回会话</button></header>
    <HandoffNotice>已生成完整交接文档 · 覆盖第 {result.covered_range?.join('–')??'—'} 条。可交给新的 AI 接着开发，不需要附带上一份摘要。{extra>0&&<p>生成期间又新增 {extra} 条，未包含在本次文档中。<button className={handoffButton} onClick={onUpdate}>更新压缩</button></p>}</HandoffNotice>
    {!!result.path_warnings.length&&<HandoffNotice danger>以下路径未在会话中出现：{result.path_warnings.join('、')}</HandoffNotice>}
    <div className="flex gap-4"><label className="flex-1 text-xs text-jolu-muted">文件名<input className={handoffInput} value={name} onChange={e=>setName(e.target.value)}/></label><div className="text-xs text-jolu-muted">交接对象<p className="mt-2 text-jolu-text">{target}</p></div></div>
    {fileNameError(name)&&<p className="text-xs text-jolu-danger">{fileNameError(name)}</p>}
    <div className="grid min-h-[300px] flex-1 grid-cols-[minmax(0,1fr)_230px] gap-4">
      <div className="flex min-h-0 flex-col rounded-xl border border-jolu-border"><div className="flex gap-4 border-b border-jolu-border p-3">{['预览','Markdown 源码'].map((label,i)=><button key={label} className={`text-xs ${source===(i===1)?'text-jolu-on':'text-jolu-muted'}`} onClick={()=>setSource(i===1)}>{label}</button>)}</div><div className="min-h-0 flex-1 overflow-auto p-4">{source?<textarea aria-label="Markdown 源码" value={markdown} onChange={e=>setMarkdown(e.target.value)} className={handoffInput+' h-full min-h-[260px] font-mono'}/>:<MarkdownPreview source={markdown} stripFront={false}/>}</div></div>
      <aside className="space-y-4 text-xs"><div className="rounded-xl border border-jolu-border p-4 text-jolu-muted"><h3 className="mb-3 font-medium text-jolu-text">本次压缩记录</h3><p>摘要版本：第 {result.version} 版</p><p>当前覆盖：第 {result.covered_range?.join('–')??'—'} 条</p><p>执行 AI：{state?.latest?.executor_platform??'—'}</p><p>模型：{state?.latest?.model??'默认模型'}</p><p>输入 / 输出：{state?.latest?.input_tokens??'—'} / {state?.latest?.output_tokens??'—'}</p><p>生成轮次：{state?.latest?.draft_rounds??'—'}</p><p>模型用量不并入用量统计</p></div><div className="rounded-xl border border-jolu-border p-4 text-jolu-muted"><h3 className="mb-3 font-medium text-jolu-text">交给另一个 AI</h3><p className="leading-6">{HANDOFF_PROMPT}</p><button className={handoffButton+' mt-3'} onClick={()=>copy(HANDOFF_PROMPT)}>复制交接指令</button></div></aside>
    </div>
    {error&&<HandoffNotice danger>{error}</HandoffNotice>}
    <footer className="flex items-center gap-2"><label className="mr-auto flex gap-2 text-xs text-jolu-muted"><input type="checkbox" disabled={!session.cwd} checked={projectDir} onChange={e=>setProjectDir(e.target.checked)}/>项目目录</label><button className={handoffButton} onClick={()=>copy(markdown)}>复制 Markdown</button><button className={handoffPrimary} disabled={busy||!!fileNameError(name)} onClick={()=>void save()}>{busy?'正在保存…':'保存 MD'}</button></footer>
  </section>
}

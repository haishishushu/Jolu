import { useEffect, useRef, useState } from 'react'
import { type ChatItem, type Draft, formatTokens } from '@/lib/sessionHandoff'
import { MarkdownPreview } from '@/components/skills/skillParts'
import { HandoffNotice, handoffButton, handoffInput, handoffPrimary } from './handoffParts'
export function CompressChat({ items, busy, drafts, round, onRound, onSend, onFinish, onCancel, onRetry, onChangeConnection, estimate, onInstruction }: {
  items:ChatItem[];busy:boolean;drafts:Draft[];round:number;onRound:(round:number)=>void;onSend:(text:string)=>void;onFinish:()=>void;onCancel:()=>void;onRetry:()=>void;onChangeConnection:()=>void;estimate:number|null;onInstruction:(text:string)=>void
}) {
  const [text,setText]=useState('')
  const scroll=useRef<HTMLDivElement>(null)
  const pinned=useRef(true)
  useEffect(()=>{if(pinned.current && scroll.current) scroll.current.scrollTop=scroll.current.scrollHeight},[items])
  const last=drafts.at(-1)
  const limited=(last?.round??0)>=11
  return <>
    <div ref={scroll} onScroll={()=>{const el=scroll.current;if(el)pinned.current=el.scrollHeight-el.scrollTop-el.clientHeight<60}} className="min-h-0 flex-1 space-y-3 overflow-y-auto border-y border-jolu-border py-4" aria-live="polite">
      {items.map((item,i)=><div key={i} className="flex gap-3 text-xs"><span className="w-8 shrink-0 text-jolu-muted">{item.kind==='user'?'你':item.kind==='draft'?'AI':'Jolu'}</span><div className="min-w-0 flex-1">
        {item.kind==='draft'?<details className="rounded-lg border border-jolu-border p-3"><summary className="cursor-pointer font-medium text-jolu-text">交接草稿 v{item.round}（约 {item.text.length} 字） · 展开全文</summary><MarkdownPreview source={item.text} stripFront={false}/></details>:<p className={item.kind==='error'?'text-jolu-danger':'whitespace-pre-wrap leading-6 text-jolu-text'}>{item.text}{item.total?`（${item.done}/${item.total}）`:''}</p>}
        {item.kind==='draft'&&<><p className="mt-2 line-clamp-3 whitespace-pre-wrap text-jolu-muted">{item.text.split(/(?=^## )/m).slice(0,3).join('').slice(0,320)}</p><label className="mt-2 flex gap-2 text-jolu-on"><input type="radio" name="draft-version" checked={round===item.round} disabled={busy} onChange={()=>onRound(item.round!)} />使用 v{item.round}{item.round===last?.round?' · 最新':''}</label></>}
        {!!item.warnings?.length&&<HandoffNotice danger>以下路径未在会话中出现：{item.warnings.join('、')}</HandoffNotice>}
        {item.kind==='error'&&!busy&&<div className="mt-2 flex gap-2">{item.retryable&&<button className={handoffButton} onClick={onRetry}>重试</button>}<button className={handoffButton} onClick={onChangeConnection}>更换连接</button></div>}
      </div></div>)}
      {busy&&<p className="text-xs text-jolu-muted">正在整理交接草稿…</p>}
    </div>
    <form className="flex gap-2" onSubmit={e=>{e.preventDefault();if(text.trim()&&!busy&&!limited){onSend(text.trim());setText('')}}}>
      <textarea aria-label="修改意见" className={handoffInput} rows={2} disabled={busy||limited||!last} placeholder={busy?'生成中，完成后可继续提意见':limited?'已达到修改上限':'对草稿提修改意见…'} value={text} onChange={e=>{setText(e.target.value);onInstruction(e.target.value)}}/>
      <button className={handoffPrimary} disabled={busy||limited||!last||!text.trim()}>发送</button>
    </form>
    <p className="text-[11px] text-jolu-muted">本轮预计发送约 {estimate===null?'—':formatTokens(estimate)} token · 已用 输入 {last?.input_tokens==null?'—':formatTokens(last.input_tokens)} / 输出 {last?.output_tokens==null?'—':formatTokens(last.output_tokens)} token</p>
    <div className="flex items-center gap-2"><p className="flex-1 text-[10px] text-jolu-muted">取消保留旧摘要与原会话；已发出的模型请求可能产生用量。</p><button className={handoffButton} onClick={onCancel}>取消压缩</button><button className={handoffPrimary} disabled={busy||!round} onClick={onFinish}>用 v{round||1} 生成 MD</button></div>
  </>
}

import { SelectField } from "@/components/ui/SelectField"
import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { isTauri, type SessionItemDto } from '@/lib/api'
import { automationApi, type AutomationAccountDto } from '@/lib/automationApi'
import { handoffApi } from '@/lib/handoffApi'
import { dialogMode, handoffFileName, fileNameError, formatTokens, HANDOFF_TARGETS, reduceChat, type ChatEventDto, type ChatItem, type Draft, type HandoffResult, type HandoffState } from '@/lib/sessionHandoff'
import { projectName, sessionTitle } from '@/lib/sessionThreads'
import { HandoffShell, HandoffNotice, handoffButton, handoffInput, handoffPrimary } from './handoffParts'
import { CompressChat } from './CompressChat'
type Envelope={job_id:string;event:ChatEventDto}
export function CompressDialog({session,onClose,onResult}: {session:SessionItemDto;onClose:()=>void;onResult:(result:HandoffResult,name:string,target:string)=>void}) {
  const [state,setState]=useState<HandoffState|null>(null)
  const [accounts,setAccounts]=useState<(AutomationAccountDto&{platform:string})[]>([])
  const [connection,setConnection]=useState('')
  const [model,setModel]=useState('')
  const [models,setModels]=useState<string[]>([])
  const [target,setTarget]=useState('Claude Code')
  const [focus,setFocus]=useState('')
  const project=projectName(session.cwd)
  const [name,setName]=useState(()=>handoffFileName(project,'','Claude',new Date()))
  const manualName=useRef(false)
  const [error,setError]=useState('')
  const [chat,setChat]=useState(false)
  const [busy,setBusy]=useState(false)
  const [confirm,setConfirm]=useState(false)
  const [items,setItems]=useState<ChatItem[]>([])
  const [drafts,setDrafts]=useState<Draft[]>([])
  const [round,setRound]=useState(0)
  const [estimate,setEstimate]=useState<number|null>(null)
  const job=useRef<string|null>(null)
  const buffer=useRef<Envelope[]>([])
  const starting=useRef(false)
  const alive=useRef(true)
  const ready=useRef<Promise<()=>void>|null>(null)
  const estimateRevision=useRef(0)
  const instruction=useRef('')
  const accept=(event:ChatEventDto)=>{
    setItems(value=>reduceChat(value,event))
    if(event.type==='draft'){setDrafts(value=>[...value,event]);setRound(event.round);setBusy(false);setEstimate(null)}
    if(event.type==='error')setBusy(false)
  }
  const reload=async()=>{setError('');try{setState(await handoffApi.state(session.platform,session.session_id))}catch(e){setError(String(e))}}
  useEffect(()=>{
    alive.current=true
    if(isTauri){
      void reload()
      void Promise.all((['claude','codex'] as const).map(async platform=>(await automationApi.accounts(platform)).map(a=>({...a,platform})))).then(rows=>{if(!alive.current)return;const all=rows.flat();setAccounts(all);setConnection(all.find(a=>a.current&&a.status==='connected')?.id??'')}).catch(e=>{if(alive.current)setError(String(e))})
      ready.current=listen<Envelope>('handoff-chat',({payload})=>{if(!alive.current)return;if(payload.job_id===job.current)accept(payload.event);else if(starting.current)buffer.current.push(payload)})
    }else{
      setState({readable:session.has_source,total:24,latest:null,digest_covered_to:0,digest_range:null,new_range:[1,24],reusable:false,estimate_tokens:12400})
    }
    return ()=>{alive.current=false;void ready.current?.then(stop=>stop()).catch(()=>{});if(job.current)void handoffApi.cancel(job.current).catch(()=>{})}
  },[session.platform,session.session_id])
  useEffect(()=>{if(!manualName.current)setName(handoffFileName(project,focus,target==='Claude Code'?'Claude':target,new Date()))},[project,focus,target])
  const selected=accounts.find(a=>a.id===connection)
  useEffect(()=>{let active=true;setModel('');setModels([]);if(selected&&isTauri)void automationApi.models(selected.platform as 'claude'|'codex',selected.id).then(values=>{if(active)setModels(values)}).catch(e=>{if(active)setError(String(e))});return()=>{active=false}},[connection])
  const mode=state?dialogMode(state):null
  const close=()=>chat?setConfirm(true):onClose()
  const cancel=async()=>{try{if(job.current)await handoffApi.cancel(job.current);job.current=null;onClose()}catch(e){setError(String(e))}}
  const start=async()=>{
    if(!state||!isTauri)return
    setBusy(true);setError('')
    try{
      if(mode==='reuse'){
        const markdown=await handoffApi.render(session.platform,session.session_id,project,focus,target)
        onResult({version:state.latest!.version,markdown,file_name:name,path_warnings:state.latest!.path_warnings,covered_range:state.latest!.covered_range},name,target)
        return
      }
      await ready.current
      if(job.current)await handoffApi.cancel(job.current)
      job.current=null;buffer.current=[];starting.current=true;setItems([]);setDrafts([]);setRound(0);setChat(true);instruction.current=''
      const id=await handoffApi.start(session.platform,session.session_id,connection,model||null,focus,target)
      if(!alive.current){await handoffApi.cancel(id);return}
      job.current=id;starting.current=false
      for(const event of buffer.current)if(event.job_id===id)accept(event.event)
      buffer.current=[]
    }catch(e){starting.current=false;setError(String(e));setBusy(false);setItems(value=>[...value,{kind:'error',text:String(e),retryable:true}])}
  }
  const refine=async(text:string)=>{if(!job.current)return;instruction.current=text;setBusy(true);setItems(v=>[...v,{kind:'user',text}]);try{await handoffApi.refine(job.current,text)}catch(e){setBusy(false);setItems(v=>[...v,{kind:'error',text:String(e),retryable:true}])}}
  const finish=async()=>{if(!job.current)return;setBusy(true);try{const result=await handoffApi.finish(job.current,round,project);job.current=null;onResult(result,name,target)}catch(e){setError(String(e));setBusy(false)}}
  const estimateInput=(text:string)=>{const revision=++estimateRevision.current;setEstimate(null);if(job.current&&text.trim())void handoffApi.estimate(job.current,text).then(n=>{if(alive.current&&revision===estimateRevision.current)setEstimate(n)}).catch(()=>{})}
  return <HandoffShell title={chat?`压缩会话 · ${sessionTitle(session)}`:mode==='reuse'?'会话已压缩，直接生成 MD':'压缩会话'} onClose={close} chat={chat}>
    {confirm?<><HandoffNotice>关闭压缩会话？关闭将丢弃未生成的草稿，已整理的纪要会保留。</HandoffNotice><div className="flex justify-end gap-2"><button className={handoffButton} onClick={()=>setConfirm(false)}>继续压缩</button><button className={handoffPrimary} onClick={()=>void cancel()}>丢弃并关闭</button></div></>:chat?<>
      <p className="text-xs text-jolu-muted">{project} · 共 {state?.total??'—'} 条 · 执行：{selected?.name??'—'} · {model||'默认模型'}</p>
      <CompressChat items={items} busy={busy} drafts={drafts} round={round} onRound={setRound} onSend={text=>void refine(text)} onFinish={()=>void finish()} onCancel={close} onRetry={()=>void(instruction.current&&job.current?refine(instruction.current):start())} onChangeConnection={()=>{setChat(false);setError('')}} estimate={estimate} onInstruction={estimateInput}/>
    </>:<>
      <p className="text-xs text-jolu-muted">把开发进度整理成 Markdown，让另一个 AI 读完即可接着工作。</p>
      <div className="rounded-lg bg-jolu-side p-3 text-xs text-jolu-text">{sessionTitle(session)}<p className="mt-2 text-jolu-muted">{project} · {state?.total??'—'} 条消息 · {session.session_id}</p></div>
      {!state?<p className="text-xs text-jolu-muted">正在读取…</p>:<HandoffNotice>
        {mode==='reuse'?'没有新增消息 · 无需再次调用 AI':mode==='unreadable'?'仅用量记录 / 源文件不可读':mode==='first'?'还没有有效摘要':'本次增量压缩'}
        <p>{mode==='unreadable'?'无法仅凭 Token 数与会话标题生成可靠的开发交接。':mode==='reuse'?'本次只调整文档包装，主体事实和任务进度沿用有效摘要。':`本次整理第 ${state.new_range?.join('–')??`1–${state.total}`} 条消息，已有纪要覆盖 ${state.digest_covered_to} 条。`}</p>
      </HandoffNotice>}
      {mode!=='reuse'&&mode!=='unreadable'&&<><div className="grid grid-cols-2 gap-4"><label className="text-xs text-jolu-muted">执行压缩的 AI<SelectField label="执行压缩的 AI" className="mt-1.5 h-10 w-full" value={connection} onValueChange={setConnection} options={[{value:'',label:'选择可用连接'},...accounts.map(a=>({value:a.id,label:a.name,disabled:!a.current||a.status!=='connected',description:!a.current?'需为 CLI 当前登录账号':a.status!=='connected'?'连接不可用':undefined}))]} /></label><label className="text-xs text-jolu-muted">压缩模型<SelectField label="压缩模型" className="mt-1.5 h-10 w-full" value={model} onValueChange={setModel} options={[{value:'',label:'默认模型'},...models.map(id=>({value:id,label:id}))]} /></label></div><p className="text-[11px] leading-5 text-jolu-muted">预计发送约 {formatTokens(state?.estimate_tokens??0)} token · 将会话内容发送给所选 AI。没有可用连接时暂不能压缩新增消息，保留上次摘要。</p></>}
      <div className="grid grid-cols-2 gap-4"><label className="text-xs text-jolu-muted">交接给<SelectField label="交接给" className="mt-1.5 h-10 w-full" value={target} onValueChange={setTarget} options={HANDOFF_TARGETS.map(t=>({value:t,label:t}))} /></label><label className="text-xs text-jolu-muted">交接主题<input className={handoffInput} value={focus} onChange={e=>setFocus(e.target.value)} placeholder="接手后要做什么（可留空）"/></label></div>
      <label className="text-xs text-jolu-muted">文件名（可修改）<input className={handoffInput} value={name} onChange={e=>{manualName.current=true;setName(e.target.value)}}/></label>
      {fileNameError(name)&&<p className="text-xs text-jolu-danger">{fileNameError(name)}</p>}
      <div className="rounded-lg border border-jolu-border p-4 text-xs leading-6 text-jolu-muted"><p className="font-medium text-jolu-text">这份 MD 会包含</p>项目目标与约束 · 当前开发进度 · 已完成改动<br/>关键文件与决策 · 验证结果 · 待办与下一步执行建议</div>
      <p className="text-xs text-jolu-muted">原始会话保持完整；交接文档按当前目标整理，过时结论以新消息为准。</p>
      {!isTauri&&<HandoffNotice>浏览器预览不调用模型；请在桌面应用中执行压缩。</HandoffNotice>}
      <div className="flex justify-end gap-2">{(mode==='unreadable'||!!error)&&<button className={handoffButton} onClick={()=>void reload()}>重新读取会话</button>}<button className={handoffButton} onClick={onClose}>取消</button><button className={handoffPrimary} disabled={busy||!isTauri||!state||mode==='unreadable'||!!fileNameError(name)||(mode!=='reuse'&&(!selected?.current||selected.status!=='connected'))} onClick={()=>void start()}>{mode==='reuse'?'直接生成 MD':'压缩并生成 MD'}</button></div>
    </>}
    {error&&<HandoffNotice danger>{error}</HandoffNotice>}
  </HandoffShell>
}

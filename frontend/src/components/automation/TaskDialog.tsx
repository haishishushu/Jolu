import * as Dialog from "@radix-ui/react-dialog"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { createPortal } from "react-dom"
import {
  Check, ChevronDown, ChevronRight, ChevronUp, Circle, CircleCheck, CircleDot, FileText, FolderOpen, Info, LoaderCircle, MessageSquareText, MessagesSquare, Plus, Search, X,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type SessionItemDto } from "@/lib/api"
import {
  WEEKDAYS, accountText, dayTime, nextFire, projectOf, quotaTip, sessionTaskName, shortId, toSchedule, toolName, validateForm, type FormErrors, type TaskForm,
} from "@/lib/automation"
import {
  automationApi, type AutomationAccountDto, type AutomationPlatform, type QuotaVerdictDto, type TaskDto, type TaskInputDto, type ToolInfoDto,
} from "@/lib/automationApi"
import { sessionTitle } from "@/lib/sessionThreads"
import { dateKey, localZone, zoneName } from "@/lib/usageStats"
import { DEMO_MODELS, demoAccounts, demoQuota } from "@/mock/automation"
import { demoSessions } from "@/mock/sessionThreads"
import { Tip } from "@/components/ui/Tip"
import { JoluButton } from "@/components/ui/JoluButton"
import { Notice } from "./automationParts"
import { SkillPicker } from "./SkillPicker"

export interface TaskDialogRequest {
  mode: "create" | "edit"
  task?: TaskDto
  /** 来自会话线程的新建（画板 72） */
  session?: SessionItemDto
  nonce: number
}

const PLATFORMS: AutomationPlatform[] = ["claude", "codex"]
const INPUT = "flex h-[38px] w-full items-center gap-2.5 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs text-jolu-text outline-none focus-within:border-jolu-accent focus-visible:border-jolu-accent"
const MENU_CLASS = "motion-popover z-[120] min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[360px] overflow-y-auto rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover"
const ITEM_CLASS = "relative flex min-h-8 cursor-default select-none items-center gap-2 rounded-md py-1.5 pl-2 pr-7 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40 data-[state=checked]:text-jolu-on"

function samePath(a: string | null | undefined, b: string | null | undefined): boolean {
  const n = (p: string | null | undefined) => (p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase()
  return !!n(a) && n(a) === n(b)
}

function initialForm(req: TaskDialogRequest): TaskForm {
  const today = dateKey(new Date())
  const base: TaskForm = {
    name: "", platform: "claude", connectionId: "", model: "", cwd: "", sessionId: null, prompt: "", launch: "now",
    freq: "daily", time: "09:00", weekday: 1, date: today, timezone: localZone(), autoResume: true, deadline: "", serialQueue: true, skills: [],
  }
  if (req.task) {
    const t = req.task
    const pad = (n: number) => String(n).padStart(2, "0")
    const deadline = t.wait_deadline ? (() => { const d = new Date(t.wait_deadline); return `${dateKey(d)}T${pad(d.getHours())}:${pad(d.getMinutes())}` })() : ""
    return {
      ...base, name: t.name, platform: t.platform, connectionId: t.connection_id, model: t.model ?? "", cwd: t.cwd, sessionId: t.session_id,
      prompt: t.prompt, launch: t.launch, freq: t.schedule?.freq ?? "daily", time: t.schedule?.time ?? "09:00", weekday: t.schedule?.weekday ?? 1,
      date: t.schedule?.date ?? today, autoResume: t.auto_resume, deadline, serialQueue: t.serial_queue, skills: t.skills,
    }
  }
  if (req.session) {
    const s = req.session
    const platform: AutomationPlatform = s.platform === "codex" ? "codex" : "claude"
    return { ...base, name: sessionTaskName(s.title), platform, connectionId: s.connection_id ?? "", cwd: s.cwd ?? "", sessionId: s.session_id }
  }
  return base
}

function Field({ label, error, note, children }: { label: string; error?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-[7px]">
      <span className="text-[11px] text-jolu-muted">{label}</span>
      {children}
      {error ? <span role="alert" className="text-[10px] text-jolu-danger">{error}</span> : note && <span className="text-[10px] text-jolu-muted">{note}</span>}
    </div>
  )
}

interface Option { value: string; label: string; note?: string; disabled?: boolean; tag?: string }

function FieldSelect({ value, options, onChange, placeholder, label, disabled, invalid }: {
  value: string
  options: Option[]
  onChange: (v: string) => void
  placeholder: string
  label: string
  disabled?: boolean
  invalid?: boolean
}) {
  const selected = options.find((o) => o.value === value)
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild disabled={disabled}>
        <button type="button" aria-label={label} className={cn(INPUT, "justify-between text-left disabled:cursor-not-allowed disabled:opacity-45 data-[state=open]:border-jolu-accent", invalid && "border-jolu-danger")}>
          <span className={cn("truncate", !selected && "text-jolu-muted")}>{selected?.label ?? placeholder}</span>
          <ChevronDown aria-hidden className="size-[15px] shrink-0 text-jolu-muted" />
        </button>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className={MENU_CLASS} style={{ maxHeight: "min(300px, var(--radix-dropdown-menu-content-available-height))" }}>
          {options.length ? (
            <Menu.RadioGroup value={value} onValueChange={onChange}>
              {options.map((o) => (
                <Menu.RadioItem key={o.value} value={o.value} disabled={o.disabled} className={ITEM_CLASS}>
                  <span className="flex min-w-0 flex-col">
                    <span className="flex items-center gap-1.5">
                      <span className="truncate">{o.label}</span>
                      {o.tag && <span className="rounded-[4px] bg-jolu-selected px-1.5 text-[10px] text-jolu-on">{o.tag}</span>}
                    </span>
                    {o.note && <span className="truncate text-[10px] text-jolu-muted">{o.note}</span>}
                  </span>
                  <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3.5" /></Menu.ItemIndicator>
                </Menu.RadioItem>
              ))}
            </Menu.RadioGroup>
          ) : <div className="px-2 py-1.5 text-xs text-jolu-muted">没有可选项</div>}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

function Checkbox({ checked, disabled, onChange, title, note }: { checked: boolean; disabled?: boolean; onChange?: (v: boolean) => void; title: string; note?: string }) {
  return (
    <button type="button" role="checkbox" aria-checked={checked} disabled={disabled} onClick={() => onChange?.(!checked)} className="flex items-start gap-2.5 rounded-md text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed">
      <span className={cn("mt-px grid size-4 shrink-0 place-items-center rounded-[4px]", checked ? "bg-jolu-accent" : "border border-jolu-border bg-jolu-bg", disabled && "opacity-60")}>
        {checked && <Check aria-hidden className="size-3 text-[#071B10]" />}
      </span>
      <span className="flex min-w-0 flex-col gap-1">
        <span className="text-xs text-jolu-text">{title}</span>
        {note && <span className="text-[10px] text-jolu-muted">{note}</span>}
      </span>
    </button>
  )
}

/**
 * 新建 / 编辑任务弹窗（画板 60、70、72）：居中弹窗，标题与底部操作栏固定、表单区单独滚动；执行工具与账号、模型与项目、执行会话、执行内容、
 * Skills（可选）、启动方式与定时 / 额度提示、自动续跑与高级设置。
 */
export function TaskDialog({ request, onClose, onSaved, onBackToSession }: {
  request: TaskDialogRequest
  onClose: () => void
  onSaved: (id: number, launch: TaskInputDto["launch"]) => void
  onBackToSession: (session: SessionItemDto) => void
}) {
  const [form, setForm] = useState<TaskForm>(() => initialForm(request))
  const [submitted, setSubmitted] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [accounts, setAccounts] = useState<AutomationAccountDto[] | null>(null)
  const [models, setModels] = useState<string[]>([])
  const [tools, setTools] = useState<ToolInfoDto[] | null>(null)
  const [quota, setQuota] = useState<QuotaVerdictDto | null>(null)
  const [quotaLoading, setQuotaLoading] = useState(false)
  const [advanced, setAdvanced] = useState(false)
  const [pickerOpen, setPickerOpen] = useState(false)
  const [skillsOpen, setSkillsOpen] = useState(false)
  /** 会话下拉挂在弹窗根节点上定位，不被表单滚动区裁切 */
  const [root, setRoot] = useState<HTMLDivElement | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const now = Date.now()
  const set = <K extends keyof TaskForm>(key: K, value: TaskForm[K]) => setForm((f) => ({ ...f, [key]: value }))
  const fromSession = request.mode === "create" && !!request.session

  useEffect(() => {
    if (!isTauri) return setTools(PLATFORMS.map((platform) => ({ platform, found: true })))
    automationApi.tools().then(setTools, () => setTools(null))
  }, [])

  useEffect(() => {
    let alive = true
    setAccounts(null)
    const load = isTauri ? automationApi.accounts(form.platform) : Promise.resolve(demoAccounts(form.platform))
    load.then((list) => {
      if (!alive) return
      setAccounts(list)
      // 新建时默认选中该工具当前登录的账号；来自会话且未归属账号时保持未选（画板 72）
      setForm((f) => (f.connectionId || request.mode === "edit" || fromSession ? f : { ...f, connectionId: list.find((a) => a.current)?.id ?? "" }))
    }, () => alive && setAccounts([]))
    return () => { alive = false }
  }, [form.platform, request.mode, fromSession])

  useEffect(() => {
    if (!form.connectionId) return setModels([])
    let alive = true
    const load = isTauri ? automationApi.models(form.platform, form.connectionId) : Promise.resolve(DEMO_MODELS[form.platform])
    load.then((list) => alive && setModels(list), () => alive && setModels([]))
    return () => { alive = false }
  }, [form.platform, form.connectionId])

  useEffect(() => {
    if (!form.connectionId) return setQuota(null)
    let alive = true
    setQuotaLoading(true)
    const load = isTauri ? automationApi.quota(form.connectionId, false) : Promise.resolve(demoQuota(Date.now()))
    load.then((v) => alive && setQuota(v), (e: unknown) => alive && setQuota({ state: "unknown", reason: String(e), resets_at: null, windows: [] }))
      .finally(() => alive && setQuotaLoading(false))
    return () => { alive = false }
  }, [form.connectionId])

  const account = accounts?.find((a) => a.id === form.connectionId)
  const accountName = account?.name ?? "所选账号"
  const tip = quota ? quotaTip(quota, accountName, now) : null
  const quotaDisabled = tip?.disabled ? tip.body : null
  const errors: FormErrors = submitted ? validateForm(form, now, quotaDisabled) : {}
  const tool = tools?.find((t) => t.platform === form.platform)
  const next = form.launch === "schedule" ? nextFire(toSchedule(form), now) : null

  const save = async () => {
    setSubmitted(true)
    const e = validateForm(form, Date.now(), quotaDisabled)
    if (Object.keys(e).length) return
    const input: TaskInputDto = {
      name: form.name.trim(), platform: form.platform, connection_id: form.connectionId, model: form.model || null, cwd: form.cwd.trim(),
      session_id: form.sessionId, prompt: form.prompt.trim(), skills: form.skills, launch: form.launch,
      schedule: form.launch === "schedule" ? toSchedule(form) : null, auto_resume: form.autoResume,
      wait_deadline: form.deadline ? new Date(form.deadline).getTime() : null, serial_queue: form.serialQueue,
    }
    if (!isTauri) return onSaved(request.task?.id ?? 0, input.launch)
    setSaving(true)
    setSaveError(null)
    try {
      if (request.mode === "edit" && request.task) {
        await automationApi.update(request.task.id, input)
        onSaved(request.task.id, input.launch)
      } else {
        onSaved(await automationApi.create(input), input.launch)
      }
    } catch (reason) {
      setSaveError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setSaving(false)
    }
  }

  const pickDir = async () => {
    if (!isTauri) return set("cwd", "D:/develop/jolu")
    try {
      const dir = await automationApi.pickDir()
      if (dir) setForm((f) => ({ ...f, cwd: dir, sessionId: samePath(dir, f.cwd) ? f.sessionId : null }))
    } catch (reason) {
      setSaveError(String(reason))
    }
  }

  const title = request.mode === "edit" ? "编辑任务" : fromSession ? "新建任务 · 来自会话" : "新建任务"
  const primary = request.mode === "edit" ? "保存修改" : form.launch === "quota" ? "创建并等待" : "创建任务"
  const accountOptions: Option[] = (accounts ?? []).map((a) => ({
    value: a.id,
    label: accountText(a.name, a.kind),
    tag: a.current ? "当前登录" : undefined,
    note: a.current ? undefined : `不是 ${toolName(form.platform)} 当前登录的账号，执行前需在供应商页启用`,
  }))
  const modelOptions: Option[] = [{ value: "", label: `跟随 ${toolName(form.platform)} 默认`, note: "不指定模型参数" }, ...models.map((m) => ({ value: m, label: m }))]

  return (
    <Dialog.Root open onOpenChange={(open) => !open && !saving && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          ref={setRoot}
          onInteractOutside={(e) => { if (saving) e.preventDefault() }}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[620px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
        >
          <div className="flex h-[34px] items-center gap-2.5">
            <Dialog.Title className="flex-1 truncate text-xl font-semibold text-jolu-text">{title}</Dialog.Title>
            <Dialog.Close aria-label="关闭" disabled={saving} className="grid size-[30px] place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <X aria-hidden className="size-4" />
            </Dialog.Close>
          </div>

          <div ref={scrollRef} className="-mr-2 flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-2">
            <Field label="任务名称 *" error={errors.name}>
              <input value={form.name} onChange={(e) => set("name", e.target.value)} maxLength={80} placeholder="例如：完善登录页面" className={cn(INPUT, "placeholder:text-jolu-muted", errors.name && "border-jolu-danger")} />
            </Field>

            <div className="grid grid-cols-2 gap-3">
              <Field label="执行工具 *" note={tool && !tool.found ? `未检测到 ${toolName(form.platform)}，执行时会提示安装` : undefined}>
                <FieldSelect
                  label="执行工具"
                  value={form.platform}
                  placeholder="选择执行工具"
                  onChange={(v) => setForm((f) => ({ ...f, platform: v as AutomationPlatform, connectionId: "", model: "", sessionId: null }))}
                  options={PLATFORMS.map((p) => ({ value: p, label: toolName(p), tag: "已适配", note: tools && !tools.find((t) => t.platform === p)?.found ? "本机未检测到" : undefined }))}
                />
              </Field>
              <Field label="执行账号 *" error={errors.connectionId} note={account && !account.current ? "执行时会检查账号；Jolu 不会自动切换原工具账号" : undefined}>
                <FieldSelect
                  label="执行账号"
                  value={form.connectionId}
                  placeholder={accounts === null ? "正在读取账号…" : accounts.length ? "请选择可用账号" : "没有可用账号"}
                  onChange={(v) => set("connectionId", v)}
                  options={accountOptions}
                  invalid={!!errors.connectionId}
                />
              </Field>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <Field label="模型 *">
                <FieldSelect label="模型" value={form.model} placeholder="选择账号后获取模型" disabled={!form.connectionId} onChange={(v) => set("model", v)} options={form.connectionId ? modelOptions : []} />
              </Field>
              <Field label="项目目录 *" error={errors.cwd}>
                <div className={cn(INPUT, "pr-1.5", errors.cwd && "border-jolu-danger")}>
                  <input value={form.cwd} onChange={(e) => set("cwd", e.target.value)} placeholder="选择项目目录" className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-jolu-muted" />
                  <Tip label="选择目录">
                    <button type="button" aria-label="选择项目目录" onClick={() => void pickDir()} className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
                      <FolderOpen aria-hidden className="size-[15px]" />
                    </button>
                  </Tip>
                </div>
              </Field>
            </div>

            <SessionPicker
              form={form}
              open={pickerOpen}
              onOpenChange={setPickerOpen}
              accountName={account?.name ?? null}
              fromSession={fromSession}
              onPick={(session) => {
                setForm((f) => ({ ...f, sessionId: session?.session_id ?? null, cwd: session?.cwd && !f.cwd ? session.cwd : f.cwd }))
                setPickerOpen(false)
              }}
              initial={request.session ?? null}
              root={root}
              scroller={scrollRef.current}
            />

            <Field label="执行内容 *" error={errors.prompt}>
              <textarea
                value={form.prompt}
                onChange={(e) => set("prompt", e.target.value)}
                placeholder={form.sessionId ? "描述希望 AI 在这个会话中继续完成的任务……" : "描述希望 AI 完成的任务……"}
                className={cn("h-24 w-full resize-none rounded-lg border border-jolu-border bg-jolu-bg p-3 text-xs leading-[1.6] text-jolu-text outline-none placeholder:text-jolu-muted focus:border-jolu-accent", errors.prompt && "border-jolu-danger")}
              />
            </Field>

            <SkillPicker
              platform={form.platform}
              cwd={form.cwd}
              value={form.skills}
              onChange={(names) => set("skills", names)}
              open={skillsOpen}
              onOpenChange={setSkillsOpen}
              root={root}
              scroller={scrollRef.current}
            />

            <Field label="启动方式" error={errors.launch}>
              <div role="radiogroup" aria-label="启动方式" className="grid grid-cols-3 gap-2">
                {([["now", "立即执行"], ["schedule", "定时执行"], ["quota", "额度恢复后执行"]] as const).map(([value, label]) => {
                  const checked = form.launch === value
                  const disabled = value === "quota" && !!quotaDisabled
                  return (
                    <button
                      key={value}
                      type="button"
                      role="radio"
                      aria-checked={checked}
                      disabled={disabled && !checked}
                      title={disabled ? quotaDisabled ?? undefined : undefined}
                      onClick={() => set("launch", value)}
                      className={cn(
                        "flex h-[34px] items-center gap-2 rounded-lg border px-3 text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45",
                        checked ? "border-jolu-accent bg-jolu-selected text-jolu-on" : "border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-stage",
                      )}
                    >
                      {checked ? <CircleDot aria-hidden className="size-4 text-jolu-on" /> : <Circle aria-hidden className="size-4 text-jolu-muted" />}
                      {label}
                    </button>
                  )
                })}
              </div>
            </Field>

            {form.launch === "schedule" && (
              <div className="flex flex-col gap-3 rounded-xl border border-jolu-border p-3.5">
                <div className={cn("grid gap-3", form.freq === "daily" ? "grid-cols-2" : "grid-cols-3")}>
                  <Field label="执行频率">
                    <FieldSelect label="执行频率" value={form.freq} placeholder="选择频率" onChange={(v) => set("freq", v as TaskForm["freq"])} options={[{ value: "once", label: "单次" }, { value: "daily", label: "每天" }, { value: "weekly", label: "每周" }]} />
                  </Field>
                  {form.freq === "weekly" && (
                    <Field label="星期">
                      <FieldSelect label="星期" value={String(form.weekday)} placeholder="选择星期" onChange={(v) => set("weekday", Number(v))} options={WEEKDAYS.map((w, i) => ({ value: String(i + 1), label: w }))} />
                    </Field>
                  )}
                  {form.freq === "once" && (
                    <Field label="执行日期">
                      <input type="date" value={form.date} min={dateKey(new Date())} onChange={(e) => set("date", e.target.value)} className={INPUT} />
                    </Field>
                  )}
                  <Field label="执行时间" error={errors.time}>
                    <input type="time" value={form.time} onChange={(e) => set("time", e.target.value)} className={cn(INPUT, errors.time && "border-jolu-danger")} />
                  </Field>
                </div>
                <Field label="时区" note="其他时区待确认：目前按本机时区调度">
                  <FieldSelect label="时区" value={form.timezone} placeholder="本机时区" onChange={(v) => set("timezone", v)} options={[{ value: localZone(), label: localZone(), note: "本机时区" }]} />
                </Field>
                {next && <span className="text-[11px] text-jolu-on">下次运行：{dayTime(next, now)}（{zoneName(form.timezone)}时间）</span>}
                <span className="text-[10px] text-jolu-muted">支持单次 / 每天 / 每周；每周选择星期。当前仍在执行时跳过本次触发，不叠加新运行。</span>
              </div>
            )}

            {form.launch === "quota" && (
              form.connectionId ? (
                quotaLoading && !quota ? (
                  <Notice icon={LoaderCircle} title="正在读取额度…" />
                ) : tip && (
                  <Notice icon={tip.disabled ? Info : CircleCheck} tone={tip.disabled ? "danger" : "selected"} title={tip.title}>{tip.body}</Notice>
                )
              ) : <Notice title="选择账号后读取额度">仅已适配且可读取额度的账号可选择此启动方式。</Notice>
            )}

            {fromSession && (
              <Notice icon={CircleCheck} title={<span className="text-jolu-on">已带入当前会话</span>}>
                {request.session?.connection_id
                  ? "请填写任务内容与启动方式；创建前将验证账号与原会话是否兼容。"
                  : "请先选择执行账号、填写任务内容。当前会话未归属账号，创建前将验证账号与原会话是否兼容。"}
              </Notice>
            )}

            <Checkbox checked={form.autoResume} onChange={(v) => set("autoResume", v)} title="执行中额度耗尽时，等待恢复后自动继续" note="续跑能力以所选工具为准；不能安全续跑时保留记录并提示处理。" />

            <button type="button" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)} className="flex h-7 items-center gap-2.5 rounded-md text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <ChevronRight aria-hidden className={cn("size-[15px] text-jolu-muted transition-transform", advanced && "rotate-90")} />
              <span className="text-xs text-jolu-text">高级设置</span>
              <span className="text-[10px] text-jolu-muted">{form.deadline ? `等待至 ${dayTime(new Date(form.deadline).getTime(), now)}` : "不限等待时间"}{form.serialQueue ? " · 同账号顺序执行" : " · 同账号可并行"}</span>
            </button>
            {advanced && (
              <div className="flex flex-col gap-3 rounded-xl border border-jolu-border p-3.5">
                <Field label="等待截止时间" error={errors.deadline}>
                  <div className="grid grid-cols-2 gap-2">
                    <FieldSelect
                      label="等待截止时间"
                      value={form.deadline ? "until" : "none"}
                      placeholder="不限制"
                      onChange={(v) => set("deadline", v === "none" ? "" : `${dateKey(new Date(Date.now() + 86_400_000))}T09:00`)}
                      options={[{ value: "none", label: "不限制" }, { value: "until", label: "指定截止时间" }]}
                    />
                    {form.deadline && <input type="datetime-local" value={form.deadline} onChange={(e) => set("deadline", e.target.value)} className={INPUT} />}
                  </div>
                </Field>
                {/* 关闭态画板 60、68 未画，按 D-09 显示提示文案（待确认） */}
                <Checkbox
                  checked={form.serialQueue}
                  onChange={(v) => set("serialQueue", v)}
                  title="同一账号按队列顺序执行"
                  note={form.serialQueue ? "默认开启，暂停当前任务后再处理后续任务。" : "同账号任务可能同时运行并争用额度；同一工作目录仍按顺序执行。"}
                />
                <span className="text-[10px] text-jolu-muted">到期仍不可用则标记「等待超时」，保留记录，由用户重新排队。恢复时间已知：到点检查；时间未知或检查失败：间隔重试，逐步延长。</span>
              </div>
            )}

            <span className="text-[10px] text-jolu-muted">
              {fromSession
                ? "来源会话不会被替换为新会话。账号或项目发生变化时重新校验；需要用户重新选择时明确提示。"
                : form.launch === "quota"
                  ? "额度恢复任务默认执行一次。仅已适配且可读取额度的账号可选择此启动方式。"
                  : "本地任务需要 Jolu 后台运行；所选账号必须是原工具当前登录的账号。"}
            </span>
          </div>

          {saveError && <p role="alert" className="text-[11px] text-jolu-danger">{saveError}</p>}
          <div className="flex h-[42px] items-end justify-end gap-2.5 border-t border-jolu-border pt-2">
            {fromSession && request.session ? (
              <JoluButton size="sm" variant="outline" disabled={saving} onClick={() => onBackToSession(request.session!)}>返回会话</JoluButton>
            ) : (
              <JoluButton size="sm" variant="outline" disabled={saving} onClick={onClose}>取消</JoluButton>
            )}
            <JoluButton size="sm" variant="primary" disabled={saving} onClick={() => void save()}>
              {saving ? <LoaderCircle aria-hidden className="animate-spin" /> : <Plus aria-hidden />}{saving ? "正在保存…" : primary}
            </JoluButton>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

/** 执行会话搜索选择（画板 70）：新建会话 + 当前项目里可继续的会话；仅用量记录的会话不可选 */
function SessionPicker({ form, open, onOpenChange, accountName, fromSession, onPick, initial, root, scroller }: {
  form: TaskForm
  open: boolean
  onOpenChange: (open: boolean) => void
  accountName: string | null
  fromSession: boolean
  onPick: (session: SessionItemDto | null) => void
  initial: SessionItemDto | null
  /** 弹窗根节点：下拉以它为定位容器 */
  root: HTMLElement | null
  scroller: HTMLElement | null
}) {
  const [query, setQuery] = useState("")
  const [sessions, setSessions] = useState<SessionItemDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [known, setKnown] = useState<SessionItemDto | null>(initial)
  const ref = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [place, setPlace] = useState<{ top: number; left: number; width: number; maxHeight: number } | null>(null)

  useEffect(() => {
    if (!open) return
    let alive = true
    setError(null)
    const timer = window.setTimeout(() => {
      const load = isTauri
        ? api.sessionsList({ platform: form.platform, connection: null, project: null, since: null, search: query.trim() || null })
        : Promise.resolve(demoSessions(Date.now()))
      load.then((list) => alive && setSessions(list.sessions.filter((s) => s.platform === form.platform)), (e: unknown) => alive && setError(String(e)))
    }, query ? 250 : 0)
    return () => { alive = false; window.clearTimeout(timer) }
  }, [open, query, form.platform])

  // 下拉相对弹窗根节点定位在触发器下方；表单滚动或窗口变化时跟随，底部不超出弹窗
  useLayoutEffect(() => {
    if (!open || !root) return setPlace(null)
    const update = () => {
      const trigger = triggerRef.current
      if (!trigger) return
      const box = root.getBoundingClientRect()
      const r = trigger.getBoundingClientRect()
      const top = r.bottom - box.top + 6
      setPlace({ top, left: r.left - box.left, width: r.width, maxHeight: Math.max(160, box.height - top - 12) })
    }
    update()
    scroller?.addEventListener("scroll", update)
    window.addEventListener("resize", update)
    return () => {
      scroller?.removeEventListener("scroll", update)
      window.removeEventListener("resize", update)
    }
  }, [open, root, scroller])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (!ref.current?.contains(target) && !panelRef.current?.contains(target)) onOpenChange(false)
    }
    document.addEventListener("mousedown", onDown)
    return () => document.removeEventListener("mousedown", onDown)
  }, [open, onOpenChange])

  const matches = useMemo(() => (sessions ?? []).filter((s) => !form.cwd || samePath(s.cwd, form.cwd)).slice(0, 30), [sessions, form.cwd])
  const selected = form.sessionId ? (matches.find((s) => s.session_id === form.sessionId) ?? sessions?.find((s) => s.session_id === form.sessionId) ?? (known?.session_id === form.sessionId ? known : null)) : null
  const project = projectOf(form.cwd || null)

  return (
    <div ref={ref} className="relative flex flex-col gap-[7px]">
      <span className="text-[11px] text-jolu-muted">执行会话</span>
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className={cn("flex h-[52px] w-full items-center gap-2.5 rounded-lg border bg-jolu-bg px-3 text-left outline-none focus-visible:border-jolu-accent", open ? "border-jolu-accent" : "border-jolu-border")}
      >
        <MessagesSquare aria-hidden className="size-[18px] shrink-0 text-jolu-on" />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="truncate text-xs font-medium text-jolu-text">{form.sessionId ? (selected ? sessionTitle(selected) : `会话 ${shortId(form.sessionId)}`) : "新建会话"}</span>
          <span className="truncate text-[10px] text-jolu-muted">
            {form.sessionId
              ? `${project} · ${selected ? dayTime(selected.last_ts, Date.now()) : toolName(form.platform)} · ${shortId(form.sessionId)}`
              : "从新的上下文开始"}
          </span>
        </span>
        {open ? <ChevronUp aria-hidden className="size-[15px] text-jolu-muted" /> : <ChevronDown aria-hidden className="size-[15px] text-jolu-muted" />}
      </button>
      <span className="text-[10px] text-jolu-muted">
        {form.sessionId ? (fromSession ? "已从会话线程带入；执行时验证是否可在此会话继续。" : "将在此会话追加任务指令，保留已有上下文。") : "将新建会话执行任务。"}
      </span>

      {open && root && place && createPortal(
        <div
          ref={panelRef}
          style={{ top: place.top, left: place.left, width: place.width, maxHeight: place.maxHeight }}
          className="absolute z-10 flex flex-col gap-1.5 overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg p-2 shadow-popover"
        >
          <label className="flex h-[38px] items-center gap-2 rounded-lg border border-jolu-accent px-2.5">
            <Search aria-hidden className="size-4 text-jolu-muted" />
            <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索会话标题、ID 或正文" aria-label="搜索会话" className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted" />
            {query && <button type="button" aria-label="清除搜索" onClick={() => setQuery("")} className="text-jolu-muted hover:text-jolu-text"><X aria-hidden className="size-3.5" /></button>}
          </label>
          <button type="button" onClick={() => onPick(null)} className={cn("flex h-[38px] items-center gap-2.5 rounded-lg px-2.5 text-left outline-none hover:bg-jolu-side focus-visible:bg-jolu-side", !form.sessionId && "bg-jolu-selected")}>
            <Plus aria-hidden className="size-4 text-jolu-on" />
            <span className="text-xs text-jolu-on">新建会话</span>
            <span className="text-[10px] text-jolu-muted">从新的上下文开始</span>
          </button>
          <span className="px-2.5 text-[10px] text-jolu-muted">{form.cwd ? `${project} · ` : ""}{sessions === null && !error ? "正在读取会话…" : `匹配 ${matches.length} 条会话`}</span>
          {error && <span role="alert" className="px-2.5 text-[11px] text-jolu-danger">会话读取失败：{error}</span>}
          <div className="flex max-h-[240px] min-h-0 shrink flex-col gap-0.5 overflow-y-auto">
            {matches.map((s) => {
              const usable = s.has_source
              const checked = s.session_id === form.sessionId
              return (
                <button
                  key={s.session_id}
                  type="button"
                  disabled={!usable}
                  onClick={() => { setKnown(s); onPick(s) }}
                  className={cn("flex h-14 shrink-0 items-center gap-2.5 rounded-lg px-2.5 text-left outline-none hover:bg-jolu-side focus-visible:bg-jolu-side disabled:cursor-not-allowed disabled:hover:bg-transparent", checked && "bg-jolu-selected hover:bg-jolu-selected")}
                >
                  {usable ? <MessageSquareText aria-hidden className="size-4 shrink-0 text-jolu-muted" /> : <FileText aria-hidden className="size-4 shrink-0 text-jolu-muted" />}
                  <span className="flex min-w-0 flex-1 flex-col gap-1">
                    <span className={cn("truncate text-xs", checked ? "text-jolu-on" : usable ? "text-jolu-text" : "text-jolu-muted")}>{sessionTitle(s)}</span>
                    <span className="truncate text-[10px] text-jolu-muted">{usable ? `${dayTime(s.last_ts, Date.now())} · ${shortId(s.session_id)}` : "仅用量记录 · 无法继续此会话"}</span>
                  </span>
                  {checked && <Check aria-hidden className="size-4 text-jolu-on" />}
                  {!usable && <Info aria-hidden className="size-4 text-jolu-muted" />}
                </button>
              )
            })}
          </div>
          <span className="border-t border-jolu-border px-2.5 pt-2 text-[10px] text-jolu-muted">{toolName(form.platform)}{accountName ? ` · ${accountName}` : ""}{form.cwd ? ` · ${project}` : ""}</span>
        </div>,
        root,
      )}
    </div>
  )
}

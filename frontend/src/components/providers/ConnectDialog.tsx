import { SelectField } from "@/components/ui/SelectField"
import { useDialogClose } from "@/hooks/useDialogClose"
import { useBrowserAuthorization } from "@/hooks/useBrowserAuthorization"
import { OPENCODE_SERVICES } from "@/lib/providerCatalog"
import { useEffect, useId, useRef, useState, type ReactNode } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { BadgeCheck, Check, ChevronDown, CircleAlert, CircleCheck, Eye, EyeOff, LoaderCircle, LogIn, RefreshCw, Search, X } from "lucide-react"
import { api, isTauri, type LoginDetectionDto } from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  API_PROTOCOLS,
  canEnterManualModel,
  PROVIDERS,
  defaultMode,
  fallsBackToManualModel,
  filterModels,
  modelFetchErrorText,
  providerEntry,
  showsModeTabs,
  singleModeReason,
  protocolBaseUrlHint,
  protocolLabel,
  validateConnectionName,
  type ApiProtocol,
  type ConnectMode,
  type ProviderId,
} from "@/lib/providerCatalog"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluButton } from "@/components/ui/JoluButton"
import { useToast } from "@/components/ui/Toast"
import type { Connection, PlatformId } from "@/types"

export interface ConnectActions {
  connections: readonly Connection[]
  readLocal: (platform: string, kind: "auth" | "api", name?: string, edition?: string) => Promise<{ added: number; existing: number; warnings?: string[] }>
  add: (input: { platform: string; kind: "auth" | "api"; name: string; secret?: string | null; base_url?: string | null; model?: string | null; service?: string | null; protocol?: ApiProtocol | null }) => Promise<void>
}

/**
 * 新建连接弹窗（Jolu.pen 画板 11 订阅 / 13 仅订阅 / 15–17 API Key）。
 * 订阅：检测本机登录 → 显示账号 → 保存；API Key：填写地址与密钥 → 获取模型 → 选择 → 保存（后端验证后入库）。
 * 自定义供应商（D-11）复用 API Key 结构，多一个「协议」选择；模型接口不可用时改为手填模型 ID，
 * 后端以「获取模型成功」或「手填模型」作为接口可达的依据，不查余额。
 */
export function ConnectDialog({ platform: initialPlatform, actions, onClose: onDismiss }: {
  platform: ProviderId
  actions: ConnectActions
  onClose: () => void
}) {
  const { open: dialogOpen, close: onClose } = useDialogClose(onDismiss)
  const toast = useToast()
  const [provider, setProvider] = useState(initialPlatform)
  const entry = providerEntry(provider)
  const platform = entry.platformId
  const [service,setService]=useState<string>("opencode-go")
  const [mode, setMode] = useState<ConnectMode>(initialPlatform==="opencode"?"apiKey":defaultMode(entry))
  const [name, setName] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  // 订阅
  const [detecting, setDetecting] = useState(false)
  const [detection, setDetection] = useState<LoginDetectionDto | null>(null)
  const edition = entry.edition ?? "default"
  const loginRequest = useRef(0)
  const browser = useBrowserAuthorization(platform, edition, dialogOpen && mode === "subscription")
  const actualDetection = browser.detection ?? detection
  // API Key
  const [baseUrl, setBaseUrl] = useState(entry.baseUrl?.value ?? "")
  const [secret, setSecret] = useState("")
  const [secretVisible, setSecretVisible] = useState(false)
  const [models, setModels] = useState<string[]>([])
  const [model, setModel] = useState("")
  const [protocol, setProtocol] = useState<ApiProtocol>(entry.baseUrlChoices?.[0].protocol ?? "openai")
  const [fetching, setFetching] = useState(false)
  /** 每次地址 / 密钥变化或重新获取都递增；旧请求回来时直接丢弃，防止串到新地址 */
  const modelRequest = useRef(0)
  const [pendingPlatform, setPendingPlatform] = useState<ProviderId | null>(null)
  /** 模型列表为空或接口不支持枚举时，改为手动填写模型 ID（画板 27） */
  const [manualModel, setManualModel] = useState(false)
  const [modelError, setModelError] = useState<string | null>(null)
  const [askVerify, setAskVerify] = useState(false)
  const [verifying, setVerifying] = useState(false)
  const verifyingRef = useRef(false)
  const [verifyResult, setVerifyResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)

  // 对话框关闭或卸载后，正在返回的获取 / 验证结果不再写入草稿。
  useEffect(() => {
    if (!dialogOpen) modelRequest.current++
    return () => { modelRequest.current++ }
  }, [dialogOpen])

  const resetVerification = () => {
    verifyingRef.current = false
    setVerifying(false)
    setAskVerify(false)
    setVerifyResult(null)
  }

  const custom = platform === "custom"
  const tabs = platform!=="opencode" && showsModeTabs(entry)
  const reason = singleModeReason(entry)
  const kind = mode === "subscription" ? "auth" : "api"
  const hasDraft = Boolean(name.trim() || secret.trim() || actualDetection || browser.pending)

  const switchPlatform = (next: ProviderId) => {
    const nextEntry = providerEntry(next)
    // 更换平台不携带原平台的密钥与检测结果（画板 29）
    modelRequest.current++
    setFetching(false)
    resetVerification()
    setProvider(next)
    ++loginRequest.current
    setDetecting(false)
    browser.cancel()
    setMode(next==="opencode"?"apiKey":defaultMode(nextEntry))
    setService("opencode-go")
    setName("")
    setDetection(null)
    setBaseUrl(nextEntry.baseUrl?.value ?? "")
    setSecret("")
    setSecretVisible(false)
    setModels([])
    setModel("")
    setProtocol(nextEntry.baseUrlChoices?.[0].protocol ?? "openai")
    setManualModel(false)
    setModelError(null)
    setError(null)
    setPendingPlatform(null)
  }

  const detect = async () => {
    if (detecting) return
    const request = ++loginRequest.current
    browser.cancel()
    setDetecting(true)
    setError(null)
    try {
      const result = await api.detectLocalLogin(platform, entry.edition)
      if (request === loginRequest.current) setDetection(result)
    } catch (reason) {
      if (request === loginRequest.current) { setDetection(null); setError(`检测失败：${String(reason)}`) }
    } finally {
      if (request === loginRequest.current) setDetecting(false)
    }
  }

  const invalidateModels = () => {
    modelRequest.current++
    resetVerification()
    setModels([])
    if (!manualModel) setModel("")
    setModelError(null)
    setFetching(false)
  }

  const cancelFetch = () => {
    modelRequest.current++
    setFetching(false)
  }

  const fetchModels = async () => {
    const request = ++modelRequest.current
    resetVerification()
    setFetching(true)
    setModelError(null)
    try {
      const list = await api.fetchRemoteModels(platform, baseUrl.trim(), secret.trim(), custom || platform === "zcode" ? protocol : null)
      if (request !== modelRequest.current) return
      setModels(list)
      setManualModel(false)
      // 空列表或新列表不含原选择时不自动选中（画板 29）
      setModel((current) => (list.includes(current) ? current : ""))
      toast.success(`已获取 ${list.length} 个模型`)
    } catch (reason) {
      if (request !== modelRequest.current) return
      // 失败就地说明，已填写内容保留（画板 27）
      setModelError(String(reason))
      // 模型接口不可用时改为手填（画板 27）；密钥被拒时不改。ZCode 手填后还须验证模型。
      if ((custom || platform === "zcode") && !manualModel && fallsBackToManualModel(String(reason))) {
        setManualModel(true)
        setModel("")
      }
    } finally {
      if (request === modelRequest.current) setFetching(false)
    }
  }

  const verifyModel = async () => {
    if (verifyingRef.current || fetching || !model.trim() || !secret.trim() || !baseUrl.trim()) return
    const request = ++modelRequest.current
    verifyingRef.current = true
    setVerifying(true)
    setAskVerify(false)
    setVerifyResult(null)
    try {
      await api.verifyCustomModel(baseUrl.trim(), secret.trim(), protocol, model.trim())
      if (request === modelRequest.current) {
        setVerifyResult({ ok: true, message: "验证通过：服务已接受该模型的请求。" })
        setModelError(null)
      }
    } catch (reason) {
      if (request === modelRequest.current) setVerifyResult({ ok: false, message: String(reason) })
    } finally {
      if (request === modelRequest.current) {
        verifyingRef.current = false
        setVerifying(false)
      }
    }
  }

  const nameError = validateConnectionName(name, actions.connections, provider, kind)
  const urlError = baseUrl.trim() && !/^https?:\/\/[^\s/]+/i.test(baseUrl.trim()) ? "请输入完整地址，例如 https://api.example.com/v1" : null
  const canSave = !saving && !nameError && (mode === "subscription"
    ? Boolean(actualDetection?.found) && !browser.pending && !detecting
    : Boolean(baseUrl.trim() && !urlError && secret.trim() && model.trim())
      && (platform !== "zcode" || (!fetching && !verifying && (!manualModel || verifyResult?.ok === true))))

  const save = async () => {
    if (!canSave) return
    setSaving(true)
    setError(null)
    try {
      if (mode === "subscription") {
        if (browser.detection?.found) {
          await browser.save(name.trim())
          toast.success(`已连接 ${entry.name}`, "浏览器账号已通过真实额度查询，凭证加密保存在本机")
          onClose()
          return
        }
        const summary = await actions.readLocal(platform, "auth", name.trim(), entry.edition)
        if (!summary.added && !summary.existing) throw new Error("未读取到可保存的登录，请重新检测")
        toast.success(summary.added ? `已连接 ${entry.name}` : "该账号已连接，无需重复添加", summary.warnings?.join("；"))
      } else {
        await actions.add({ platform, service:platform==="opencode"?service:null, kind: "api", name: name.trim(), secret: secret.trim(), base_url: baseUrl.trim(), model: model.trim(), protocol: custom || platform === "zcode" ? protocol : null })
        toast.success(`已连接 ${entry.name}`, custom
          ? (manualModel ? "模型 ID 为手动填写，未经模型接口验证；余额将在账号管理中独立查询" : "已通过模型接口验证；余额将在账号管理中独立查询")
          : "接口可达已验证；模型调用权限与额度分别展示")
      }
      onClose()
    } catch (reason) {
      // 失败保留输入（画板 29）
      setError(String(reason))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog.Root open={dialogOpen} onOpenChange={(open) => {
      if (open || saving) return
      // 关闭、Esc 时有草稿先确认（画板 27「未保存更改」）
      if (hasDraft) setConfirmDiscard(true)
      else onClose()
    }}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[720px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-7 shadow-dialog outline-none"
        >
          <div className="flex items-center justify-between">
            <Dialog.Title className="text-xl font-semibold text-jolu-text">
              {custom ? "新建自定义供应商连接" : mode === "subscription" ? "新建订阅连接" : "新建 API Key 连接"}
            </Dialog.Title>
            <Dialog.Close asChild>
              <button type="button" aria-label="关闭" disabled={saving} className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
                <X aria-hidden className="size-5" />
              </button>
            </Dialog.Close>
          </div>

          <div className="-mx-1 flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto px-1">
            <div className="flex h-10 items-center gap-2.5">
              <span className="grid size-7 place-items-center"><PlatformLogo platform={platform} size={22} /></span>
              <span className="text-sm font-semibold text-jolu-text">{entry.name}</span>
              <span className="flex-1" />
              {!tabs && !custom && <span className="rounded-[5px] bg-jolu-stage px-2 py-1 text-[11px] text-jolu-muted">{mode === "subscription" ? "当前仅订阅" : "当前仅 API Key"}</span>}
              <PlatformSwitcher
                current={provider}
                onPick={(next) => {
                  if (next === provider) return
                  if (hasDraft) setPendingPlatform(next)
                  else switchPlatform(next)
                }}
              />
            </div>
            {pendingPlatform && (
              <div role="alert" className="flex items-center gap-3 rounded-[10px] border border-jolu-border bg-jolu-side px-4 py-2.5 text-xs text-jolu-text">
                <span className="min-w-0 flex-1">更换为 {providerEntry(pendingPlatform).name} 会清空已填写的内容与检测结果。</span>
                <JoluButton size="sm" variant="outline" onClick={() => setPendingPlatform(null)}>继续填写</JoluButton>
                <JoluButton size="sm" variant="soft" onClick={() => switchPlatform(pendingPlatform)}>更换平台</JoluButton>
              </div>
            )}

            {tabs ? (
              <div role="tablist" aria-label="连接方式" className="flex h-[38px] gap-1 rounded-lg bg-jolu-side p-[3px]">
                {(["subscription", "apiKey"] as const).map((value) => (
                  <button
                    key={value}
                    type="button"
                    role="tab"
                    aria-selected={mode === value}
                    onClick={() => { if (value !== mode) { invalidateModels(); setMode(value); setError(null) } }}
                    className={cn(
                      "flex-1 rounded-md text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                      mode === value ? "bg-jolu-bg font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text",
                    )}
                  >
                    {value === "subscription" ? "订阅" : "API Key"}
                  </button>
                ))}
              </div>
            ) : reason && <p className="text-xs text-jolu-muted">{reason}</p>}

            <Field label={custom ? "名称 *" : "连接名称 *"} hint={mode === "apiKey" ? "用于区分多个账号或服务，不影响供应商配置。" : actualDetection?.found && !name.trim() ? "请填写连接名称后保存。" : undefined} error={name && nameError ? nameError : null}>
              {(id) => (
                <TextInput id={id} value={name} onChange={setName} placeholder={mode === "subscription" ? "例如：个人订阅" : custom ? "例如：公司内部网关" : "例如：工作 API"} />
              )}
            </Field>

            {platform==="opencode" && <label className="text-xs text-jolu-text">模型服务<SelectField label="模型服务" className="mt-2 h-10 w-full" value={service} options={OPENCODE_SERVICES.map(s=>({value:s.id,label:s.label}))} onValueChange={value=>{const selected=OPENCODE_SERVICES.find(s=>s.id===value)!;setService(selected.id);setMode(selected.mode);setBaseUrl(selected.baseUrl);setSecret("");setDetection(null);invalidateModels()}} /></label>}
            {mode === "subscription" ? (
              <>
              <SubscriptionSection
                platform={platform}
                providerName={entry.name}
                detecting={detecting || browser.pending}
                detection={actualDetection}
                onDetect={() => void detect()}
                browserPending={browser.pending}
                browserMessage={browser.message}
                browserSupported={browser.supported}
                browserReady={!!browser.detection?.found}
                onBrowser={() => { ++loginRequest.current; setDetection(null); setError(null); void browser.start() }}
                tabs={tabs}
              />
              </>
            ) : (
              <>
                {custom && (
                  <ProtocolPicker
                    value={protocol}
                    onChange={(next) => {
                      if (next === protocol) return
                      // 协议决定鉴权头，切换后已获取的模型作废
                      setProtocol(next)
                      invalidateModels()
                    }}
                  />
                )}
                {entry.baseUrlChoices && <label className="text-xs text-jolu-text">服务区域与协议<SelectField label="服务区域与协议" className="mt-2 h-10 w-full" value={baseUrl} options={entry.baseUrlChoices.map(c=>({value:c.value,label:c.label}))} onValueChange={value=>{const choice=entry.baseUrlChoices!.find(c=>c.value===value)!;setBaseUrl(choice.value);setProtocol(choice.protocol);invalidateModels()}} /></label>}
                <Field label="Base URL *" hint={entry.baseUrl?.locked ? "该服务限定官方地址，不开放编辑。" : undefined} error={urlError}>
                  {(id) => (
                    <TextInput
                      id={id}
                      value={baseUrl}
                      readOnly={platform==="opencode"?service!=="openai-compatible":entry.baseUrl?.locked}
                      onChange={(value) => { setBaseUrl(value); invalidateModels() }}
                      placeholder={`请输入完整地址，例如 ${custom ? protocolBaseUrlHint(protocol) : "https://api.example.com/v1"}`}
                      mono
                    />
                  )}
                </Field>
                <Field label="API Key *" hint="默认隐藏；点击眼睛临时显示。">
                  {(id) => (
                    <TextInput
                      id={id}
                      value={secret}
                      onChange={(value) => { setSecret(value); invalidateModels() }}
                      // 离开输入区后恢复遮罩（画板 29）
                      onBlur={() => setSecretVisible(false)}
                      type={secretVisible ? "text" : "password"}
                      placeholder="粘贴 API Key"
                      mono
                      trailing={
                        <button
                          type="button"
                          aria-label={secretVisible ? "隐藏 API Key" : "显示 API Key"}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => setSecretVisible((value) => !value)}
                          className="grid size-6 place-items-center rounded text-jolu-muted hover:text-jolu-text"
                        >
                          {secretVisible ? <Eye aria-hidden className="size-4" /> : <EyeOff aria-hidden className="size-4" />}
                        </button>
                      }
                    />
                  )}
                </Field>
                <div className="flex flex-col gap-2">
                  {custom && manualModel && modelError && !fetching && (
                    <div role="alert" className="flex items-center gap-2.5 rounded-lg bg-jolu-danger-soft px-2.5 py-2 text-[11px] text-jolu-danger">
                      <CircleAlert aria-hidden className="size-3.5 shrink-0" />
                      <span className="min-w-0 flex-1 break-all">{modelFetchErrorText(modelError)}</span>
                    </div>
                  )}
                  <span className="text-xs font-medium text-jolu-text">{manualModel ? "模型 ID *" : "默认模型 *"}</span>
                  <div className="flex gap-2">
                    {manualModel ? (
                      <div className="min-w-0 flex-1">
                        <TextInput id="manual-model" value={model} disabled={fetching || verifying} onChange={(value) => { modelRequest.current++; resetVerification(); setModel(value) }} placeholder="输入服务支持的完整模型 ID" mono />
                      </div>
                    ) : (
                      <ModelPicker models={models} value={model} onChange={setModel} />
                    )}
                    {fetching ? (
                      <JoluButton variant="outline" className="h-10 w-[116px] px-3" onClick={cancelFetch}>取消获取</JoluButton>
                    ) : (
                      <JoluButton
                        variant="soft"
                        className="h-10 w-[116px] px-3"
                        disabled={verifying || !baseUrl.trim() || Boolean(urlError) || !secret.trim() || !isTauri}
                        onClick={() => void fetchModels()}
                      >
                        <RefreshCw aria-hidden />
                        {custom ? (models.length ? "重新获取" : "获取模型") : models.length || modelError ? "重新获取" : "获取模型"}
                      </JoluButton>
                    )}
                    {platform === "zcode" && manualModel && (
                      <JoluButton variant="outline" className="h-10 px-3" disabled={fetching || verifying || !model.trim() || !secret.trim() || !isTauri} onClick={() => setAskVerify(true)}>
                        {verifying ? <LoaderCircle aria-hidden className="animate-spin" /> : <BadgeCheck aria-hidden />}
                        {verifying ? "正在验证" : "验证模型"}
                      </JoluButton>
                    )}
                  </div>
                  {platform === "zcode" && manualModel && (
                    <>
                      {askVerify && (
                        <div role="alertdialog" aria-label="验证模型" className="flex items-center gap-3 rounded-[10px] border border-jolu-border bg-jolu-side px-4 py-2.5 text-xs text-jolu-text">
                          <span className="min-w-0 flex-1">验证会向服务发送一次最小请求，可能产生少量 API 用量。</span>
                          <JoluButton size="sm" variant="outline" onClick={() => setAskVerify(false)}>取消</JoluButton>
                          <JoluButton size="sm" variant="soft" disabled={fetching || verifying} onClick={() => void verifyModel()}>继续验证</JoluButton>
                        </div>
                      )}
                      <span role={verifyResult?.ok === false ? "alert" : "status"} className={cn("flex items-center gap-1.5 text-[11px]", verifyResult ? verifyResult.ok ? "text-jolu-on" : "text-jolu-danger" : "text-jolu-muted")}>
                        {verifyResult?.ok && <CircleCheck aria-hidden className="size-3.5 shrink-0" />}
                        {verifying ? "正在验证模型，请等待服务返回…" : verifyResult?.message ?? "未验证 · 验证模型通过后才能保存连接。"}
                      </span>
                    </>
                  )}
                  {fetching ? (
                    <span role="status" className="flex items-center gap-1.5 text-[11px] text-jolu-muted">
                      <LoaderCircle aria-hidden className="size-3 animate-spin" />正在获取可用模型…等待接口返回，请勿重复提交。
                    </span>
                  ) : modelError && !(custom && manualModel) ? (
                    <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
                      <span className="text-jolu-danger">获取失败：{modelError}</span>
                      {!manualModel && canEnterManualModel(platform, modelError) && (
                        <button type="button" onClick={() => { setManualModel(true); setModel("") }} className="font-medium text-jolu-on hover:underline">
                          手动填写模型
                        </button>
                      )}
                    </div>
                  ) : (
                    <span className="text-[11px] text-jolu-muted">
                      {custom
                        ? (manualModel ? "手填模型后即可保存，作为接口可达的依据；可再次点击「获取模型」改回从列表选择。" : "填写地址与密钥后点击「获取模型」，再从下拉框选择；模型接口不可用时改为手填模型 ID。")
                        : manualModel ? platform === "zcode" ? "模型接口不支持枚举时手动填写；验证当前模型后再保存。" : "模型接口不支持枚举时手动填写；保存时会验证地址与密钥。" : "填写地址与密钥后点击「获取模型」，再从下拉框选择；修改后需重新获取。"}
                    </span>
                  )}
                </div>
              </>
            )}
          {(error || browser.error) && <p role="alert" className="text-xs text-jolu-danger">{error || browser.error}</p>}
          </div>
          {confirmDiscard && (
            <div role="alertdialog" aria-label="放弃未保存的更改？" className="flex items-center gap-3 rounded-[10px] border border-jolu-border bg-jolu-side px-4 py-3 text-xs text-jolu-text">
              <span className="min-w-0 flex-1"><b className="font-semibold">放弃未保存的更改？</b> 已填写的内容不会保存。</span>
              <JoluButton size="sm" variant="outline" onClick={() => setConfirmDiscard(false)}>继续编辑</JoluButton>
              <JoluButton size="sm" variant="danger-soft" onClick={onClose}>放弃更改</JoluButton>
            </div>
          )}

          <div className="flex items-center justify-end gap-2.5">
            <JoluButton variant="outline" disabled={saving} onClick={() => (hasDraft ? setConfirmDiscard(true) : onClose())}>取消</JoluButton>
            <JoluButton variant="primary" disabled={!canSave} onClick={() => void save()}>
              {saving && <LoaderCircle aria-hidden className="animate-spin" />}
              {saving ? (mode === "apiKey" ? "正在验证" : "正在保存") : "保存连接"}
            </JoluButton>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function SubscriptionSection({ providerName, detecting, detection, onDetect, tabs, browserPending, browserMessage, browserSupported, browserReady, onBrowser }: {
  platform: PlatformId
  providerName: string
  detecting: boolean
  detection: LoginDetectionDto | null
  onDetect: () => void
  tabs: boolean
  browserPending: boolean
  browserMessage: string | null
  browserSupported: boolean
  browserReady: boolean
  onBrowser: () => void
}) {
  const detectButton = (
    <JoluButton variant="primary" disabled={detecting || !isTauri} onClick={onDetect}>
      {detecting ? <LoaderCircle aria-hidden className="animate-spin" /> : <RefreshCw aria-hidden className="!text-[#071B10]" />}
      读取本地登录信息
    </JoluButton>
  )
  const browserButton = <JoluButton aria-label="浏览器授权" variant="soft" disabled={detecting || browserPending || !browserSupported || !isTauri} onClick={onBrowser}>{browserPending && <LoaderCircle aria-hidden className="animate-spin" />}浏览器授权</JoluButton>

  if (detection?.found) {
    const account = detection.account ?? (detection.sources.length ? detection.sources.join("、") : "未获取账号")
    return (
      <>
        <div className="flex flex-col gap-3 rounded-[10px] bg-jolu-selected p-5">
          <span className="flex items-center gap-2 text-[13px] font-semibold text-jolu-on">
            <CircleCheck aria-hidden className="size-[18px]" />
            {browserReady ? "浏览器账号已验证" : detection.sources.length && !detection.account ? "已检测到本机来源" : "已检测到账号"}
          </span>
          <div className="flex flex-wrap items-center gap-4">
            <span className="break-all text-base font-semibold text-jolu-text">{account}</span>
            {detection.plan && (
              <span className="flex h-7 items-center gap-1.5 rounded-[7px] bg-jolu-bg px-2.5 text-xs font-semibold text-jolu-on">
                <BadgeCheck aria-hidden className="size-3.5" />
                {detection.plan}
              </span>
            )}
          </div>
          <div className="flex gap-6 text-xs">
            <Info label="凭证标识" value={detection.masked ?? "未获取"} />
            <Info label="套餐" value={detection.plan ?? "未获取"} />
          </div>
          <span className="text-[11px] text-jolu-muted">{browserReady ? "账号已通过真实额度查询；保存后凭证加密保存在本机。" : "账号与套餐以本机客户端记录为准，未记录时显示「未获取」。"}</span>
        </div>
        <div className="flex gap-2.5">
          {detectButton}
          {browserButton}
        </div>
        {!tabs && <p className="text-xs text-jolu-muted">此状态只提供订阅连接。平台和账号支持 API Key 时才显示第二种方式。</p>}
      </>
    )
  }

  return (
    <>
      <div className="flex flex-col gap-3.5 rounded-[10px] bg-jolu-side p-5">
        <LogIn aria-hidden className="size-6 text-jolu-text" />
        <span className="text-base font-semibold text-jolu-text">使用 {providerName} 账号连接</span>
        <span className="text-xs text-jolu-muted">{browserSupported ? "读取官方客户端的本地登录，或使用系统默认浏览器完成官方授权。" : "读取官方客户端的本地登录；此供应商的浏览器授权待接入。"}</span>
        <div className="flex gap-2.5">
          {detectButton}
          {browserButton}
        </div>
      </div>
      <div className="flex items-center gap-3 rounded-[10px] border border-jolu-border p-4">
        <LoaderCircle aria-hidden className={cn("size-5 shrink-0 text-jolu-muted", detecting && "animate-spin")} />
        <div className="flex min-w-0 flex-col gap-[7px]">
          <span className="text-[13px] text-jolu-text">{browserPending ? "等待浏览器授权与额度验证" : detecting ? "正在读取本地登录" : "尚未获取有效登录"}</span>
          <span role="status" className="text-xs text-jolu-muted">{browserPending ? browserMessage ?? "请在系统默认浏览器完成官方登录，验证成功后会自动返回。" : detection?.message ?? "读取或授权成功后才能保存连接。"}</span>
        </div>
      </div>
      <p className="text-[11px] text-jolu-muted">Jolu 不要求输入账号密码。账号与套餐信息以检测结果为准。</p>
    </>
  )
}

/** 自定义供应商的接口协议（D-11）：OpenAI 兼容用 Bearer，Anthropic 兼容用 x-api-key */
function ProtocolPicker({ value, onChange }: { value: ApiProtocol; onChange: (protocol: ApiProtocol) => void }) {
  const labelId = useId()
  return (
    <div className="flex flex-col gap-[7px]">
      <span id={labelId} className="text-xs font-medium text-jolu-text">协议 *</span>
      <div role="radiogroup" aria-labelledby={labelId} className="flex h-[38px] gap-1 rounded-lg bg-jolu-side p-[3px]">
        {API_PROTOCOLS.map((item) => (
          <button
            key={item}
            type="button"
            role="radio"
            aria-checked={value === item}
            onClick={() => onChange(item)}
            className={cn(
              "flex-1 rounded-md text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
              value === item ? "bg-jolu-bg font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text",
            )}
          >
            {protocolLabel(item)}
          </button>
        ))}
      </div>
      <span className="text-[11px] text-jolu-muted">决定请求格式与鉴权方式：OpenAI 兼容使用 Bearer，Anthropic 兼容使用 x-api-key。</span>
    </div>
  )
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <span className="flex min-w-0 flex-1 gap-2">
      <span className="shrink-0 text-jolu-muted">{label}</span>
      <span className="truncate font-medium text-jolu-text">{value}</span>
    </span>
  )
}

/** 「更换平台」：列出全部供应商，待接入的不可选 */
function PlatformSwitcher({ current, onPick }: { current: ProviderId; onPick: (platform: ProviderId) => void }) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <JoluButton variant="neutral">更换平台</JoluButton>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} style={{maxHeight:"var(--radix-dropdown-menu-content-available-height)"}} className="motion-popover z-[100] w-[220px] overflow-y-auto rounded-lg border border-jolu-border bg-jolu-bg p-[5px] shadow-popover">
          {PROVIDERS.map((item) => {
            const pending = item.subscription !== "ready" && item.apiKey !== "ready"
            return (
              <Menu.Item
                key={item.id}
                disabled={pending}
                onSelect={() => onPick(item.id)}
                className="flex h-8 cursor-default select-none items-center gap-2 rounded-[5px] px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-45"
              >
                <PlatformLogo platform={item.platformId} />
                <span className="min-w-0 flex-1">{item.name}</span>
                {pending ? <span className="text-[11px] text-jolu-muted">待接入</span> : item.id === current && <Check aria-hidden className="size-3.5 text-jolu-on" />}
              </Menu.Item>
            )
          })}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/** 模型下拉（画板 17）：可搜索，支持方向键与回车选择；空列表不自动选中 */
export function ModelPicker({ models, value, onChange }: { models: string[]; value: string; onChange: (model: string) => void }) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  const rootRef = useRef<HTMLDivElement>(null)
  const listId = useId()
  const visible = filterModels(models, query)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener("pointerdown", onPointerDown)
    return () => document.removeEventListener("pointerdown", onPointerDown)
  }, [open])

  useEffect(() => { setActive(Math.max(0, visible.indexOf(value))) }, [query, open]) // eslint-disable-line react-hooks/exhaustive-deps

  const pick = (model: string) => {
    onChange(model)
    setOpen(false)
    setQuery("")
  }

  return (
    <div ref={rootRef} className="relative min-w-0 flex-1">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        disabled={!models.length}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex h-10 w-full items-center gap-2.5 rounded-lg border bg-jolu-bg px-3 text-left text-[13px] outline-none focus-visible:border-jolu-accent",
          open ? "border-jolu-accent" : "border-jolu-border",
          "disabled:cursor-not-allowed",
        )}
      >
        <span className={cn("min-w-0 flex-1 truncate", value ? "font-mono text-jolu-text" : "text-jolu-muted")}>
          {value || (models.length ? "选择模型" : "请先获取模型")}
        </span>
        <ChevronDown aria-hidden className={cn("size-4 shrink-0 text-jolu-muted transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="motion-popover absolute inset-x-0 top-full z-10 mt-1.5 flex flex-col gap-1.5 rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover">
          <label className="flex h-8 items-center gap-2 rounded-md bg-jolu-side px-2">
            <Search aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
            <input
              autoFocus
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") { event.preventDefault(); setActive((i) => Math.min(i + 1, visible.length - 1)) }
                else if (event.key === "ArrowUp") { event.preventDefault(); setActive((i) => Math.max(i - 1, 0)) }
                else if (event.key === "Enter" && visible[active]) { event.preventDefault(); pick(visible[active]) }
                else if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false) }
              }}
              placeholder="搜索模型"
              aria-label="搜索模型"
              aria-activedescendant={visible[active] ? `${listId}-${active}` : undefined}
              className="min-w-0 flex-1 bg-transparent text-[11px] text-jolu-text outline-none placeholder:text-jolu-muted"
            />
          </label>
          <ul id={listId} role="listbox" aria-label="模型" className="max-h-[196px] overflow-y-auto">
            {visible.map((item, index) => (
              <li
                key={item}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={item === value}
                onMouseEnter={() => setActive(index)}
                onClick={() => pick(item)}
                className={cn(
                  "flex min-h-[34px] cursor-default items-center gap-2 rounded-[7px] px-2 py-1.5 font-mono text-xs",
                  item === value ? "bg-jolu-selected text-jolu-on" : index === active ? "bg-jolu-side text-jolu-text" : "text-jolu-text",
                )}
              >
                <span className="min-w-0 flex-1 truncate">{item}</span>
                {item === value && <Check aria-hidden className="size-[15px] shrink-0" />}
              </li>
            ))}
            {!visible.length && <li className="px-2 py-1.5 text-[11px] text-jolu-muted">没有匹配的模型</li>}
          </ul>
        </div>
      )}
    </div>
  )
}

function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string | null; children: (id: string) => ReactNode }) {
  const id = useId()
  return (
    <div className="flex flex-col gap-[7px]">
      <label htmlFor={id} className="text-xs font-medium text-jolu-text">{label}</label>
      {children(id)}
      {error ? <span className="text-[11px] text-jolu-danger">{error}</span> : hint && <span className="text-[11px] text-jolu-muted">{hint}</span>}
    </div>
  )
}

function TextInput({ id, value, onChange, onBlur, placeholder, type = "text", readOnly, disabled, mono, trailing }: {
  id: string
  value: string
  onChange: (value: string) => void
  onBlur?: () => void
  placeholder?: string
  type?: "text" | "password"
  readOnly?: boolean
  disabled?: boolean
  mono?: boolean
  trailing?: ReactNode
}) {
  return (
    <div className={cn(
      "flex h-10 items-center gap-2.5 rounded-lg border border-jolu-border px-3 focus-within:border-jolu-accent",
      readOnly ? "bg-jolu-side" : "bg-jolu-bg",
    )}>
      <input
        id={id}
        type={type}
        value={value}
        readOnly={readOnly}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        onBlur={onBlur}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        className={cn("min-w-0 flex-1 bg-transparent text-[13px] text-jolu-text outline-none placeholder:text-jolu-muted", mono && "font-mono", readOnly && "text-jolu-muted")}
      />
      {trailing}
    </div>
  )
}

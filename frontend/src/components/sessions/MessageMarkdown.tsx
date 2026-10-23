import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { openUrl } from "@/lib/api"
import { useToast } from "@/components/ui/Toast"

/** 本机消息只读渲染；不执行 HTML，不自动请求消息中的远程图片。 */
export function MessageMarkdown({ text }: { text: string }) {
  const toast = useToast()
  return <div className="min-w-0 break-words text-xs leading-[1.6] text-jolu-text [&>*+*]:mt-3 [&_h1]:text-lg [&_h2]:text-base [&_h3]:text-sm [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_h4]:font-semibold [&_h5]:font-semibold [&_h6]:font-semibold [&_ul]:list-disc [&_ol]:list-decimal [&_ul]:pl-5 [&_ol]:pl-5 [&_li+li]:mt-1 [&_blockquote]:border-l-2 [&_blockquote]:border-jolu-border [&_blockquote]:pl-3 [&_blockquote]:text-jolu-muted [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-jolu-stage [&_pre]:p-3 [&_pre]:whitespace-pre [&_code]:font-mono [&_code]:text-[11px] [&_hr]:border-jolu-border [&_th]:border [&_th]:border-jolu-border [&_th]:px-3 [&_th]:py-2 [&_td]:border [&_td]:border-jolu-border [&_td]:px-3 [&_td]:py-2">
    <Markdown remarkPlugins={[remarkGfm]} components={{
      a: ({ href, children }) => href && /^https?:\/\//i.test(href)
        ? <a href={href} className="text-jolu-on underline underline-offset-2" onClick={event => { event.preventDefault(); void openUrl(href).catch(() => toast.danger("打开链接失败", "请复制链接后重试")) }}>{children}</a>
        : <span>{children}</span>,
      img: ({ alt }) => <span className="text-jolu-muted">[图片：{alt || "未命名"}]</span>,
      table: ({ children }) => <div className="overflow-x-auto"><table className="w-full border-collapse text-left text-xs">{children}</table></div>,
    }}>{text}</Markdown>
  </div>
}

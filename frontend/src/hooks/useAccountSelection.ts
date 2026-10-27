import { useState } from "react"
import { retainAccountSelection } from "@/lib/accountActions"

export interface AccountSelection {
  active: boolean
  selectedIds: string[]
  selected: ReadonlySet<string>
  enter: () => void
  exit: () => void
  toggle: (id: string) => void
  selectAll: () => void
  clear: () => void
  discard: (ids: readonly string[]) => void
}

/** 选择始终绑定当前列表；在渲染中同步裁剪，避免筛选切换后仍能提交隐藏连接。 */
export function useAccountSelection(items: readonly { id: string }[]): AccountSelection {
  const [active, setActive] = useState(false)
  const [storedIds, setStoredIds] = useState<string[]>([])
  const availableIds = items.map(item => item.id)
  const selectedIds = retainAccountSelection(storedIds, availableIds)
  if (selectedIds.length !== storedIds.length) setStoredIds(selectedIds)

  return {
    active,
    selectedIds,
    selected: new Set(selectedIds),
    enter: () => { setStoredIds([]); setActive(true) },
    exit: () => { setStoredIds([]); setActive(false) },
    toggle: id => {
      if (!availableIds.includes(id)) return
      setStoredIds(current => {
        const retained = retainAccountSelection(current, availableIds)
        return retained.includes(id) ? retained.filter(item => item !== id) : [...retained, id]
      })
    },
    selectAll: () => setStoredIds([...new Set(availableIds)]),
    clear: () => setStoredIds([]),
    discard: ids => {
      const discarded = new Set(ids)
      setStoredIds(current => current.filter(id => !discarded.has(id)))
    },
  }
}

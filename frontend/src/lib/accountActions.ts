/** 替代账号可能因授权或配置写入失败而不可用，确认切换成功前不能删除原连接。 */
export async function removeAccountWithReplacement(
  id: string,
  replacement: string | null,
  actions: { enable: (id: string) => Promise<unknown>; remove: (id: string) => Promise<unknown> },
): Promise<void> {
  if (replacement) await actions.enable(replacement)
  await actions.remove(id)
}

export interface BatchAccountDeleteResult {
  deletedIds: string[]
  failed: { id: string; message: string }[]
}

/** 筛选改变后只留下当前可见连接，后续恢复筛选不会重新选中已清理项。 */
export function retainAccountSelection(selected: readonly string[], availableIds: readonly string[]): string[] {
  const available = new Set(availableIds)
  return [...new Set(selected)].filter(id => available.has(id))
}

/** 每个连接只删除一次，保留逐项结果；引用保护等失败由调用方明确展示与重试。 */
export async function removeAccounts(
  ids: readonly string[],
  remove: (id: string) => Promise<unknown>,
): Promise<BatchAccountDeleteResult> {
  const result: BatchAccountDeleteResult = { deletedIds: [], failed: [] }
  for (const id of new Set(ids)) {
    try {
      await remove(id)
      result.deletedIds.push(id)
    } catch (error) {
      result.failed.push({ id, message: error instanceof Error ? error.message : String(error) })
    }
  }
  return result
}

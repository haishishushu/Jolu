/** 替代账号可能因授权或配置写入失败而不可用，确认切换成功前不能删除原连接。 */
export async function removeAccountWithReplacement(
  id: string,
  replacement: string | null,
  actions: { enable: (id: string) => Promise<unknown>; remove: (id: string) => Promise<unknown> },
): Promise<void> {
  if (replacement) await actions.enable(replacement)
  await actions.remove(id)
}

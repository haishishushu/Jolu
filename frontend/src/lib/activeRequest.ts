/** 将请求结果交给当前视图；返回 effect 清理函数。 */
export function observeActiveRequest<T>(request: Promise<T>, onValue: (value: T) => void, onError: (reason: unknown) => void): () => void {
  let active = true
  void request.then(
    (value) => { if (active) onValue(value) },
    (reason: unknown) => { if (active) onError(reason) },
  )
  return () => { active = false }
}

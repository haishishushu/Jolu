/** 按行对比两段文本（Skills「查看差异」：磁盘版本 vs 当前草稿） */

export type DiffLine = { kind: "same" | "add" | "del"; text: string }

/** 裁掉公共前后缀后，中间部分超过这个行数时不做 LCS（O(n·m) 内存），退化为整段替换 */
const MAX_LINES = 5000

/** 统一换行符；末尾的单个换行只是行结束符，不算一个空行 */
function splitLines(text: string): string[] {
  if (text === "") return []
  const unified = text.replace(/\r\n?/g, "\n")
  return (unified.endsWith("\n") ? unified.slice(0, -1) : unified).split("\n")
}

/** `a` 是旧版本，`b` 是新版本：只在 `a` 里的行为 del，只在 `b` 里的行为 add；同一处替换时先 del 后 add */
export function lineDiff(a: string, b: string): DiffLine[] {
  const left = splitLines(a)
  const right = splitLines(b)
  // 先裁掉公共前后缀，只对中间变化的部分做 LCS
  let head = 0
  while (head < left.length && head < right.length && left[head] === right[head]) head++
  let tail = 0
  while (tail < left.length - head && tail < right.length - head && left[left.length - 1 - tail] === right[right.length - 1 - tail]) tail++
  const same = (text: string): DiffLine => ({ kind: "same", text })
  return [
    ...left.slice(0, head).map(same),
    ...middleDiff(left.slice(head, left.length - tail), right.slice(head, right.length - tail)),
    ...left.slice(left.length - tail).map(same),
  ]
}

function middleDiff(left: string[], right: string[]): DiffLine[] {
  if (left.length > MAX_LINES || right.length > MAX_LINES) {
    return [...left.map((text): DiffLine => ({ kind: "del", text })), ...right.map((text): DiffLine => ({ kind: "add", text }))]
  }
  return lcsDiff(left, right)
}

function lcsDiff(left: string[], right: string[]): DiffLine[] {
  const n = left.length
  const m = right.length
  // lcs[i][j]：left[i..] 与 right[j..] 的最长公共子序列长度，按行压平存放
  const width = m + 1
  const lcs = new Uint16Array((n + 1) * width)
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * width + j] = left[i] === right[j] ? lcs[(i + 1) * width + j + 1] + 1 : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1])
    }
  }
  const out: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (left[i] === right[j]) {
      out.push({ kind: "same", text: left[i] })
      i++
      j++
    } else if (lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
      out.push({ kind: "del", text: left[i++] })
    } else {
      out.push({ kind: "add", text: right[j++] })
    }
  }
  while (i < n) out.push({ kind: "del", text: left[i++] })
  while (j < m) out.push({ kind: "add", text: right[j++] })
  return out
}

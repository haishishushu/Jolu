import assert from 'node:assert/strict'
import test from 'node:test'
import {pushToast, markToastExiting, removeToast, toastRemaining} from './toastQueue.ts'

test('所有颜色均在 5 秒后到期', () => {
  for (const tone of ['success', 'warn', 'danger', 'info']) {
    const [item] = pushToast([], {tone, title: '提示'}, 1, 1000)
    assert.equal(toastRemaining(item, 5999), 1)
    assert.equal(toastRemaining(item, 6000), 0)
  }
})
test('连续提示只保留最新一条，详情较短时也不露出旧提示', () => {
  let list = []
  for (let id = 1; id <= 20; id++) list = pushToast(list, {tone: 'warn', title: `${id}`}, id, id * 100)
  assert.equal(list.length, 1)
  assert.equal(list[0].title, '20')
  assert.equal(list[0].createdAt, 2000)
})
test('重复内容续时并保留节点，详情变化则立即替换', () => {
  const input = {tone: 'success', title: '完成', detail: 'A'}
  const first = pushToast([], input, 1, 1000)
  const again = pushToast(first, input, 2, 2000)
  assert.equal(again[0].id, 1)
  assert.equal(toastRemaining(again[0], 6000), 1000)
  const changed = pushToast(again, {...input, detail: 'B'}, 3, 2500)
  assert.equal(changed.length, 1)
  assert.equal(changed[0].id, 3)
})
test('旧提示的到期或退场回调不能删除替换后的新提示', () => {
  const old = markToastExiting(pushToast([], {tone: 'danger', title: '旧'}, 1, 0), 1)
  const next = pushToast(old, {tone: 'success', title: '新'}, 2, 5000)
  assert.deepEqual(removeToast(markToastExiting(next, 1), 1), next)
  assert.equal(next[0].exiting, false)
  assert.equal(removeToast(markToastExiting(next, 2), 2).length, 0)
})

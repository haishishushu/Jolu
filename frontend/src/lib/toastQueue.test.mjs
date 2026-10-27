import assert from 'node:assert/strict'
import test from 'node:test'
import * as queue from './toastQueue.ts'
const {pushToast, markToastExiting, removeToast, toastRemaining, visibleToasts, pauseToast} = queue

test('success/info 3s, warning 6s, errors stay until dismissed', () => {
  for (const [tone, duration] of [['success', 3000], ['info', 3000], ['warn', 6000]]) {
    const [item] = pushToast([], {tone, title: 'notice'}, 1, 1000)
    assert.equal(toastRemaining(item, 1000 + duration - 1), 1)
    assert.equal(toastRemaining(item, 1000 + duration), 0)
  }
  assert.equal(toastRemaining(pushToast([], {tone:'danger', title:'failure'}, 1, 0)[0], 999999), Infinity)
  assert.equal(toastRemaining(pushToast([], {tone:'warn', title:'pending navigation',persistent:true}, 1, 0)[0],999999),Infinity)
})

test('three slots include exiting cards; queued notices start counting when shown; success retains errors', () => {
  let list = pushToast([], {tone:'danger', title:'failure'}, 1, 0)
  for (let id=2; id<=5; id++) list = pushToast(list, {tone:'success', title:`done ${id}`}, id, 0)
  assert.deepEqual(visibleToasts(list).map(x=>x.id), [1,2,3])
  list = markToastExiting(list, 2)
  assert.equal(visibleToasts(list).length, 3)
  list = removeToast(list, 2, 10000)
  assert.deepEqual(visibleToasts(list).map(x=>x.id), [1,3,4])
  assert.equal(toastRemaining(list.find(x=>x.id===4), 10000), 3000)
  assert.ok(list.some(x=>x.id===1))
})

test('duplicates keep their node, increment count and renew time; distinct details stay independent', () => {
  const input = {tone:'success', title:'done', detail:'A'}
  const first = pushToast([], input, 1, 1000)
  const again = pushToast(first, input, 2, 2000)
  assert.equal(again[0].id, 1)
  assert.equal(again[0].count, 2)
  assert.equal(toastRemaining(again[0], 4999), 1)
  const changed = pushToast(again, {...input, detail:'B'}, 3, 2500)
  assert.equal(changed.length, 2)
})

test('pointer and focus pause independently, then resume the remaining time', () => {
  let list = pushToast([], {tone:'success', title:'done'}, 1, 1000)
  list = pauseToast(list, 1, 'pointer', true, 2000)
  list = pauseToast(list, 1, 'focus', true, 2500)
  list = pauseToast(list, 1, 'pointer', false, 9000)
  assert.equal(toastRemaining(list[0], 20000), 2000)
  list = pauseToast(list, 1, 'focus', false, 20000)
  assert.equal(toastRemaining(list[0], 21999), 1)
  assert.equal(toastRemaining(list[0], 22000), 0)
})

test('duplicates do not resume hovered cards; stale callbacks cannot dismiss a new card', () => {
  const input = {tone:'info', title:'notice'}
  let list = pauseToast(pushToast([], input, 1, 0), 1, 'pointer', true, 1000)
  list = pushToast(list, input, 2, 2000)
  assert.equal(toastRemaining(list[0], 50000), 3000)
  list = markToastExiting(list, 1)
  list = pushToast(list, input, 3, 2100)
  list = removeToast(list, 1, 2200)
  assert.deepEqual(list.map(x=>x.id), [3])
  assert.deepEqual(removeToast(markToastExiting(list, 1), 1, 2300), list)
})

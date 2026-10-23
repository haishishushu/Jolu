import assert from 'node:assert/strict'
import test from 'node:test'
import { handoffFileName, dedupeFileName, dialogMode, reduceChat, formatTokens, fileNameError } from './sessionHandoff.ts'
test('交接文件名与非法字符', () => {
  assert.equal(handoffFileName('jolu', '会话界面优化', 'Claude', new Date(2026,9,21,16,48)), 'jolu_会话界面优化_交接-Claude_20261021-1648.md')
  assert.match(handoffFileName('p', 'a/b:c', 'AI', new Date()), /a-b-c/)
  assert.equal(dedupeFileName('a.md', n => n === 'a.md'), 'a-2.md')
  assert.ok(fileNameError('CON.md'))
  assert.ok(fileNameError('../a.md'))
  assert.equal(fileNameError('a.md'), null)
})
test('四种状态及复用优先', () => {
  assert.equal(dialogMode({ readable:false, reusable:false, latest:null }), 'unreadable')
  assert.equal(dialogMode({ readable:true, reusable:false, latest:null }), 'first')
  assert.equal(dialogMode({ readable:true, reusable:false, latest:{} }), 'incremental')
  assert.equal(dialogMode({ readable:true, reusable:true, latest:{} }), 'reuse')
})
test('同阶段进度更新，草稿保留版本与警告', () => {
  let items = reduceChat([], { type:'step', text:'整理纪要', done:1, total:3 })
  items = reduceChat(items, { type:'step', text:'整理纪要', done:2, total:3 })
  assert.equal(items.length, 1)
  assert.equal(items[0].done, 2)
  items = reduceChat(items, { type:'draft', round:1, narrative_md:'草稿', path_warnings:['x'], input_tokens:12, output_tokens:4 })
  assert.equal(items[1].round, 1)
  assert.deepEqual(items[1].warnings,['x'])
  assert.equal(formatTokens(1234),'1.2k')
  assert.equal(formatTokens(999),'999')
})

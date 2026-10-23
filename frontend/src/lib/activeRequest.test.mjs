import test from "node:test"
import assert from "node:assert/strict"
import { observeActiveRequest } from "./activeRequest.ts"

test("切换文件后，迟到的旧成功结果不能覆盖新文件", async () => {
  const old = Promise.withResolvers()
  const next = Promise.withResolvers()
  let content = null
  const errors = []
  const cancel = observeActiveRequest(old.promise, (value) => { content = value }, (error) => errors.push(error))
  cancel()
  observeActiveRequest(next.promise, (value) => { content = value }, (error) => errors.push(error))
  next.resolve("new.md")
  await next.promise
  old.resolve("old.md")
  await old.promise
  assert.equal(content, "new.md")
  assert.deepEqual(errors, [])
})

test("离开视图后忽略迟到错误，当前请求仍能展示失败", async () => {
  const old = Promise.withResolvers()
  const next = Promise.withResolvers()
  const errors = []
  const cancel = observeActiveRequest(old.promise, () => {}, (error) => errors.push(error))
  cancel()
  observeActiveRequest(next.promise, () => {}, (error) => errors.push(error))
  old.reject("old failure")
  next.reject("current failure")
  await Promise.allSettled([old.promise, next.promise])
  assert.deepEqual(errors, ["current failure"])
})

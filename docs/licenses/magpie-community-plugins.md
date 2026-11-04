# magpie-community/plugins 许可

Jolu 模型入口的部分协议解析逻辑参考或移植自 [magpie-community/plugins](https://github.com/magpie-community/plugins)。Trae 模型目录相关实现位于 `backend/src/gateway/adapter/trae_models.rs`，参考插件的 `batchLists`、`listOf`、`chatModel` 与 `devModel`；对话相关实现位于同目录 `trae_chat.rs`、`trae_stream.rs` 与 `trae.rs`，参考 `chatBody`、`nativeTools`、`conversationID`、`parts`、`tokensOf`、`NativeCalls` 和 SSE 处理。Jolu 另加账号授权绑定、严格失败、取消释放与用量未知值处理，不宣称完整等价移植。以下保留原仓库 MIT 许可；这不代表供应商对第三方客户端兼容性或服务使用权限的承诺。

WorkBuddy 请求兼容实现位于 `backend/src/gateway/adapter/workbuddy_request.rs`，参考 `packages/workbuddy/index.mjs` 的 `FLAGGED`、`unflag`、`systemOn` 与 `asReasoning`，对照版本为 `9bba5fedc2a9aa3aeeaa5571f0052ea1c89dac9c`。仅移植请求文本和基础字段兼容，不包含插件的全部工具转换、重试和系统提示降级策略。

```text
MIT License

Copyright (c) 2026 magpie-community contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

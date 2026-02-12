# luars 与 C Lua 5.5 的行为差异

本文档记录了 luars（Rust 实现）与官方 C Lua 5.5 之间的全部已知行为差异。

---

## 1. 长度运算符 `#t`

**C Lua 5.5**：`#t` 返回表的一个"有效边界"（valid border），对于稀疏表，结果是**未定义**的——可能返回数组部分的任意有效边界，并在必要时搜索哈希部分的延续键。

**luars**：`#t` 始终返回数组有效长度（即 `lenhint`）。只追踪数组部分，不搜索哈希部分。行为是**确定性的**，对稀疏表的结果可能与 C Lua 不同。

---

## 2. table 标准库不支持元方法

以下函数全部使用原始访问（`rawget` / `rawset` / `rawlen`），**不触发** `__index`、`__newindex`、`__len` 元方法：

- `table.insert`
- `table.remove`
- `table.sort`
- `table.concat`
- `table.move`
- `table.unpack`

**C Lua 5.5**：这些函数通过通用 API 操作表，会触发相应元方法。

---

## 3. `pairs()` 不支持 `__pairs` 元方法

**C Lua 5.5**：`pairs(t)` 会检查 `__pairs` 元方法；如果存在，调用它并返回最多 4 个值（含 to-be-closed 变量）。

**luars**：`pairs(t)` 始终返回 `next, t, nil`（3 个值），不检查 `__pairs` 元方法，不支持 to-be-closed 变量作为第 4 返回值。

---

## 4. `ipairs()` 不支持 `__index` 元方法

**C Lua 5.5**：`ipairs` 的迭代器通过通用表访问获取值，会触发 `__index` 元方法。

**luars**：`ipairs` 的迭代器使用 `raw_geti()` 直接读取数组，不触发 `__index`。

---

## 5. 无 C API / testC 支持

luars 是纯 Rust 实现，不提供 C API（`lua_State*` 等），也不支持 `T`（testC）测试库。

以下官方测试文件中依赖 testC 的部分全部跳过：
- `api.lua`（整个文件）
- `memerr.lua`（整个文件）
- `coroutine.lua`（C API 相关测试）
- `events.lua`（C API 相关测试）
- `errors.lua`（C 函数消息测试）
- `gc.lua`（userdata GC 测试）
- `strings.lua`（pushfstring、外部字符串测试）
- `nextvar.lua`（非表上的 table 库测试）
- `code.lua`（整个文件，opcode 测试）

---

## 6. 无 C 模块加载支持

- `package.loadlib` 始终返回错误 `"loadlib not implemented"`。
- `package.cpath` 搜索器（searcher 3）始终返回错误，无法加载 `.so` / `.dll` 形式的 C 模块。

---

## 7. 无 `string.dump` 的二进制块加载

luars 的编译器生成自有字节码格式。`string.dump` 生成的二进制块格式与 C Lua 不同，不能互相加载。但是可以被自己的`load`函数加载, `calls.lua` 中的二进制块测试已跳过。

---

## 8. debug 库限制

### 8.1 debug.sethook / debug.gethook
`debug.sethook` 接受参数但**不执行任何操作**（stub 实现）。`debug.gethook` 始终返回 nil。

这导致：
- 钩子函数（call/return/line/count hooks）不工作
- `locals.lua` 中 "close vs return hooks" 测试已跳过

### 8.2 其他
- `debug.getinfo`、`debug.getlocal`、`debug.setlocal`、`debug.getupvalue`、`debug.setupvalue`、`debug.upvalueid`、`debug.upvaluejoin`、`debug.traceback`、`debug.getmetatable`、`debug.setmetatable`、`debug.getregistry` 已实现。

---

## 9. `warn()` 简化实现

**C Lua 5.5**：`warn` 支持 `@on` / `@off` 控制开关及消息拼接协议（以 `@` 开头的控制消息）。

**luars**：`warn` 直接将参数输出到 stderr，不支持 `@on` / `@off` 控制，不支持消息拼接协议。

---

## 10. 字符串库差异

### 10.1 字符串内部编码
luars 字符串使用 UTF-8 编码，不支持任意二进制字节。模式匹配中的 `\255` 等非 UTF-8 字节转义不可用。`pm.lua` 中相关测试已跳过。luars有专有的二进制类型 `binary`，支持任意字节，但不与字符串互操作。

### 10.2 `string.format("%c", ...)`
`%c` 格式化符的行为与 C Lua 存在差异。`strings.lua` 中相关测试已跳过。

### 10.3 长字符串复用
C Lua 会对相同长字符串常量进行地址复用（`const` 标记辅助）。luars 不做此优化。`literals.lua` 中相关测试已跳过。

---

## 11. 不支持 locale 相关数字解析

**C Lua 5.5**：支持通过 `os.setlocale` 改变小数点字符（如 `pt_BR` locale 下 `3,4` 解析为 `3.4`）。

**luars**：数字解析始终使用 `.` 作为小数点，不受 locale 影响。`literals.lua` 中相关测试已跳过。

---

## 12. os 库限制

- `os.date` 中 `isdst`（夏令时标志）未正确实现，始终为默认值。

---

## 13. 命令行选项限制

- `-E` 选项（忽略环境变量）被接受但不执行任何操作。
- `-W` 选项（开启警告）被接受但不执行任何操作。

---

## 14. GC 相关差异

### 14.1 `__gc` 终结器标志
`__gc` 终结器执行时，未保存和恢复 `L->allowhook`，未设置 `ci->callstatus |= CIST_FIN` 标志。

### 14.2 栈收缩
GC 期间未实现栈收缩（stack shrinking）。

---

## 15. 编译器差异

### 15.1 位运算与整数除法语法（Lua 5.3 兼容）
编译器在遇到 Lua 5.3 风格的位运算符（`&`、`|`、`~`、`<<`、`>>`）和整数除法（`//`）时会报特定的"不支持"错误，而不是通用语法错误。

> 注意：Lua 5.5 本身也移除了这些运算符，改用函数调用。此处差异仅在错误消息文本上。

### 15.2 负数表索引
存在一个已知的编译器 bug，涉及负数表索引的字节码生成。具体测试已禁用。

---

## 16. main.lua 和 db.lua 测试跳过

- `main.lua`：测试命令行解释器交互行为（stdin 提示符、命令行参数处理等），在 all.lua 中跳过。
- `db.lua`：debug 库的完整测试，因 hook 等功能不完整而在 all.lua 中跳过。

---

## 17. 字节码 dump 格式

`string.dump` 输出 luars 自有字节码格式，与 C Lua 的字节码不兼容。upvalue 名称在 dump 输出中暂未包含。

---

## 18. nextvar.lua 中跳过的测试段

`test.lua`（nextvar.lua 的副本）中以下 5 段测试用 `if false then ... end` 跳过：

1. **table 库元方法测试**（~第 609 行）—— table.insert/sort/concat/remove/unpack 使用 `__len`/`__index`/`__newindex` 代理
2. **table.insert 溢出测试**（~第 658 行）—— 使用 `__len` 返回 `math.maxinteger`
3. **`__pairs` 元方法测试**（~第 912 行）—— pairs 触发自定义迭代器 + to-be-closed
4. **ipairs + `__index` 元方法测试**（~第 930 行）—— ipairs 通过 `__index` 读取虚拟元素
5. **yield inside `__pairs` 测试**（~第 943 行）—— 在 `__pairs` 返回的迭代器中 yield

---

## 总结

luars 的设计取舍：
- **table 操作全部使用原始访问**，不经过元方法，获得更好的性能和更简单的实现
- **`#t` 运算符确定性化**，始终返回数组部分的 lenhint，放弃 C Lua 的"未定义行为"边界搜索
- **纯 Rust 实现**，无 C FFI，因此无法加载 C 模块，无 testC 支持
- **UTF-8 字符串**，不支持任意二进制字节
- **debug hooks 为 stub**，不影响普通 Lua 程序运行
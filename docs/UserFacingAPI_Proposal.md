# User-Facing API Design Proposal (mlua-inspired)

> **Status**: Draft  
> **Date**: 2026-02-21

## 1. Motivation

当前 `lua-rs` 的用户 API 围绕 `LuaValue`（一个 16 字节的 tagged union）设计。`LuaValue` 是 VM 内部的值表示，
直接暴露给用户存在以下问题：

1. **生命周期不安全**：`LuaValue` 内含 GC 对象的裸指针（`TablePtr`、`FunctionPtr` 等），
   用户持有的 `LuaValue` 随时可能因 GC 回收而悬空。目前必须通过 `create_ref` 手动将值注册到
   registry 中，并在不需要时手动 `release_ref`——容易忘记导致内存泄漏或 dangling pointer。

2. **操作不便**：要操作一个表，用户需要反复调用 `vm.raw_set(&table, key, val)` 这样的自由函数，
   而不是自然的 `table.set(key, val)`。

3. **无法安全存储**：用户想把一个 Lua 表存到自己的 struct 里长期使用，目前没有安全的途径。

**目标**：参考 [mlua](https://github.com/mlua-rs/mlua) 的设计，提供一组 **用户级包装类型**
（`LuaTableRef`、`LuaFunctionRef`、`LuaStringRef`、`LuaAnyRef`），它们：

- 内部持有 `RefId`（registry 引用）+ `*mut LuaVM` 指针
- 实现 `Drop` 自动释放引用（RAII）
- 标记为 `!Send + !Sync`（不能跨线程传递）
- 提供友好的方法 API（`table.get("key")`、`func.call(args)`）
- 支持将任意第三方 struct 传入 Lua（`AnyUserData` 方案）

## 2. Core Design

### 2.1 Architecture Overview

```
用户代码                    包装层                       VM 内部
─────────                ──────────                  ──────────
LuaTableRef  ───────►  RefId + *mut LuaVM  ────►  Registry[ref_id] → GcTable
LuaFunctionRef ─────►  RefId + *mut LuaVM  ────►  Registry[ref_id] → GcFunction
LuaStringRef ───────►  RefId + *mut LuaVM  ────►  Registry[ref_id] → GcString
LuaAnyRef    ───────►  RefId + *mut LuaVM  ────►  Registry[ref_id] → LuaValue (any)
```

### 2.2 内部结构

所有包装类型共享相同的内部结构：

```rust
use std::marker::PhantomData;

/// 所有 Ref 类型的共用内核
struct RefInner {
    /// Registry 中的引用 ID（由 RefManager 分配）
    ref_id: RefId,
    /// 指向所属 LuaVM 的裸指针（用于 Drop 时自动释放）
    vm: *mut LuaVM,
    /// 标记为 !Send + !Sync
    _marker: PhantomData<*const ()>,
}

impl Drop for RefInner {
    fn drop(&mut self) {
        // 安全性：RefInner 是 !Send，只能在创建它的线程上 drop
        // vm 指针在 LuaVM 存活期间始终有效
        unsafe {
            (*self.vm).release_ref_id(self.ref_id);
        }
    }
}
```

### 2.3 !Send + !Sync 保证

通过 `PhantomData<*const ()>` 自动使类型成为 `!Send + !Sync`。
这意味着：

- ✅ 可以在同一线程内自由传递和存储
- ❌ 不能 `Send` 到其他线程
- ❌ 不能通过 `Arc<Mutex<>>` 跨线程共享

这与 mlua 的行为一致，保证了 GC 安全性（Lua VM 本身不是线程安全的）。

### 2.4 与现有 API 的关系

新的 Ref 类型是**增量添加**，不破坏现有 API：

| 现有 API | 新 API | 关系 |
|---------|--------|------|
| `LuaValue` | 不变 | VM 内部值表示，仍然是核心类型 |
| `create_ref()` → `LuaRefValue` | `vm.to_ref(value)` → `LuaAnyRef` | 自动 Drop，无需手动 release |
| `vm.raw_set(&table, k, v)` | `table_ref.set("key", value)` | 方法调用更自然 |
| `vm.call(func, args)` | `func_ref.call(args)` | 方法调用更自然 |
| `vm.table_pairs(&table)` | `table_ref.pairs()` | 方法调用更自然 |

## 3. Wrapper Types

### 3.1 LuaTableRef

持有一个 Lua table 的引用，提供表操作方法。

```rust
pub struct LuaTableRef {
    inner: RefInner,
}

impl LuaTableRef {
    // ==================== 读取 ====================

    /// 获取字符串键对应的值
    pub fn get(&self, key: &str) -> LuaResult<LuaValue>;

    /// 获取整数键对应的值
    pub fn geti(&self, key: i64) -> LuaResult<LuaValue>;

    /// 获取任意 LuaValue 键对应的值
    pub fn get_value(&self, key: &LuaValue) -> LuaResult<LuaValue>;

    /// 获取值并转换为 Rust 类型
    pub fn get_as<T: FromLua>(&self, key: &str) -> LuaResult<T>;

    // ==================== 写入 ====================

    /// 设置字符串键的值
    pub fn set(&self, key: &str, value: impl IntoLua) -> LuaResult<()>;

    /// 设置整数键的值
    pub fn seti(&self, key: i64, value: impl IntoLua) -> LuaResult<()>;

    /// 设置任意键值对
    pub fn set_value(&self, key: LuaValue, value: LuaValue) -> LuaResult<()>;

    // ==================== 遍历 ====================

    /// 获取所有键值对（快照）
    pub fn pairs(&self) -> LuaResult<Vec<(LuaValue, LuaValue)>>;

    /// 获取数组长度（等同于 Lua 的 #t）
    pub fn len(&self) -> LuaResult<usize>;

    /// 追加值到数组末尾（等同于 table.insert）
    pub fn push(&self, value: impl IntoLua) -> LuaResult<()>;

    // ==================== 转换 ====================

    /// 获取底层 LuaValue（从 registry 取出）
    pub fn to_value(&self) -> LuaValue;

    /// 获取 RefId（高级用法）
    pub fn ref_id(&self) -> RefId;
}
```

### 3.2 LuaFunctionRef

持有一个 Lua function（Lua 闭包、C 函数、Rust 闭包均可）的引用。

```rust
pub struct LuaFunctionRef {
    inner: RefInner,
}

impl LuaFunctionRef {
    /// 同步调用函数
    pub fn call(&self, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>;

    /// 同步调用并取第一个返回值
    pub fn call1(&self, args: Vec<LuaValue>) -> LuaResult<LuaValue>;

    /// 异步调用函数
    pub async fn call_async(&self, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>;

    /// 获取底层 LuaValue
    pub fn to_value(&self) -> LuaValue;

    /// 获取 RefId
    pub fn ref_id(&self) -> RefId;
}
```

### 3.3 LuaStringRef

持有一个 Lua string 的引用（短字符串已经 interned，长字符串需要 GC 保护）。

```rust
pub struct LuaStringRef {
    inner: RefInner,
}

impl LuaStringRef {
    /// 获取字符串内容（&str 的生命周期绑定到 self）
    pub fn as_str(&self) -> &str;

    /// 转为 owned String
    pub fn to_string_lossy(&self) -> String;

    /// 获取字节长度
    pub fn len(&self) -> usize;

    /// 获取底层 LuaValue
    pub fn to_value(&self) -> LuaValue;
}
```

### 3.4 LuaAnyRef

通用引用类型，可以持有**任何** Lua 值。用于不确定类型时的通用存储。

```rust
pub struct LuaAnyRef {
    inner: RefInner,
}

impl LuaAnyRef {
    /// 获取底层 LuaValue
    pub fn to_value(&self) -> LuaValue;

    /// 尝试转换为 LuaTableRef（如果底层是 table）
    pub fn as_table(&self) -> Option<LuaTableRef>;

    /// 尝试转换为 LuaFunctionRef（如果底层是 function）
    pub fn as_function(&self) -> Option<LuaFunctionRef>;

    /// 尝试转换为 LuaStringRef（如果底层是 string）
    pub fn as_string(&self) -> Option<LuaStringRef>;

    /// 获取值的类型
    pub fn kind(&self) -> LuaValueKind;

    /// 尝试提取为 Rust 类型
    pub fn get_as<T: FromLua>(&self) -> LuaResult<T>;

    /// 获取 RefId
    pub fn ref_id(&self) -> RefId;
}
```

### 3.5 LuaVM 上的工厂方法

```rust
impl LuaVM {
    // ==================== 创建 Ref ====================

    /// 将任意 LuaValue 包装为 LuaAnyRef（自动注册到 registry）
    pub fn to_ref(&mut self, value: LuaValue) -> LuaAnyRef;

    /// 将 table 类型的 LuaValue 包装为 LuaTableRef
    /// 如果 value 不是 table 则返回 None
    pub fn to_table_ref(&mut self, value: LuaValue) -> Option<LuaTableRef>;

    /// 将 function 类型的 LuaValue 包装为 LuaFunctionRef
    pub fn to_function_ref(&mut self, value: LuaValue) -> Option<LuaFunctionRef>;

    /// 将 string 类型的 LuaValue 包装为 LuaStringRef
    pub fn to_string_ref(&mut self, value: LuaValue) -> Option<LuaStringRef>;

    /// 创建一个新空表并返回 LuaTableRef
    pub fn create_table_ref(&mut self, array: usize, hash: usize) -> LuaResult<LuaTableRef>;

    /// 通过 TableBuilder 创建表并返回 LuaTableRef
    pub fn build_table_ref(&mut self, builder: TableBuilder) -> LuaResult<LuaTableRef>;

    // ==================== 全局变量快捷方法 ====================

    /// 获取全局变量作为 LuaTableRef
    pub fn get_global_table(&mut self, name: &str) -> LuaResult<Option<LuaTableRef>>;

    /// 获取全局变量作为 LuaFunctionRef
    pub fn get_global_function(&mut self, name: &str) -> LuaResult<Option<LuaFunctionRef>>;
}
```

## 4. Third-Party Struct Passing

### 4.1 Problem Statement

当前 `UserDataTrait` 要求实现者控制类型定义（通过 `#[derive(LuaUserData)]`）。
但用户经常需要将**第三方库的 struct**（如 `reqwest::Response`、`tokio::fs::File`）
传入 Lua，而这些类型无法添加 derive 宏。

### 4.2 Solution: `OpaqueUserData<T>`

提供一个泛型包装器，能将任意 `T: 'static` 包装成 userdata，无需实现任何 trait：

```rust
/// 将任意 Rust 类型包装为 Lua userdata 的透明容器。
///
/// 不提供字段访问或元方法——纯粹的"黑盒"存储。
/// 用户在 Rust 侧通过 downcast 取回原始类型。
pub struct OpaqueUserData<T: 'static> {
    value: T,
}

// 自动实现 UserDataTrait（最小化实现）
impl<T: 'static> UserDataTrait for OpaqueUserData<T> {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn as_any(&self) -> &dyn std::any::Any { &self.value }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { &mut self.value }

    // 所有其他方法使用默认实现（返回 None / 不支持）
}
```

### 4.3 LuaVM 上的快捷方法

```rust
impl LuaVM {
    /// 将任意 Rust 值推入 Lua 作为 opaque userdata。
    ///
    /// 返回的 LuaValue 在 Lua 中表现为 userdata，
    /// 可以存储在表中、作为参数传递等，但 Lua 侧无法访问其字段。
    /// Rust 侧通过 downcast 取回。
    ///
    /// # Example
    /// ```ignore
    /// // 第三方类型
    /// let client = reqwest::Client::new();
    /// let ud = vm.push_any(client)?;
    /// vm.set_global("http_client", ud)?;
    ///
    /// // 后续在 Rust 回调中取回
    /// fn handle(state: &mut LuaState) -> LuaResult<usize> {
    ///     let ud = state.get_userdata::<reqwest::Client>(1)?;
    ///     // 使用 client...
    ///     Ok(0)
    /// }
    /// ```
    pub fn push_any<T: 'static>(&mut self, value: T) -> LuaResult<LuaValue>;

    /// push_any 的带元表版本，允许在 Lua 侧自定义行为
    pub fn push_any_with_metatable<T: 'static>(
        &mut self,
        value: T,
        metatable: LuaValue,
    ) -> LuaResult<LuaValue>;
}
```

### 4.4 ConfigurableUserData：可配置的第三方类型导出

对于想要给第三方类型提供部分 Lua 访问能力的场景，提供 builder 模式：

```rust
/// 为第三方类型构建自定义 userdata 行为。
///
/// # Example
/// ```ignore
/// // 为 std::net::SocketAddr 创建可访问的 userdata
/// let addr = "127.0.0.1:8080".parse::<std::net::SocketAddr>().unwrap();
///
/// let ud = UserDataBuilder::new(addr)
///     .add_field_getter("ip", |a| UdValue::Str(a.ip().to_string()))
///     .add_field_getter("port", |a| UdValue::Integer(a.port() as i64))
///     .add_method("to_string", |a, _args| vec![UdValue::Str(a.to_string())])
///     .set_tostring(|a| a.to_string())
///     .build(&mut vm)?;
/// ```
pub struct UserDataBuilder<T: 'static> {
    value: T,
    field_getters: HashMap<String, Box<dyn Fn(&T) -> UdValue>>,
    field_setters: HashMap<String, Box<dyn Fn(&mut T, UdValue)>>,
    methods: HashMap<String, CFunction>,
    tostring: Option<Box<dyn Fn(&T) -> String>>,
}

impl<T: 'static> UserDataBuilder<T> {
    pub fn new(value: T) -> Self;
    pub fn add_field_getter(self, name: &str, f: impl Fn(&T) -> UdValue + 'static) -> Self;
    pub fn add_field_setter(self, name: &str, f: impl Fn(&mut T, UdValue) + 'static) -> Self;
    pub fn set_tostring(self, f: impl Fn(&T) -> String + 'static) -> Self;
    pub fn build(self, vm: &mut LuaVM) -> LuaResult<LuaValue>;
}
```

## 5. API Reference

### 5.1 完整类型层次

```
LuaVM
 ├── to_ref(LuaValue)            → LuaAnyRef
 ├── to_table_ref(LuaValue)      → Option<LuaTableRef>
 ├── to_function_ref(LuaValue)   → Option<LuaFunctionRef>
 ├── to_string_ref(LuaValue)     → Option<LuaStringRef>
 ├── create_table_ref(a, h)      → LuaResult<LuaTableRef>
 ├── get_global_table(name)      → LuaResult<Option<LuaTableRef>>
 ├── get_global_function(name)   → LuaResult<Option<LuaFunctionRef>>
 ├── push_any<T>(value)          → LuaResult<LuaValue>
 └── push_any_with_metatable<T>  → LuaResult<LuaValue>

LuaTableRef (!Send, !Sync, Drop)
 ├── get(key: &str)              → LuaResult<LuaValue>
 ├── geti(key: i64)              → LuaResult<LuaValue>
 ├── get_value(key: &LuaValue)   → LuaResult<LuaValue>
 ├── get_as<T>(key: &str)        → LuaResult<T>
 ├── set(key, value)             → LuaResult<()>
 ├── seti(key, value)            → LuaResult<()>
 ├── set_value(key, value)       → LuaResult<()>
 ├── pairs()                     → LuaResult<Vec<(LuaValue, LuaValue)>>
 ├── len()                       → LuaResult<usize>
 ├── push(value)                 → LuaResult<()>
 ├── to_value()                  → LuaValue
 └── ref_id()                    → RefId

LuaFunctionRef (!Send, !Sync, Drop)
 ├── call(args)                  → LuaResult<Vec<LuaValue>>
 ├── call1(args)                 → LuaResult<LuaValue>
 ├── call_async(args)            → LuaResult<Vec<LuaValue>>  [async]
 ├── to_value()                  → LuaValue
 └── ref_id()                    → RefId

LuaStringRef (!Send, !Sync, Drop)
 ├── as_str()                    → &str
 ├── to_string_lossy()           → String
 ├── len()                       → usize
 └── to_value()                  → LuaValue

LuaAnyRef (!Send, !Sync, Drop)
 ├── to_value()                  → LuaValue
 ├── as_table()                  → Option<LuaTableRef>
 ├── as_function()               → Option<LuaFunctionRef>
 ├── as_string()                 → Option<LuaStringRef>
 ├── kind()                      → LuaValueKind
 ├── get_as<T>()                 → LuaResult<T>
 └── ref_id()                    → RefId
```

### 5.2 内部实现要点

1. **RefInner 是唯一的核心**：所有 Ref 类型都是 `RefInner` 的 newtype wrapper，
   零额外开销（编译后与裸 RefId + *mut LuaVM 相同大小）。

2. **所有可变操作通过 `unsafe` 解引用 `*mut LuaVM`**：这是安全的，因为：
   - `!Send` 保证不跨线程
   - Ref 的生命周期语义上不超过 LuaVM（用户需保证 VM 存活）
   - 与 mlua 的 `Lua<'lua>` 借用不同，我们选择裸指针方案以避免生命周期标注传播

3. **Clone 语义**：Ref 类型**不实现 Clone**。如需多处引用同一对象，
   调用 `vm.to_ref(ref.to_value())` 创建新的 registry 引用。
   或者后续考虑引用计数方案。

4. **GC 安全**：值存在 registry 中，registry 是 GC root，
   因此 Ref 持有期间对应的 GC 对象不会被回收。

## 6. Usage Examples

### 6.1 基本用法：表操作

```rust
use luars::{LuaVM, SafeOption, Stdlib, LuaValue};

fn main() -> luars::LuaResult<()> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // 在 Lua 侧创建一个表
    vm.execute(r#"
        config = {
            host = "localhost",
            port = 8080,
            debug = true,
            tags = {"web", "api"}
        }
    "#)?;

    // 获取全局表的 Ref
    let config = vm.get_global_table("config")?.unwrap();

    // 读取字段
    let host: String = config.get_as("host")?;
    let port: i64 = config.get_as("port")?;
    println!("Server: {}:{}", host, port);  // Server: localhost:8080

    // 修改字段
    config.set("port", LuaValue::integer(9090))?;
    config.set("debug", LuaValue::boolean(false))?;

    // 遍历
    for (k, v) in config.pairs()? {
        println!("{} = {}", k, v);
    }

    // config 在这里自动 Drop，释放 registry 引用
    Ok(())
}
```

### 6.2 函数引用

```rust
fn main() -> luars::LuaResult<()> {
    let mut vm = LuaVM::new(SafeOption::default());

    vm.execute(r#"
        function greet(name)
            return "Hello, " .. name .. "!"
        end
    "#)?;

    // 获取函数引用
    let greet = vm.get_global_function("greet")?.unwrap();

    // 调用——无需每次 get_global
    let msg = vm.create_string("World")?;
    let result = greet.call(vec![msg])?;
    println!("{}", result[0]);  // Hello, World!

    // 多次调用
    let msg2 = vm.create_string("Rust")?;
    let result2 = greet.call1(vec![msg2])?;
    println!("{}", result2);  // Hello, Rust!

    Ok(())
}
```

### 6.3 长期存储引用

```rust
struct GameState {
    /// Lua 侧的玩家数据表
    player_data: LuaTableRef,
    /// Lua 侧的更新回调
    on_update: LuaFunctionRef,
}

impl GameState {
    fn new(vm: &mut LuaVM) -> luars::LuaResult<Self> {
        vm.execute(r#"
            player = { hp = 100, x = 0, y = 0 }
            function on_update(dt)
                player.x = player.x + dt
            end
        "#)?;

        Ok(GameState {
            player_data: vm.get_global_table("player")?.unwrap(),
            on_update: vm.get_global_function("on_update")?.unwrap(),
        })
    }

    fn tick(&self, dt: f64) -> luars::LuaResult<()> {
        // 直接调用存储的函数引用
        self.on_update.call(vec![LuaValue::number(dt)])?;

        // 读取更新后的数据
        let x: f64 = self.player_data.get_as("x")?;
        println!("Player at x={}", x);
        Ok(())
    }
}
```

### 6.4 传入第三方 Struct

```rust
use std::collections::HashMap;

// 假设这是来自第三方库的类型
struct ExternalConfig {
    values: HashMap<String, String>,
}

fn main() -> luars::LuaResult<()> {
    let mut vm = LuaVM::new(SafeOption::default());

    let ext_config = ExternalConfig {
        values: HashMap::from([
            ("key1".into(), "value1".into()),
            ("key2".into(), "value2".into()),
        ]),
    };

    // 方式1：作为不透明 userdata 推入
    let ud = vm.push_any(ext_config)?;
    vm.set_global("ext_config", ud)?;

    // 在 Rust 回调中取回
    vm.register_function("get_ext_value", |state| {
        let ud = state.get_arg(1).unwrap();
        let key = state.get_arg(2).and_then(|v| v.as_str().map(|s| s.to_owned()));
        // downcast 取回原始类型
        if let (Some(config), Some(key)) = (
            ud.as_userdata().and_then(|u| u.downcast_ref::<ExternalConfig>()),
            key,
        ) {
            if let Some(val) = config.values.get(&key) {
                let s = state.vm_mut().create_string(val)?;
                state.push_value(s)?;
                return Ok(1);
            }
        }
        state.push_value(LuaValue::nil())?;
        Ok(1)
    })?;

    vm.execute(r#"
        local v = get_ext_value(ext_config, "key1")
        print(v)  -- "value1"
    "#)?;

    Ok(())
}
```

### 6.5 UserDataBuilder 用法

```rust
use std::net::SocketAddr;

fn main() -> luars::LuaResult<()> {
    let mut vm = LuaVM::new(SafeOption::default());

    let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();

    let ud = UserDataBuilder::new(addr)
        .add_field_getter("ip", |a| UdValue::Str(a.ip().to_string()))
        .add_field_getter("port", |a| UdValue::Integer(a.port() as i64))
        .set_tostring(|a| a.to_string())
        .build(&mut vm)?;

    vm.set_global("server_addr", ud)?;

    vm.execute(r#"
        print(server_addr)           -- 127.0.0.1:8080
        print(server_addr.ip)        -- 127.0.0.1
        print(server_addr.port)      -- 8080
    "#)?;

    Ok(())
}
```

## 7. Implementation Plan

### Phase 1: 核心 Ref 基础设施
- [ ] 实现 `RefInner`（RefId + *mut LuaVM + PhantomData + Drop）
- [ ] 实现 `LuaAnyRef`（通用引用）
- [ ] 在 `LuaVM` 上添加 `to_ref()` 方法
- [ ] 添加单元测试：创建、Drop 自动释放、!Send 编译检查

### Phase 2: 类型化 Ref
- [ ] 实现 `LuaTableRef` 及其全部方法
- [ ] 实现 `LuaFunctionRef` 及其全部方法（含 async）
- [ ] 实现 `LuaStringRef` 及其全部方法
- [ ] 在 `LuaVM` 上添加 `to_table_ref()`、`to_function_ref()`、`to_string_ref()` 工厂方法
- [ ] 添加 `get_global_table()`、`get_global_function()` 快捷方法
- [ ] 添加单元测试：各类型方法、类型转换

### Phase 3: 第三方 Struct 支持
- [ ] 实现 `OpaqueUserData<T>`
- [ ] 在 `LuaVM` 上添加 `push_any<T>()`
- [ ] 实现 `UserDataBuilder<T>` builder 模式
- [ ] 添加单元测试

### Phase 4: 导出与文档
- [ ] 在 `lib.rs` 中 pub use 所有新类型
- [ ] 更新 API Reference 文档
- [ ] 更新 README 示例

### 文件结构

```
crates/luars/src/
  lua_vm/
    lua_ref.rs          ← 扩展：添加 RefInner, LuaAnyRef, LuaTableRef, ...
    mod.rs              ← 扩展：添加 to_ref(), push_any() 等方法
  lua_value/
    userdata_trait.rs   ← 扩展：添加 OpaqueUserData<T>
    userdata_builder.rs ← 新文件：UserDataBuilder<T>
  lib.rs                ← 导出新类型
```

### 设计决策记录

| 决策 | 选择 | 理由 |
|------|------|------|
| 引用内部存储 | RefId + `*mut LuaVM` | 与现有 RefManager 无缝集成 |
| 线程安全 | `!Send + !Sync` | Lua VM 不是线程安全的，强制单线程 |
| 自动释放 | `Drop` impl | RAII 风格，避免手动 release_ref 的遗忘 |
| Clone | 不实现 | 每个 Ref 占一个 registry slot，防止意外复制 |
| 与 LuaValue 关系 | 并存 | LuaValue 仍是内部核心，Ref 是用户层包装 |
| 生命周期标注 | 无（裸指针） | 避免 `'lua` 生命周期在用户 API 中传播 |
| 第三方 struct | OpaqueUserData + Builder | 简单场景用 opaque，复杂场景用 builder |

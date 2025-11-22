# Dispatcher Architecture

## 概述

Dispatcher模块负责Lua虚拟机指令的分发和执行。它被设计为独立于主VM循环,以便在多种执行上下文中复用。

## 目录结构

```
src/lua_vm/dispatcher/
├── mod.rs                         # 主分发器和DispatchAction枚举
├── load_instructions.rs           # 加载指令(LOADNIL, LOADI, LOADK, MOVE等)
├── control_instructions.rs        # 控制流指令(RETURN, CALL, JMP等)
└── arithmetic_instructions.rs     # 算术指令(ADD, SUB, MUL等) [待实现]
```

## 核心类型

### `DispatchAction`

指令执行后的动作枚举:

```rust
pub enum DispatchAction {
    Continue,  // 继续执行下一条指令
    Return,    // 从当前函数返回
    Yield,     // 协程yield(将来支持)
    Call,      // 调用另一个函数(将来支持)
}
```

### `dispatch_instruction()`

核心分发函数:

```rust
pub fn dispatch_instruction(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction>
```

## 使用场景

### 1. VM主循环 (`LuaVM::run()`)

```rust
fn run(&mut self) -> LuaResult<LuaValue> {
    loop {
        let instr = /* 获取当前指令 */;
        let action = dispatch_instruction(self, instr)?;
        
        match action {
            DispatchAction::Continue => { /* 继续 */ }
            DispatchAction::Return => {
                if self.frames.is_empty() {
                    return Ok(/* 最终返回值 */);
                }
            }
            // ...
        }
    }
}
```

### 2. CALL指令执行

当实现CALL指令时,也会使用dispatcher来执行被调用函数的指令:

```rust
fn exec_call(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    // 1. 设置新的调用帧
    vm.frames.push(new_frame);
    
    // 2. 执行被调用函数(复用dispatcher)
    loop {
        let instr = /* ... */;
        let action = dispatch_instruction(vm, instr)?;
        
        if action == DispatchAction::Return {
            break;
        }
    }
    
    Ok(DispatchAction::Continue)
}
```

### 3. 协程Resume

协程恢复执行时,也使用相同的dispatcher:

```rust
fn resume_coroutine(thread: &mut LuaThread) -> LuaResult<Vec<LuaValue>> {
    // 切换到协程的栈帧
    let vm = /* 准备VM状态 */;
    
    loop {
        let instr = /* ... */;
        let action = dispatch_instruction(&mut vm, instr)?;
        
        match action {
            DispatchAction::Yield => {
                return Ok(thread.yield_values.clone());
            }
            // ...
        }
    }
}
```

## 指令实现规范

每个指令处理函数遵循相同的签名:

```rust
pub fn exec_<instruction_name>(
    vm: &mut LuaVM, 
    instr: u32
) -> LuaResult<DispatchAction>
```

**实现要点:**

1. **参数提取**: 使用`Instruction::get_a/b/c/bx/sbx`提取指令参数
2. **栈帧访问**: 通过`vm.current_frame()`获取当前栈帧
3. **寄存器操作**: 使用`vm.register_stack[base_ptr + reg]`访问寄存器
4. **返回Action**: 大部分指令返回`DispatchAction::Continue`

**示例:**

```rust
pub fn exec_loadi(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    let a = Instruction::get_a(instr) as usize;
    let sbx = Instruction::get_sbx(instr);
    let frame = vm.current_frame();
    let base_ptr = frame.base_ptr;

    vm.register_stack[base_ptr + a] = LuaValue::integer(sbx as i64);

    Ok(DispatchAction::Continue)
}
```

## 待实现指令

### 高优先级

- [ ] **CALL/TAILCALL**: 函数调用(需要新帧管理)
- [ ] **JMP/TEST/TESTSET**: 条件跳转
- [ ] **ADD/SUB/MUL/DIV/MOD**: 算术运算
- [ ] **GETTABLE/SETTABLE**: 表索引
- [ ] **NEWTABLE**: 创建表

### 中优先级

- [ ] **POW/IDIV**: 幂运算和整除
- [ ] **BAND/BOR/BXOR**: 位运算
- [ ] **SHL/SHR**: 位移运算
- [ ] **UNM/BNOT/NOT**: 一元运算
- [ ] **CONCAT**: 字符串连接
- [ ] **EQ/LT/LE**: 比较运算

### 低优先级

- [ ] **FORPREP/FORLOOP**: 数值for循环
- [ ] **TFORPREP/TFORLOOP**: 通用for循环
- [ ] **SETLIST**: 表列表初始化
- [ ] **CLOSURE**: 闭包创建
- [ ] **VARARG**: 变参处理
- [ ] **EXTRAARG**: 扩展参数

## 性能优化

### 内联建议

关键热路径函数应标记为`#[inline(always)]`:

```rust
#[inline(always)]
pub fn exec_move(vm: &mut LuaVM, instr: u32) -> LuaResult<DispatchAction> {
    // ...
}
```

### 分支预测

使用`likely`宏提示常见路径:

```rust
if likely!(b != 0) {
    // 常见情况
} else {
    // 罕见情况
}
```

### 避免借用检查开销

缓存常用指针:

```rust
let frame = vm.current_frame();
let base_ptr = frame.base_ptr;
let stack = &mut vm.register_stack;

// 重复使用base_ptr和stack,避免多次借用
stack[base_ptr + a] = value;
stack[base_ptr + b] = another_value;
```

## 测试

每个新指令都应该添加单元测试:

```rust
#[test]
fn test_loadi() {
    let mut vm = LuaVM::new();
    let mut chunk = Chunk::new();
    chunk.max_stack_size = 10;
    
    // LOADI 0 42
    let instr = Instruction::encode_asbx(OpCode::LoadI, 0, 42);
    chunk.code.push(instr);
    
    // RETURN 0 2 0
    let instr = Instruction::encode_abc(OpCode::Return, 0, 2, 0);
    chunk.code.push(instr);
    
    let func = vm.create_function(Rc::new(chunk), vec![]);
    let result = vm.call_function(func, vec![]).unwrap();
    
    assert_eq!(result.as_integer(), Some(42));
}
```

## 未来方向

### Async支持

Dispatcher已经为异步执行做好准备:

```rust
// 将来可能的async版本
pub async fn dispatch_instruction_async(
    vm: &mut LuaVM, 
    instr: u32
) -> LuaResult<DispatchAction> {
    match Instruction::get_opcode(instr) {
        OpCode::Call => exec_call_async(vm, instr).await,
        // ...
    }
}
```

### JIT编译

Dispatcher的清晰结构便于添加JIT支持:

```rust
// 可能的JIT接口
pub trait JitCompiler {
    fn compile(&self, chunk: &Chunk) -> Option<CompiledFunction>;
}

// 在run()中:
if let Some(compiled) = jit.compile(&chunk) {
    return compiled.execute(vm);
}
// 否则回退到解释执行
```

### 调试支持

Dispatcher可以插入调试钩子:

```rust
pub fn dispatch_instruction_debug(
    vm: &mut LuaVM, 
    instr: u32,
    debugger: &mut Debugger
) -> LuaResult<DispatchAction> {
    debugger.on_instruction(vm, instr)?;
    let action = dispatch_instruction(vm, instr)?;
    debugger.after_instruction(vm, &action)?;
    Ok(action)
}
```

## 参考文档

- [COROUTINE_ASYNC_DESIGN.md](../COROUTINE_ASYNC_DESIGN.md) - 协程与async集成设计
- [COROUTINE_DESIGN.md](../COROUTINE_DESIGN.md) - 协程实现设计
- Lua 5.4 VM实现: `lvm.c`中的`luaV_execute()`函数

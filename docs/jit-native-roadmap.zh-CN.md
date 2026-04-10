# JIT 原生后端推进计划

## 2026-04-09 阶段固化

为了避免路线图继续停留在“最近一次 benchmark 之后的口头判断”，先把当前状态固定下来。

### 现在处在哪个阶段

当前最准确的阶段判断不是“还在搭框架”，也不是“已经接近 LuaJIT”，而是：

1. 真实的 trace lifecycle、lowering、snapshot/deopt、side-exit runtime bridge 已经落地。
2. 原生 backend 已经不是占位实现，多个稳定 trace family 可以进入真实机器码入口。
3. 最小 SSA / value-kind / memory-effect artifact 已经出现，并且开始被 lowering 与 native backend 消费。
4. 项目正在从“native trace 能跑”过渡到“native trace 逐步摆脱 `LuaValue` 槽位流”。

也就是说，当前阶段更接近：

- 已经完成 JIT 成立所需的基本架构；
- 正在进入 tracing compiler 真正变薄、变成值流的阶段；
- 但还没有形成 LuaJIT 那种成熟的中端优化与 native child trace linking。

### 哪些工作已经完成

截至当前代码状态，可以把已完成项压缩成下面几类：

1. Trace lifecycle 已经拆成 `Recorded` / `Lowered` / `Executable`，观测口径也同步了。
2. `LoweredTrace` 已经携带 snapshots、exit metadata、per-exit deopt recovery、最小 restore 收窄、value hints、最小 SSA artifact、SSA memory effects、以及第一批 SSA 驱动的 table-int rewrite。
3. runtime 已经具备 hot exit、side trace、ready child dispatch、redundant recovery shrink 这些侧链能力。
4. native backend 已经能执行 `LinearIntForLoop`、`LinearIntJmpLoop`、`NumericForLoop`、`GuardedNumericForLoop`、`NumericJmpLoop` 与 terminal return traces。
5. native lowering 已经有第一批真正 LuaJIT 方向的结构工作：
   - pure-integer peeling 让一部分 boxed numeric loop 迁移到 `LinearIntForLoop`
   - `LinearIntForLoop` 与部分 `NumericForLoop` 已开始使用 carried block params
   - materialization 已开始从 loop-header 同步收窄到 fallback / exit 边界

### 还没完成的核心差距

真正还缺的不是“再多几个 native family”，而是下面这些：

1. native numeric trace 仍然总体是 slot/tag 驱动，不是真正的 registerized value flow。
2. 最小 restore 已经开始成形，但还没到 LuaJIT 那种广泛的 no-restore / sink / 边界物化层级。
3. side trace 仍然主要靠 runtime bridge，而不是 machine-code patch/link。
4. 当前优化仍以 family widening 和局部 lowering 特化为主，但 compile_shared 里已经出现了第一条可复用的 numeric value-flow normalize seam，并开始承载一小串真正的 forwarding / dead-step-elimination rewrite，中端开始有了实际落点。

### 当前主线与辅线

从现在开始，主线要固定为：

1. 让已经进入 native 的热点 trace 更少依赖 `LuaValue` 槽位读写。
2. 继续把 carried-state / block-param / boundary-only materialization 推进到 numeric 路径。
3. 让 SSA/value-flow 和最小 restore 成为 lowering 与 native emission 的真实输入。

辅线才是：

1. 再补一个纯整数算子到 `LinearIntForLoop`
2. 再扩一个新的 specialized family
3. 单纯为了 benchmark 命中统计去保留 helper-heavy native family

### benchmark 在当前阶段的角色

benchmark 现在应该被当作探针，而不是主线本身：

1. 它用来判断某个 trace family 的 steady-state 是否足够干净。
2. 它用来筛掉“native 命中更多但 wall-clock 更差”的伪收益。
3. 它不应该继续主导架构，把项目带回 benchmark-specific widening。

所以当前最准确的表述不是“继续死磕 benchmark”，而是：

- 用 benchmark 约束主线是否真的在收窄 steady-state 成本；
- 同时把真正要保留的实现，限定在 LuaJIT 方向的结构工作上。

## 当前基线

当前 JIT 已经具备以下基础：

- 原生 trace 入口 ABI 已稳定，Windows 调用约定问题已经修复。
- `LinearIntForLoop`、`LinearIntJmpLoop`、`NumericForLoop`、`GuardedNumericForLoop`、`NumericJmpLoop` 已可走真实 native runtime。
- `LoadF`、`GetUpval`、`SetUpval`、`GetTableInt`、`SetTableInt` 已接入 native lowering。
- 整数 `Mod` 与整数 `IDiv` 已有 direct native fast path，不再总是经由 generic arithmetic helper。
- 现有 direct-entry 回归测试已经覆盖主要循环家族，能够直接验证机器码入口，而不是只验证“能编译”；当前基线已经推进到 `cargo test -p luars jit:: --features jit` 194 条全绿。
- 最近一轮主线推进继续沿着同一条主线前进了十二步：
   1. guard block 的 read-only `pre_steps` / preset 现在也可以消费 carried/hoisted numeric 值，不再只覆盖纯条件读取；
   2. carried-float 选择继续开始消费 value-flow，安全的两步 alias 形状 `Move tmp, stable_rhs; Binary dst = dst op tmp` 现在也能进入该路径。
   3. `compile_shared.rs` 的 `optimize_numeric_steps` 现在开始承担最小 numeric alias normalize 职责：它会传播 move-alias、重写后续 numeric/table/upvalue 读取，并裁掉纯别名 dead move；这就是后续 `fwd` / `dse` / `narrow` 类中端优化的第一条真实入口。
   4. 在这条 seam 上，第一条真正的 numeric forwarding / dead-temp-elimination rewrite 已经落地：相邻的 `Binary tmp; Move dst, tmp` 现在会在 lower 阶段直接折叠成 `Binary dst`，先覆盖 `AddI` 这类 affine 形状，再自然覆盖通用 pure numeric binary 临时值链。
   5. 这条 seam 又向前压了一步：forwarding 不再只限于“紧邻一条 `Move`”，现在可以安全跨过少量无关的纯寄存器步骤；同时 `optimize_numeric_steps` 开始裁掉死 `LoadI`，形成第一条真正保守的 numeric dead-step elimination 子集。`LoadF` 等仍保持保守，不做过度删除。
   6. forwarding 继续从“单个 `Move` 消费者”推进到 move-only 单消费者链末端：`Binary tmp -> Move a,tmp -> ... -> Move b,a` 现在可以直接折叠到最终别名目标。与此同时，pure Binary 的 DSE 也有了第一条真正安全的规则，但边界比最初设想更窄：只删除被后续写覆盖、且中间无读的 pure numeric Binary；像 `IDiv` / `Mod` / `Pow` 这类可能牵涉 helper/fallback 语义的 Binary 仍保持保守保留。
   7. 这条 forwarding seam 继续向“真实 use-chain”靠拢了一小步：当 `Binary` 经 move-only 单消费者链最终只作为一次 side-effect source use 出现时，lower 现在也能直接把结果前推到该最终别名目标，首批覆盖 `SetUpval` 与 `SetTableInt.value`。同时，pure-step DSE 也不再只停在 `LoadI`；对 overwritten 的 `LoadF` 这类纯物化步骤，现在也能安全裁掉，但仍明确不把 `GetUpval` 这类读取语义纳入删除范围。
   8. 继续按同一顺序推进后，这条 seam 已经开始覆盖第一类“终端单次 pure consumer”场景：当 move-only 单消费者链的终点不是 side-effect source，而是一次纯 `Binary` 消费时，前面的 `Binary` 现在也能直接前推到该最终别名目标。与此同时，overwritten 纯物化 DSE 又扩了一格，`LoadBool` 也进入了相同的安全删除集合。为了把这些规则从零散点状逻辑收成更清晰的中端入口，`compile_shared.rs` 现在还显式引入了一个 `run_numeric_midend_passes(...)` pipeline，明确当前顺序就是 `forward -> normalize/prune`。
   9. 这条 compile_shared numeric mid-end 现在不再是单轮点状改写。`run_numeric_midend_passes(...)` 已改成显式的多轮稳定化流程：每轮先做 forwarding，再做 normalize/prune，最多迭代 4 轮并在稳定后提前停止。新增回归覆盖“两段本地 pure-consumer 链稳定化”场景，当前稳定结果是第二段 `Binary` 保持自己的目标寄存器，由后续 alias normalization 消掉尾部 move/use，而不是强行把第二段 producer 直接重定向到最终别名寄存器。
   10. 继续按这个顺序推进后，numeric lowering 已不再只暴露 step 向量，而是开始产出带最小 value-state 的 `NumericLowering { steps, value_state }`。`NumericForLoop` 先接上了 integer self-update carried 值并把同步收窄到 fallback / loop-exit 边界；随后 `NumericJmpLoop` 也接上了同一条 lowering seam，在 guard 不触碰目标寄存器时可以直接消费 integer self-update value-state，并同样只在 fallback / side-exit 边界物化回槽位。这里的重点不是“又多了一个 family”，而是 compile_shared 产出的值流事实终于开始被两个真实 native 热点 family 复用。
   11. 再继续按同一顺序推进后，`GuardedNumericForLoop` 也改成消费同一个 `NumericLowering`，并且 integer carried 值不再只参与 loop body，还会直接喂给 guarded tail check；这样 guarded numeric family 也开始共享“值流在 trace 内携带、只在 fallback / loop-exit / side-exit 边界物化”的同一套结构。与此同时，仓内已经没有任何调用点依赖旧的 `LuaTable::next_into` / `NativeTable::next_into` zero-copy 迭代 helper，这两个未使用接口已被删除。
   12. 继续沿着同一条主线推进后，`NumericJmpLoop` 的 integer carried 值也不再只停留在 loop body。guard block 现在同样可以消费这条 carried integer override，所以 tail 条件读取和 guard `pre_steps` 读到的是当前 trace 内值，而不是槽位里的旧物化状态。这样 `NumericForLoop`、`GuardedNumericForLoop`、`NumericJmpLoop` 三个热点 numeric family 已经都接上了同一类“trace 内携带值、边界物化”的整数值流结构，只是覆盖范围还停在最小 self-update 子集。
   13. 再往前推进一步后，guard override 不再只允许单个 hoisted numeric 值。`GuardedNumericForLoop` 和 `NumericJmpLoop` 现在都可以同时携带“self-update 目标寄存器的 carried integer 值”和“stable rhs 的 hoisted integer/float 值”，因此 `continue_preset` / `exit_preset` 与 guard 条件终于共享了同一条最小 value-state seam，而不会在两者只能二选一时退回槽位读取。
   14. 再继续沿着同一条主线收边界后，stable rhs 的 carried/hoisted 路径也不再只检查 guard/preset 会不会改写该寄存器，而是进一步要求 loop body 自身也不能改写这个 rhs。换句话说，现在只有“rhs 在整个 trace steady-state 内确实稳定”的 self-update 形状才会继续走 carried integer/float 快路径；一旦 body 会重写 stable rhs，就立即退回普通 numeric step 执行，而不会错误地把 entry/preheader 里的旧 rhs 值跨迭代复用。
   15. 再往前推进一格后，integer carried 路径也不再要求“整个 loop body 只剩一条 self-update”。`compile_shared` 现在可以在含有不破坏 carried seam 的 residual steps 时仍识别 integer `self_update` value-state，而 `NumericForLoop`、`GuardedNumericForLoop`、`NumericJmpLoop` 的 native lowering 也会按“残余步骤前缀 -> carried self-update -> 残余步骤后缀”的顺序执行 body，让这些 residual steps 继续读到 trace 内当前 integer 值，同时避免把 carried 目标寄存器每轮都物化回槽位。
   16. 沿着完全相同的 seam，再往前推进一格后，float carried 路径也不再只覆盖“body 恰好等于 self-update”的精确形状。`compile_shared` 现在可以在含有安全 residual steps 时继续报告 float `self_update` value-state，而 `NumericForLoop`、`GuardedNumericForLoop`、`NumericJmpLoop` 的 native lowering 也会按“残余步骤前缀 -> carried float self-update -> 残余步骤后缀”的顺序执行 body；这样 residual steps 同样会消费 trace 内当前 float 值，steady-state 不必每轮把 float carried 目标物化回槽位。
   17. 主线已经开始从纯 numeric self-update 向 table/global 热点扩展，但仍坚持“先打最小 seam”而不是直接扩新 family。当前 first cut 是把 `GetField` / `SetField` 这种 short-string 字段访问正式接进 numeric lowering 与 native helper 链，这样像 `local_table.value = local_table.value + 1` 这样的局部字段 RMW loop 终于可以进入现有 `NumericForLoop`。release `bench_locals.lua` 上，`Local table field` 从先前大约 `44 M ops/sec` 提升到了大约 `101 M ops/sec`，同时 JIT 统计里 `Root native numeric for dispatches` 从 `1` 增到 `2`、`Executable slots` 从 `2` 增到 `3`。但 `global_table.value` / `_ENV` 仍未改善，说明下一步必须把同一条 seam 继续接到 `GetTabUp` / `SetTabUp`，否则 global-path 仍然会卡在 upvalue-table 这一跳上。
   18. 继续按这个顺序推进后，`GetTabUp` / `SetTabUp` 也已经接进同一条 short-string field seam，不再要求 `_ENV["name"]` 先落回解释器槽位再做后续字段/算术更新。新增 lowering 回归和 native 直入回归都已覆盖这条 upvalue-table 路径。release `bench_locals.lua` 上，这一步把先前卡住的 global-path 一起拉进 native：`Global var access` 现在约 `99.19 M ops/sec`，`Global table field` 约 `16.69 M ops/sec`，`_ENV lookup (math.pi)` 约 `22.97 M ops/sec`，`Local table field` 也继续保持在约 `108.77 M ops/sec`。对应 JIT 统计里，`Root native dispatches` 提升到 `19999890`，`Root native numeric for dispatches` 提升到 `19999889`，`Native profile upvalue helpers` / `table helpers` 分别达到 `29999833` / `59999662`，说明 `_ENV` 与 global short-string hop 已经真正进入当前 native numeric steady state。`bench_metatables.lua` 本轮仍大致维持原有形状，说明下一步应继续盯住“helper-heavy table/metamethod steady state 变薄”，而不是回到 numeric family 扩张。
   19. 顺着这条“steady-state 变薄”主线再往前推了一小步后，known-miss 的 `__index` table chain 也开始消费整数/short-string 专用查找，而不是一律退回通用 `finishget` 链路。这里没有新增 trace family，也没有改 metamethod 语义，只是在执行器 helper 层给 `GetTable` / `GetI` / `GetField` miss 后、且 `__index` 继续落到 table 的场景补了一条更窄的 fast path。release `bench_metatables.lua` 上，`__index (table)` 从先前大约 `12.94 M ops/sec` 小幅提升到大约 `14.19 M ops/sec`；`__index (function)`、`__newindex`、`__len` 仍大致维持原有量级，且 `Native profile table helpers` 依然约 `1099839`，说明这一步只削掉了 table-only miss 链的固定成本，还没有触到真正更重的 function metamethod / 通用 helper 调度层。下一步仍应继续压薄这些 helper-heavy metamethod steady-state，而不是回头扩 numeric family。
   20. 再按“先 function-valued metamethod、再 native table helper”的顺序推进后，两个更窄的固定成本点也被收掉了。第一，一元 metamethod 不再错误地走二参桥：`__len` / `__tostring` / `__unm` / `__bnot` 现在都消费真正的一参调用路径，`__len` 还新增了“只收到一个参数”的回归，算是同时修了语义和固定成本。第二，native short-string field helper 不再每次都重复做 key 短串判定并在写回 `dst` 后再把 tag 读回来判断 numeric 类型；`NativeTable` 新增了带 tag 返回值的 zero-copy short-string 读取，`GetField` / `GetTabUp` 对应 helper 直接消费这条更薄的接口。release `bench_metatables.lua` 上，`__index (table)` 进一步升到大约 `14.87 M ops/sec`，`__index (function)` 约 `11.17 M ops/sec`，`__call metamethod` 约 `16.70 M ops/sec`；而 warm 复跑 `bench_locals.lua` 时，`Global table field` / `Local table field` / `_ENV lookup (math.pi)` 仍分别保持在约 `17.15 M` / `106.16 M` / `22.50 M ops/sec` 量级，没有出现稳定回退。与此同时，`Native profile table helpers` 在 metatables 样本上仍然大约是 `1099839`，说明这一步已经把 field helper 本身压薄了一点，但更重的 function metamethod steady-state 仍然是下一刀的主目标。
   21. 再往 LuaJIT 方向收一层结构后，metamethod bridge 不再只接受 light-C function，而是统一接受所有 `is_c_callable()` 变体；同时 table 上的 `__len` / `__call` 查找也开始优先走 `meta_ptr` 直达 fasttm 路径，而不是先落回更泛化的 metatable-event helper。新增回归直接覆盖了 Rust closure 作为 `__len` 与 `__call` metamethod 的场景，连同原有 `__len` / `__tostring` / `__unm` / `__bnot` / `__call` 回归一并通过。Windows release 复跑 `bench_metatables.lua` 时，`__call metamethod` 进一步到约 `17.51 M ops/sec`，`__len metamethod` 约 `21.61 M ops/sec`；`__index (function)` 仍约 `10.60 M ops/sec`、`__index (table)` 回到约 `14.16 M ops/sec` 这一噪声带，且 `Native profile table helpers` 仍约 `1099839`。这说明当前这一步更像是把 callable metamethod bridge 和 table fasttm lookup 的主线结构摆正，并顺手削掉了 `__call` / `__len` 的一部分固定成本；真正还没被拿掉的，依然是 function-valued `__index` steady-state 的通用 helper 调度层。
   当前基线已经更新到 `cargo test -p luars jit::backend:: --features jit` 105 条全绿，`cargo test -p luars jit:: --features jit` 208 条全绿；`cargo test --all-features` 的上一轮全仓基线仍为 765/765，本轮未重跑全仓。

这意味着当前阶段已经不再是“先把框架搭起来”，而是要把已有 native trace 能力从“可运行”推进到“执行形状足够薄”，让热点路径真正接近 LuaJIT 式的值流和物化边界。

## 当前离 LuaJIT 还差什么

如果把目标定成“尽快补齐像 LuaJIT 那样的基础能力”，目前最关键的差距不是 trace 能不能进入 native，而是 native trace 进去之后还有多少通用语义成本没有被拿掉。

- control-flow 样本已经几乎总是走 native root trace，但这并不自动等价于“已经接近 LuaJIT”。
- 当前 benchmark 数据说明，side-trace chaining 已不是首要瓶颈；真正的差距在 trace 内部仍保留了大量 helper-heavy / 通用语义路径。
- `Mod`、`IDiv` 这种整数算子本来属于 LuaJIT 会优先直降的基础数值语义；这类路径如果还走 helper，就会让 trace 看起来“native 了”，但热路径成本仍接近解释器辅助执行。
- 算术 widening 已经把 `Add`、`Sub`、`Mul`、`Div`、`IDiv`、`Mod`、`Pow` 的主要 direct path 补上了一轮，但这还不等于 trace 内部已经足够“LuaJIT 化”。
- 最新 control-flow 统计表明，当前主导样本几乎全部时间都花在 `LinearIntForLoop` native root trace 本身；因此更关键的差距开始转向 trace 内固定成本，例如 loop-state 的重复 tag guard、step 值重复 load，以及每轮 `LuaValue` tag/value 往返。
- native side-trace 直连、code cache 生命周期、linking 设计仍然重要，但在当前阶段，它们不是“最快补齐基础内容”的第一优先级。

简化成一句话：当前最缺的不是更多 dispatch plumbing，而是让主导 trace family 的循环体更接近 registerized native trace，而不是继续保留固定的 `LuaValue` 内存往返成本。

## 与 LuaJIT 的结构差距清单

下面这份清单按优先级拆成“必须先补”和“可以后补”。判断标准不是实现是否优雅，而是它们是否直接决定我们能不能在纯算数和稳定热循环里接近 LuaJIT 的执行形状。

### 必须先补

1. 通用 trace IR 仍然不够像 SSA 值流。

- LuaJIT 的核心不是“识别某个循环 family 然后调用手写 executor”，而是 recorder 直接把 bytecode 录成 SSA IR，再交给优化和汇编层。
- 我们现在虽然已经有 lowering、deopt、native entry 和少量 registerized 路径，但热点值大多仍然围绕 `LuaValue` 槽位、tag 和 helper 语义流动。
- 这意味着当前很多 native trace 只是“机器码形式的 VM 操作”，还不是“寄存器中的值流计算”。

2. 热路径的值表示还没有从 `LuaValue` 槽位模型里脱出来。

- LuaJIT 在热点 trace 里会尽量把整数、浮点、指针等值留在寄存器/IR ref 中，只在 exit/snapshot/guard fail 时恢复解释器可见状态。
- 我们当前最大的稳态成本仍是 `LuaValue` 的 tag/value load-store、整数/浮点守护，以及为了保持 VM 状态可恢复而做的频繁物化。
- 这正是算数 benchmark 被 LuaJIT 拉开数量级差距的根因。

3. snapshot/deopt 还没有发展到“最小恢复”级别。

- LuaJIT 的 snapshot 系统不仅完整，而且会标记 `SNAP_NORESTORE` 这一类“不需要恢复”的值，exit 成本是按最小必要集付费的。
- 我们已经迈出了正确一步：`DeoptRecovery` 可以识别冗余恢复，runtime 也能统计 redundant recovery / fast dispatch。
- 但这还只是 bridge shrink，不是成熟的最小恢复体系。后续需要让“哪些值根本不该恢复、哪些值只在 exit 边界物化”变成 lowering 规则，而不是运行时补丁。

4. 中端优化几乎还没真正形成体系。

- LuaJIT 默认就有 `narrow`、`loop`、`fwd`、`dse`、`abc`、`sink`、`fuse` 等整套中端优化，这决定了它的热点算数循环会迅速变得非常薄。
- 我们现在做到的更多是 family widening、少量 invariant hoist、少量 known-integer reuse，以及少数 direct numeric fast path。
- 这些工作不是没价值，但它们更像在给真正的中端优化铺地基，而不是已经取代了中端优化。

5. native side trace 还没有进入 machine-code 直连阶段。

- LuaJIT 的 side trace 是以 exit patching / trace linking 为常态模型，hot side exit 会被直接补到 child trace 上。
- 我们当前的 ready child fast dispatch 和按 `exit_index` 解析 side exit 是正确前置条件，但仍属于 runtime 级桥接优化。
- 在热循环里，这和真正的 native-to-native linking 之间还有一层明显差距。

### 可以后补

1. 继续扩更多 specialized executor family。

- 这件事不是完全没用，但它已经不该是主线。
- quicksort 上 `NumericTableScanJmpLoop` 的 native 化已经证明：helper-heavy family lowering 很容易制造“native 命中上升、wall-clock 变差”的假收益。

2. 更广的 helper 覆盖与边缘数值语义补齐。

- 它们对正确性和覆盖率重要，但对当前和 LuaJIT 的主差距不是第一决定项。
- 在纯算数 benchmark 上，决定胜负的不是“又多支持了一个算子”，而是热点算数值是否已经摆脱 `LuaValue` 槽位流。

3. code cache 生命周期、统一模块管理、更多工程级 backend 清理。

- 这些都值得做，但不会直接把当前 benchmark 拉近到 LuaJIT 的层级。

把这份差距清单压成一句工程判断：

- 当前路线里“registerized trace、最小物化、最小恢复、native child linking”是对的。
- 当前路线里“继续按 family 扩张 + helper-backed lowering 深挖”已经被证明不是最高收益主线。

## 推进顺序

前面的推进顺序主要是围绕“扩大 native 覆盖面”组织的。这个方向在框架搭建期是合理的，但在当前阶段需要重排优先级，否则会继续把时间花在能证明可编译、却不能稳定提高 wall-clock 的工作上。

新的顺序应该改成下面这样。

### 0. 主线调整

从现在开始，主线目标不再是“让更多 trace family 进入 native”，而是“让已经进入 native 的热点 trace 更少依赖 `LuaValue` 槽位与 helper 语义”。

- 优先优化 trace 内部值流。
- 优先减少 steady-state 物化。
- 优先减少 exit 时真正需要恢复的状态。
- family 扩张降级为 benchmark 证明有效时才继续。

### 1. 建立真正的 SSA / registerized trace 中线

这一步现在应该成为第一优先级。

目标不是继续堆局部 cache，而是把热点 trace 的执行模型从“native 版 VM 槽位操作”推进到“trace 内值流，边界处物化”。

优先内容：

- 给 lowering 一个更明确的值表示层：整数、浮点、布尔、表引用、upvalue 引用等，不默认落回 `LuaValue`。
- 把 loop-carried state、热点临时值、guard 输入值都提升成显式 SSA/block 参数。
- 让 trace body 内部的读取优先消费这些值，而不是重新从栈槽加载。

没有这一步，后面的数值优化、guard 优化、甚至 side trace linking 都只能建立在一个过重的 steady-state 上。

### 2. 把 snapshot / deopt 推到“最小恢复”

当前已有 `DeoptRecovery`、`recover_exit_by_index`、冗余恢复检测，这是正确基础；接下来该做的是把它从“runtime 判重”升级成“lowering 规则”。

优先内容：

- 在 lowering 阶段区分必须恢复和不必恢复的值。
- 让 `register_restores` / `range_restores` / `upvalue_restores` 尽量只覆盖解释器真正可观察的状态。
- 为热点 trace 引入更明确的“仅 exit 物化”规则，而不是在稳态每轮保持强同步。

只有 exit 成本被压到最小，native trace 的收益才不会被 side exit / loop exit 吃掉。

### 3. 在此基础上继续压 steady-state 固定成本

这一步才是当前 benchmark 最可能立即受益的部分。

优先内容：

- 热点数值值的 load/store forwarding。
- 死写消除与更少的 tag store。
- 更系统的 invariant hoist，而不是仅在单一 family 上继续追加 ad hoc 规则。
- 针对稳定整数/浮点值的 guard 合并与消重。

这一层本质上是在补 LuaJIT 默认就有的 `loop`、`fwd`、`dse`、`narrow` 一类优化。

### 4. 再做 native side trace linking

这一步仍然重要，但现在应该排在 SSA 化和最小恢复之后。

原因不是它不值钱，而是如果 parent/child trace 内部本身还在频繁 `LuaValue` 往返，那么把 runtime 桥再缩短一层，收益上限也会很有限。

优先内容：

- 让 hot side exit 的 native child trace 可以被直接 patch/link。
- 把现在的 ready-side-dispatch / `exit_index` 解析真正落到 machine-code tail/head patching。
- 把 parent/child 之间可继承的 base/value 状态显式建模，而不是一律靠恢复后重进。

### 5. 最后才是继续扩 family/native 覆盖

这一步不是取消，而是降级。

继续扩 family 的前提应该改成：

- benchmark 上已经证明该 family 的 steady-state 足够干净；
- lowering 不需要引入大量 helper-heavy 语义桥；
- 预计收益来自减少 steady-state 成本，而不是单纯提升 native dispatch 计数。

## 过渡期保留策略

在真正的 SSA/registerized 中线成形前，现有 specialized family 和 native widening 仍然有过渡价值，但角色要更明确：

- 它们用于验证 ABI、deopt、snapshot、exit linking、guard 语义是否正确。
- 它们用于 benchmark 驱动地确认哪个 family 的 steady-state 足够干净，值得将来纳入通用 lowering。
- 它们不再被视为“只要继续扩就会接近 LuaJIT”的主路径。

这能避免我们在过渡实现里投入过多精力，却只得到更多 native 命中统计，而不是更高的真实吞吐。

## 已完成但需要重新定位的工作

下面这些工作并不是错的，但现在需要重新理解它们的地位：

- `LoadF`
- `GetUpval`
- `SetUpval`
- `GetTableInt`
- `SetTableInt`
- 其余仍未 native 化的数值/位运算

- 它们证明 native ABI、helper 接线、runtime bridge、deopt / side-exit plumbing 已经能承载真实 native trace。
- 它们还不能单独证明“已经在逼近 LuaJIT 的热路径形状”。
- 后续只有当这些能力被放进更通用的 SSA/registerized lowering 中，才会转化成更稳定的性能收益。

## 重排后的明确目标

如果现在只保留一个总目标，它应该从原来的“尽快扩大 native family 覆盖面”改成下面这句：

把当前 JIT 从“可 native 执行一部分 trace 的 specialized executor 系统”，推进成“拥有通用 SSA 值流、最小 snapshot 恢复、边界物化和 native trace linking 的 tracing compiler”。

这件事拆成具体工程目标，就是：

1. 先把热点 trace 的值流和 `LuaValue` 槽位流拆开。
2. 再把 snapshot/deopt 做成最小恢复而不是运行时补救。
3. 再把 steady-state 的 forwarding / narrowing / dse / fuse 类优化补起来。
4. 最后让 parent/child native trace 真正 patch/link。

只有这样，纯算数 benchmark 和稳定热循环 benchmark 才有可能从“已经 native 了但还是慢”跨到“真的接近 LuaJIT”。

## 对当前 quicksort / control-flow 结果的解释

把最近几轮 benchmark 放到这条新主线下看，含义就更清楚了：

1. `bench_control_flow.lua` 上大量 root native dispatch 但收益趋缓，说明“进入 native”已经不是主瓶颈，steady-state 形状才是。
2. `NumericTableScanJmpLoop` native 化后 wall-clock 回退，说明 helper-heavy lowering 不是当前主线。
3. `NumericTableShiftJmpLoop` 的成本拆分显示 steady-state 很干净、fallback 为零，说明真正值得做的不是再补 fallback，而是降低 repeated get/set/guard 成本。
4. 纯算数 benchmark 被 LuaJIT 远远拉开，说明我们目前最缺的是值流层级的优化，而不是更多 dispatch plumbing 或更多 family 名字。

这也给后面的决策提供了一个简单标准：

- 如果一个优化主要减少 steady-state 的 `LuaValue` 往返，它更可能是主线。
- 如果一个优化主要增加 native 命中，但仍依赖大量 helper 或恢复桥，它更可能只是过渡工程。

## 下一阶段建议

下一阶段的执行顺序建议固定为：

1. 设计并落地最小的 SSA/value-kind 层。
2. 让 `LinearIntForLoop` 和最简单的 numeric loop 真正消费这套值流，而不是继续扩更多 family。
3. 把 snapshot/deopt 规则化成最小恢复。
4. 只在 benchmark 明确显示 steady-state 足够干净时，再考虑把 `NumericTableShiftJmpLoop` 之类 family 纳入新的 lowering。

这样做的结果不是短期内统计项更漂亮，而是更有机会真正缩小和 LuaJIT 的结构性差距。

## 最小 SSA / value-kind 草案

为了避免路线图继续停留在原则层，下一阶段先按“最小可落地”版本推进，不一次性发明完整 SSA 编译器。

### 目标

先在 lowering 层引入一层最小的值类别信息，让 trace 至少知道“这个寄存器当前更像整数、浮点、布尔、表、闭包，还是未知值”，然后再让后续 registerized lowering 和 deopt 逐步消费这层信息。

### 第一阶段结构

第一阶段只做三件事：

1. 在 `LoweredTrace` 上保存 root trace 的寄存器值类别提示。
2. 值类别先采用保守集合：
   - `Unknown`
   - `Integer`
   - `Float`
   - `Numeric`
   - `Boolean`
   - `Table`
   - `Closure`
3. 只对能稳定判断的写寄存器做分类：
   - `LoadI` -> `Integer`
   - `LoadF` -> `Float`
   - `LoadTrue` / `LoadFalse` / `Not` -> `Boolean`
   - `NewTable` -> `Table`
   - `Closure` -> `Closure`
   - `Move` -> 传播来源寄存器的已知类别
   - 明显数值算子 -> `Numeric` 或 `Integer`
   - 其余保守地维持 `Unknown`

这一层还不直接改变执行语义，它的意义是把“trace 内部值流”从纯概念变成 lowering 可见的事实。

### 第二阶段消费方式

有了这层 value-kind 之后，后续按顺序消费它：

1. registerized lowering 用它决定哪些值值得从 `LuaValue` 槽位中提出来做 block-param / SSA 值流；
2. guard lowering 用它减少重复 tag guard；
3. deopt/snapshot 用它区分“必须恢复的解释器状态”和“只是在 trace 内部流动的中间值”；
4. trace report / benchmark 先把这层信息暴露出来，帮助判断某条 trace 是否已经具备进一步 registerized 的条件。

### 当前已开工的部分

这一版草案已经开始落地：

- lowering 现在已经为 root trace 收集保守的 `value-kind` 寄存器提示；
- `LoweredTrace` 已能查询某个根寄存器的值类别；
- trace report 已开始输出这类提示的摘要，便于后续 bench/profile 时观察；
- 已补 lowering 定向测试，验证 `LoadI` / `LoadF` / `Move` / `LoadTrue` / `Closure` / `NewTable` / `Add` 这些明显形状会产出符合预期的值类别。

这还只是最小脚手架，但它是后续把热点 trace 从 `LuaValue` 槽位流推进到值流 lowering 的第一块真实地基。

## 已有 native 覆盖作为过渡基线

当前已有的 native 覆盖面仍然保留，作为过渡基线：

1. 需要额外运行时上下文的 step 已能进入 native。
   - `GetUpval` / `SetUpval`
   - `GetTableInt` / `SetTableInt`
   - 这说明 upvalue 指针、表访问、写屏障与 helper 接线已经具备基础承载能力

2. 一批核心纯数值路径已经有 direct native fast path。
   - `Mod`
   - `IDiv`
   - `Add`
   - `Sub`
   - `Mul`
   - `Div`
   - `Pow`
   - 部分位移/混合数值路径

3. 终止型 trace 已能走原生出口。
   - `Return`
   - `Return0`
   - `Return1`

4. side-exit/native-child 桥接已经具备一层过渡能力。
   - runtime 可以按 `exit_index` 解析 native side exit
   - parent trace 已可缓存 ready child dispatch
   - redundant recovery / fast dispatch 已可测量

这些能力的意义是：native tracing 的语义底座已经够用了。后面真正缺的不是“还能不能再补一个 family”，而是把这些能力装进更通用、更像 LuaJIT 的 SSA/registerized lowering。

这仍然值得做，但根据当前 benchmark 观察，它已经不是最先该补的“基础短板”。

### 4. 更长期的结构优化

当 native 覆盖与 side-trace 路径稳定之后，再考虑：

- 更统一的 code cache / module 生命周期管理
- 更激进的 lowering / SSA / peephole 优化
- 更系统的 deopt / linking 设计

## 本轮开始推进的内容

本轮的推进重点已经切换到“先收掉 `LinearIntForLoop` 里最确定的固定成本”：

- 已完成整数 `Mod` 的 direct native fast path，并补了负数取模语义回归。
- 已完成整数 `IDiv` 的 direct native fast path，并补了负数整除语义回归。
- 已完成整数 `Add` / `Sub` / `Mul` 的 direct native fast path，并补了 overflow 走 helper fallback 的回归。
- root native dispatch family 统计已经确认 control-flow 样本几乎全部落在 `LinearIntForLoop`，因此真正该继续收窄的是这个 trace family 自身的循环体固定成本。
- 已新增 `LinearIntForLoop` 的条件性 invariant hoist：当 step body 不写 `loop/step/index` 三个 for-state 槽位时，native lowering 会把这三个槽位的一次性整数 guard 和 `step` 的 value load 提前到 preheader，backedge 不再每轮重复做这些 guard/load，也不再重复写回 `loop`/`index` 的整数 tag；如果 trace 会改写这些槽位，则仍保留原有保守路径。
- 在此基础上，`LinearIntForLoop` 的 step body 现在还会追踪“本轮已知为整数”的寄存器：由 `LoadI` / `Move` / `Add` / `AddI` / `Sub` / `Mul` 刚写出的整数寄存器，后续 step 不再重复做整数 tag guard；对已经被证明持续为整数的目标寄存器，也不再重复写回整数 tag。
- 这一步带来了新的直入回归覆盖“同轮 fresh integer write 后再读取”的形状；当前验证基线已提升到 `jit::backend::test::native` 29 条全绿、`cargo test -p luars jit:: --features jit` 134 条全绿。
- benchmark 结果上，这一步没有再带来清晰的 control-flow 提升，release `bench_control_flow.lua` 约为 `if-else 0.140s`，和上一轮 `0.139s` 基本持平。这说明下一阶段更该继续减少 value load/store 本身，而不是只继续消掉 tag guard / tag store。
- 更激进的 step-body SSA/value-cache 试验也已经做过，但在当前 slot-centric emitter 上没有形成稳定收益，且有时会因寄存器压力带来回退。因此它没有被保留；如果下一步要继续做“减少 value load/store”，就不该再堆本地 cache，而应直接转向更原则化的 block-param/显式物化设计。

因此，下一阶段的最短路径应该是：

1. 继续把 `LinearIntForLoop` 做得更像真正的 registerized trace，优先收掉 loop-state 与热点寄存器的固定内存往返。
2. 在不牺牲 fallback/deopt 语义的前提下，继续减少 trace 内重复的 `LuaValue` tag/value 写回。
3. 再回头看 native side-trace 直连和更广的 guard/deopt shape。

这样推进，才是在“尽快补齐 LuaJIT 基础内容”这个目标下收益最高的顺序。

## 明确的下一个目标

如果目标是“先尽量把整体形状做得接近 LuaJIT，再开始真正依赖简单测试 profile”，那么下一个目标应该收敛成一件事：

把 `LinearIntForLoop` 从“仍然围绕 LuaValue 槽位读写的 native trace”推进到“有显式 loop-carried 整数状态、只在必要点物化回槽位的 registerized native trace”。

更具体地说，这一步不是继续堆局部 cache，而是做下面三件事：

1. 把 `loop/index/step` 以及最热整数临时值变成显式的 loop-carried SSA/block 参数，而不是每轮都从栈槽重新取值。
2. 把 body 内部整数 step 的结果尽量留在 trace 内部寄存器流里，只在 fallback、loop exit、side exit 等真正需要和 VM 状态重新对齐的点物化回槽位。
3. 把这套物化点做成明确规则，而不是继续追加临时性的本地缓存优化。

这是当前最像 LuaJIT 的下一步，因为它解决的是 trace 执行形状本身，而不是继续微调某个 benchmark 上的局部开销。

当前这一步已经开始落地：

- `LinearIntForLoop` 的第一阶段 registerized lowering 已经接入，`loop/index` 现在会作为显式 loop-carried block 参数在 native backedge 上流动，body 对这些寄存器的读取可以直接使用 carried value，而不是每次都从槽位 reload。
- 这一步现在又往前推进了一层：`loop/index` 的 carried state 已经不再在 loop header 每轮物化，而是只在 body fallback 和 loop exit 前同步回槽位。也就是说，steady-state 的 loop body 已经真正摆脱了这两项状态的每轮 header 写回。
- 当前结果是：方向继续正确、测试仍然全绿，`jit::backend::test::native` 29 条、`cargo test -p luars jit:: --features jit` 134 条都通过；release `bench_control_flow.lua` 目前大致仍在 `if-else 0.149s` 左右，没有形成清晰的稳定收益。
- 这说明：把 loop-carried state 改成 block 参数并把物化点推迟到 bail/exit 边界，是必要的 LuaJIT 形状工作，但它本身还不足以兑现性能；后续是否继续做“2”，关键不在于盲目扩更多 carried value，而在于先确认哪些额外状态值得纳入同样的最小物化规则。
- 为了给下一步 side-trace 对齐做准备，runtime 现在还补了一个更小的 deopt 桥接收窄：当 root/side exit 解析出来的 `DeoptRecovery` 本身为空时，会直接跳过恢复循环，再尝试 ready side-trace dispatch。这还不是 native-native 直链，但它把“空恢复 side exit”显式区分出来了，后续如果要继续做更接近 LuaJIT 的 child trace 直达路径，可以直接复用这层语义判定，而不必再把空恢复当作普通 deopt 处理。
- 这一步现在又往前修正了一层：runtime 不再只看 `DeoptRecovery` 的 restore 列表是否为空，而是按当前 stack / upvalue 状态判断这份 recovery 是否冗余。对 native side exit，会单独统计两件事：`Native redundant side-exit recoveries` 与 `Native redundant side-exit fast dispatches`。当前 release `bench_control_flow.lua` 的结果是前者命中 `1`、后者命中 `0`，说明主样本里确实存在“恢复写回纯属重复”的 side exit，但它还没有连接到 ready native child trace；也就是说，统计这一步已经闭环，而更激进的 native-native child 直达目前还不是这个 benchmark 的主导收益点。
- 按这个结论继续往前，已经改用更复杂的 `bench_quicksort.lua` 做过一次真实热点筛选。quicksort profiling 显示 `partition` 的热点主要落在 `NumericTableScanJmpLoop` / `NumericTableShiftJmpLoop`，于是试做了 `NumericTableScanJmpLoop` 的 helper-backed native lowering。语义和测试都通过了，native side-exit fast dispatch 也大规模命中，但 release quicksort 从约 `0.120~0.134s` 回退到了约 `0.152s`。这说明当前 shape 下 helper-heavy 的 native table-scan loop 单位成本过高；该试验已经完整回滚，留下的有效结论不是“继续保留更多 native 命中”，而是“复杂 benchmark 下必须先验证 wall-clock，再决定是否保留某个 native family 扩张”。
- 同一轮 quicksort 现在又把 `NumericTableShiftJmpLoop` 的解释器 executor 成本拆开看了一次。release 基线下，这个 family 的 root/side dispatch 大约是 `47386/47377`，对应 `100458` 次成功迭代、`70770` 次 compare side exit、`23993` 次 bound side exit，而且 type/meta/table-get/table-set fallback 与 GC barrier 都是 `0`。这说明 shift family 的 steady-state 已经比 table-scan 干净得多；如果下一步继续尝试它，重点不该是再修 fallback，而应是直接压低 repeated `fast_geti` / `fast_seti` / guard 的单位成本。
- 再往前一步，generic `NumericJmpLoop` 主线已经稳定吸收了“多连续 head guards / 多连续 tail guards / mixed head+tail guards”这类 empty-prestep 真实 trace 组合；backend/state 回归都保持全绿。这是当前最接近 LuaJIT 方向的增量，因为推进的是通用 guard-block CFG，而不是 benchmark 专属 family。
- 再往前一层，这个 real head-prestep blocker 也已经被拆成两部分并各自收住了：一是 guard-block prestep helper 失败现在不再走 trace-root `Fallback`，而是按 guard exit 走 `SideExit`；二是 numeric-jmp 的 carried-float 快路径不再把 quicksort 这类“head prestep 读取 loop-carried integer 索引”的 shape 误判成 float 自更新循环。对应的 direct block-sequence 回归、real IR lowering 对照回归、compile-test 执行回归都已通过，release `bench_quicksort.lua` 也重新稳定跑通，`Root native numeric jmp dispatches` 已恢复到 `80060`，说明 narrow real head-prestep recognizer 现在可以留在 mainline。
- 同一轮里还额外试过把 recognizer 再放宽一格，去吃 quicksort `shift` helper 里的 `empty head guard -> head-prestep guard -> body` 形状。它在语义和 trace 识别上都能通过，native `NumericJmpLoop` 命中也会显著增加，但 release quicksort 会从约 `0.045~0.046s` 回退到约 `0.057~0.058s`。这个结论很重要：对 generic guard-block CFG 来说，“能识别更多 shape” 仍然不等于 “值得留在主线”；像这种 widening，如果不能同时守住 wall-clock，就应当回滚，只保留经验和回归素材。
- 同步把 call-heavy `SummaryOnly` trace 的执行边界也确认了一次：当前 `HelperPlan` 仍然只是 dispatch summary artifact，`helper_plan.rs` 里的 `execute_*_helper` 只累加统计，不承担真正 VM 语义；而 runtime 对 `CompiledTraceExecution::LoweredOnly` 仍直接返回 `None`。这意味着 quicksort 那类含 `Call` / `MetamethodFallback` 的 lowered trace 现在不是“再补一个 enterable 枚举分支”就能跑起来，而是缺一整层真正的 helper interpreter / helper-backed executor。按照当前阶段的取舍，这条线先不硬做，避免把主线重新带回 helper-heavy widening。
- 换句话说，下一条更像独立 executor family 的目标不是继续挤 numeric-jmp recognizer，而是 generic-for / `TFOR` 热点。最新 `bench_iterators.lua` trace report 里，除了已经 native 的 numeric-for 基线外，剩余主要热点高度集中在 `TFORPREP -> body(arith) -> TFORCALL -> TFORLOOP` 这一类 root：例如 `pc=57/92/127/162/273/310/346` 都是 `arith + call + backedge` 的 3-op lowered trace，只是迭代器来源不同；`pc=193/196` 则是 `next()` 这类带额外 guard/branch 的变体。所以下一步如果继续做“通用 LuaJIT 方向”的 executor 扩张，第一刀应该优先围绕这类单-call generic-for loop body 建模，而不是再为 quicksort 单独放宽更多 numeric-jmp shape。
- 进一步往实现层收窄后，这个 `TFOR` family 的第一刀也已经比较明确：先不尝试“任意 helper-call trace 都可 enter”，而是只做 `TForCall` 这一种语义固定的 loop call。解释器里 `TForCall` 的热路径本质上是“把 iterator/state/control 搬到调用位，然后执行一次 `precall(..., nargs=2, nresults=C)`”；因此最小可行方案不是整套 helper interpreter，而是一个专用 `TForCall` native helper / executor：
   1. 当 iterator 是 C-callable 且 `precall` 走 inline 完成时，native trace 继续执行同轮 `TFORLOOP`；
   2. 当 iterator 是 Lua function、`precall` 会压入新 frame 时，native trace 直接把控制权交回解释器主循环，而不是强行在 native family 内继续跑 callee；
   3. 其余 `__call` / metamethod / 非稳定 shape 继续保守回退。
- 这样做的意义在于：它仍然是通用 `generic-for` 结构工作，因为切的是 VM 明确定义的 `TForCall` 语义，而不是为 `ipairs`/`pairs`/`next` 写 benchmark 专属 executor；但它又避免了 helper-plan 那条“先把所有 call-heavy SummaryOnly trace 都做成 enterable”所需要的大体量 helper interpreter。按当前代码状态，这比继续扩 numeric-jmp recognizer 更像正确的下一步。
- 这条最小 skeleton 现在已经落地了一版：native backend 新增了 `NativeTForLoop` family，并接入了一个专用 `TForCall` helper。当前语义边界是：
   1. body 内仍只接受现有 numeric lowering 能覆盖的 step；
   2. `TForCall` 对 C-callable iterator 会在 native trace 内完成一次真实 `precall`/inline C 调用，然后继续同轮 `TFORLOOP` 判断；
   3. 对 Lua iterator，helper 会在压入新 frame 后返回 `Returned`，把控制权交还解释器主循环；
   4. hooks / 其余不稳定情况仍保守回退。
- 当前已经有最小 backend 回归覆盖这条骨架：一条 `Arithmetic -> TForCall -> TForLoop` trace 能被识别成 `NativeTForLoop`，并且 C iterator 路径可以在 native 中累计 body、最终在 `TFORLOOP` nil-exit 处按 side exit 退出。也就是说，`TFOR` 主线已经从“路线判断”进入“可继续扩 shape”的实现阶段。
- 同时，Lua iterator 这条切换回解释器的边界现在也已经有 focused 回归：同样的 `NativeTForLoop` trace 在 iterator 为 Lua function 时，会在 `precall` 压入新 frame 后返回 `Returned`，当前线程 call stack 会真的多出一个 Lua frame，而不是伪装成 native 内继续执行。这样至少 `TForCall` helper 的两条主分支都已经被测试钉住。
- 反过来看，`next()` 手写迭代 benchmark 剩下的两条 lowered root（`pc=193/196`）当前还不能直接沿着这条 skeleton 吃掉。它们虽然同样含有 `Call + guard/branch + backedge`，但 trace 里还包含一次通用 `GetTabUp next` / 全局读取；而当前 native backend 只有 `GetTableInt/SetTableInt` helper，没有对应的通用 global/table-string helper。所以这一步的真实 blocker 不是 `Call`，而是“先补一层足够保守的通用 `GetTabUp/GetField` 读取 helper”，否则继续往前只会把 `TFOR` 专用 helper 重新拉回 helper-family 大杂烩。

因此，这个目标的下一子阶段已经很明确：

1. 已完成：做 bailout-aware 的最小物化，把 loop-carried 状态只在 fallback、loop exit、side exit 前同步回槽位。
2. 下一步再考虑把更多热点整数临时值纳入同样的 carried/state 规则，但前提是它们能形成真正稳定的 loop-carried 状态，而不是重新变成寄存器压力换局部 cache 的试验。

## 之后再做什么

只有在上面这一步做完，或者至少形成一个稳定的“registerized linear-int trace”基线之后，才应该开始真正的基于简单测试的 profile。

原因很直接：

- 如果现在就大量做简单 benchmark profile，测出来的大部分热点还会被“槽位物化太早、寄存器化不够”这种结构性问题污染。
- 先把 trace 形状做对，再做 profile，得到的才更接近真实的 LuaJIT 差距，而不是当前过渡实现的噪声。

因此后续节奏应当是：

1. 先完成 `LinearIntForLoop` 的 registerized lowering。
2. 再检查 native side-trace 直连和更广 guard/deopt shape 是否仍然是显著结构短板。
3. 只有在这两块基础形状稳定后，再正式进入“基于简单测试做 profile 和收尾优化”的阶段。

## 2026-04-09：从路线图转成真实 SSA artifact

在完成 `entry_stable_register_hints`、table region/key/version、以及 restore compaction 之后，当前主干已经不再是“手写 executor 驱动的 JIT”，而是更明确的 `record -> lower -> native/summary` 形态。

这带来一个判断上的变化：

- 我们离 LuaJIT 的主差距，已经不再是“还有多少 executor 没删掉”。
- 现在真正缺的是：SSA trace IR、统一的 memory effect 语言、以及更激进的最小 snapshot 恢复。

因此下一阶段不再把“继续扩 trace family”当主线，而是开始把 lowering 推进成真实的值流 artifact。

### 最小 SSA 起步版本

这一轮先落一个最小可运行的 SSA/value artifact，而不是直接试图发明完整编译器：

1. `LoweredTrace` 直接携带一份最小 SSA trace。
2. 这份 SSA trace 先保守记录三件事：
   - 首次读到的 live-in 寄存器会生成 entry value
   - 每次写寄存器会生成新的 derived value
   - 每条 IR instruction 会显式记录它读取了哪些 SSA value、产生了哪些 SSA value
3. SSA value 暂时继续复用现有 `TraceValueKind`，让整数、浮点、布尔、表、闭包、未知这些类别直接挂在 value 上。

这一步的意义不是宣称“已经有 SSA 编译器”，而是把后续 native lowering、deopt、memory 优化都需要共享的值对象先稳定下来。

只要 trace report 已经能稳定显示 entry values / derived values / kind 分布，下一子阶段就应该继续让最简单的 numeric 与 linear-int lowering 真正消费这份 artifact，而不是再做新的 ad hoc value cache。

## 2026-04-09：最小恢复开始变成 lowering 规则

在前面完成 restore compaction、redundant recovery 统计之后，这一层现在又往前走了一步：

- `LoweredTrace` 不再给所有 exit 复制同一份 `restore_operands`；
- lowering 现在会基于 `resume_pc` 在原始 chunk 上做一层保守的 live-state 分析；
- side exit 的 restore 集会按“exit 之后解释器仍可能观察到的寄存器/上值”单独收窄；
- side exit 的 restore 候选写集合也不再来自整条 trace，而是来自该 exit 实际执行到的 guard 前缀，避免把 guard 之后根本没执行的写入误计入恢复；
- restore 现在开始消费简单的值流来源：对 `Move` 别名、`LoadI`、`LoadF`、`LoadK`、`LoadTrue/LoadFalse`、`LoadNil` 这些稳定形状，lowering 会记录“目标寄存器应从哪个来源恢复”，而不是一律假设只能从目标槽位取值；
- 这层来源恢复已经继续扩到更多可证明整数形状：当 `AddI`、`AddK`、`SubK` 的输入寄存器或其已知 restore source 可证明为整数时，restore 可以记录折叠后的整数立即数，或继续保留 `source + offset` 这类 affine 来源；对 `BAndK`、`BOrK`、`BXorK` 与 `ShlI`、`ShrI` 这类稳定整数位运算/移位形状，restore 现在也能记录“源寄存器 + 常量池/立即数”的符号来源，并在 exit 时直接重建目标寄存器，而不是退回目标槽位；
- `TestSet` 这类条件写现在也开始按 exit 臂做恢复判定：只有当 side exit 实际走到会执行 `R[A] := R[B]` 的 branch-target 臂时，这个写入才进入 restore；同一套判定框架现在也覆盖了 `ForLoop`，只有 loop-backedge 臂真正执行了计数器/控制变量更新时，这些条件写才会进入 restore。
- restore source 判定现在也开始直接消费最小 SSA artifact：当局部 hint / 当前 trace 前缀来源还不足以证明某个输入寄存器是整数时，lowering 会回退去看同一条 IR 指令在 `ssa_trace` 中对应 input value 的 kind，把 SSA producer 提供的整数事实用于选择整数 restore 形状，而不是立刻退回目标槽位。
- 拿不准的条件写路径继续保守处理，不把 runtime 正确性换成激进剪枝。

这还不是 LuaJIT 级别的 `SNAP_NORESTORE` 体系，但它已经把“最小恢复”从 runtime 判重推进成了真正的 lowering 规则。

这也意味着下一步的重点不该再是给 deopt 再加一层统计，而是继续扩大这种 lowering 侧的可证明最小恢复范围，并让更多 SSA/value 流信息参与 restore 决策。
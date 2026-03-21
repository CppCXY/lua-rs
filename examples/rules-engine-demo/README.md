# rules-engine-demo

一个更接近真实业务嵌入场景的 luars 示例：Rust 宿主负责库存、风险评分和物流 SLA，Lua 负责编排结算规则。

这个例子想证明的不是“Lua 能跑”，而是：

- 业务规则可以放到 Lua 里动态演进
- 核心数据和外部能力依然由 Rust 宿主管理
- Rust 和 Lua 之间可以自然地交换表结构、调用函数、返回决策结果

## 场景

模拟一个电商结算规则引擎：

- Rust 构建订单、客户、商品明细
- Lua 函数 `evaluate_order(order)` 决定是否通过、是否打折、走哪种物流、是否人工审核
- Lua 在决策过程中回调 Rust 提供的能力：
  - `risk_score(email, total_cents, country)`
  - `inventory_available(sku, quantity)`
  - `shipping_eta(country, tier)`
  - `audit(message)`

## 运行

使用内置脚本：

```powershell
cargo run -p rules-engine-demo
```

使用你自己的规则脚本：

```powershell
cargo run -p rules-engine-demo -- --script examples/rules-engine-demo/scripts/checkout_rules.lua
```

## 你可以改什么

直接修改 [scripts/checkout_rules.lua](scripts/checkout_rules.lua)：

- 提高高风险阈值
- 调整 VIP 折扣
- 加入新的国家合规规则
- 为特定品类切换物流策略

修改 Lua 后重新运行，不需要改 Rust 宿主代码。
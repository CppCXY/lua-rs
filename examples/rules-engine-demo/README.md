# rules-engine-demo

This example shows how to build a business rules engine entirely on top of the high-level `Lua` API.

Rust owns the order data and host capabilities. Lua owns the decision logic. The example avoids low-level `LuaVM` usage, raw value arrays, and manual stack handling.

## What this example demonstrates

- Exposing Rust capabilities with `Lua::register_function()`
- Building input data with `Lua::create_table()` and `Table::set()`
- Loading the rules script through `Lua::load(...).exec()`
- Calling `evaluate_order(order)` through `Lua::call_global1()` and decoding a structured result

## Scenario

The sample models a checkout rules engine:

- Rust builds orders, customers, and line items
- Lua decides approval, discounts, shipping tier, and manual review
- Lua can call these Rust host functions during evaluation:
  - `risk_score(email, total_cents, country)`
  - `inventory_available(sku, quantity)`
  - `shipping_eta(country, tier)`
  - `audit(message)`

## Run it

Use the embedded rules script:

```powershell
cargo run -p rules-engine-demo
```

Use your own rules file:

```powershell
cargo run -p rules-engine-demo -- --script examples/rules-engine-demo/scripts/checkout_rules.lua
```

## Main files

- `src/main.rs`: registers host functions, builds tables, and calls Lua
- `scripts/checkout_rules.lua`: the Lua rules script

## What you can change

Edit [scripts/checkout_rules.lua](scripts/checkout_rules.lua) to:

- raise or lower risk thresholds
- tune VIP discounts
- add country-specific compliance rules
- change shipping behavior for product categories

You can change the Lua policy and rerun the demo without touching the Rust host code.
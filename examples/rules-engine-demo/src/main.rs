use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use luars::lua_vm::{LuaVM, SafeOption};
use luars::{LuaValue, Stdlib, TableBuilder};

type AppResult<T> = Result<T, Box<dyn Error>>;

const DEFAULT_SCRIPT: &str = include_str!("../scripts/checkout_rules.lua");

#[derive(Clone)]
struct Customer {
    email: &'static str,
    country: &'static str,
    vip: bool,
    loyalty_points: i64,
}

#[derive(Clone)]
struct LineItem {
    sku: &'static str,
    category: &'static str,
    quantity: i64,
    unit_price_cents: i64,
}

#[derive(Clone)]
struct Order {
    id: &'static str,
    coupon_code: Option<&'static str>,
    customer: Customer,
    items: Vec<LineItem>,
}

impl Order {
    fn total_cents(&self) -> i64 {
        self.items
            .iter()
            .map(|item| item.quantity * item.unit_price_cents)
            .sum()
    }

    fn item_count(&self) -> i64 {
        self.items.iter().map(|item| item.quantity).sum()
    }
}

struct Decision {
    approved: bool,
    action: String,
    discount_cents: i64,
    shipping_tier: String,
    eta_days: i64,
    reason: String,
    tags: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let script_path = parse_script_path()?;
    let (script_name, script_source) = load_script(script_path.as_deref())?;

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;
    register_host_functions(&mut vm)?;
    vm.execute(&script_source)?;

    println!("=== luars rules engine demo ===");
    println!("rules source: {script_name}");
    println!("scenario: checkout approval, discounting, shipping, and manual review");
    println!();

    let evaluator = vm
        .get_global_function("evaluate_order")?
        .ok_or_else(|| io::Error::other("Lua global evaluate_order was not defined"))?;

    for order in sample_orders() {
        let order_value = build_order_table(&mut vm, &order)?;
        let decision = evaluate_order(&mut vm, &evaluator, order_value)?;
        print_decision(&order, &decision);
    }

    Ok(())
}

fn parse_script_path() -> AppResult<Option<PathBuf>> {
    let mut args = env::args().skip(1);
    let mut script = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--script" | "-s" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing path after --script")
                })?;
                script = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {other}"),
                )
                .into());
            }
        }
    }

    Ok(script)
}

fn print_help() {
    println!("rules-engine-demo [--script PATH]");
}

fn load_script(script_path: Option<&Path>) -> AppResult<(String, String)> {
    match script_path {
        Some(path) => Ok((
            path.display().to_string(),
            fs::read_to_string(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to read script {}: {error}", path.display()),
                )
            })?,
        )),
        None => Ok((
            "embedded scripts/checkout_rules.lua".to_string(),
            DEFAULT_SCRIPT.to_string(),
        )),
    }
}

fn register_host_functions(vm: &mut LuaVM) -> AppResult<()> {
    vm.register_function_typed(
        "risk_score",
        |email: String, total_cents: i64, country: String| -> i64 {
            let domain = email.split('@').nth(1).unwrap_or_default();
            let mut score = match domain {
                "vip.example" => 8,
                "corp.example" => 18,
                "maildrop.test" => 72,
                _ => 28,
            };

            if total_cents >= 20_000 {
                score += 18;
            }
            if total_cents >= 50_000 {
                score += 12;
            }
            if matches!(country.as_str(), "BR" | "NG") {
                score += 14;
            }

            score.min(99)
        },
    )?;

    vm.register_function_typed(
        "inventory_available",
        |sku: String, quantity: i64| -> bool {
            let available_units = match sku.as_str() {
                "LAPTOP-15" => 8,
                "MECH-KEYBOARD" => 40,
                "BATTERY-PACK" => 1,
                "GLASS-MUG" => 12,
                "CLOUD-CREDIT" => 999,
                _ => 0,
            };

            quantity <= available_units
        },
    )?;

    vm.register_function_typed("shipping_eta", |country: String, tier: String| -> i64 {
        match (country.as_str(), tier.as_str()) {
            (_, "hold") => 0,
            ("CN", "express") => 1,
            ("CN", "priority") => 2,
            ("US", "express") => 2,
            ("US", "priority") => 3,
            ("DE", "priority") => 3,
            (_, "express") => 3,
            (_, "priority") => 4,
            _ => 6,
        }
    })?;

    vm.register_function("audit", |state| {
        let message = state
            .get_arg(1)
            .and_then(|value| value.as_str().map(str::to_owned));
        if let Some(message) = message {
            println!("  [lua audit] {message}");
        }
        Ok(0)
    })?;

    Ok(())
}

fn sample_orders() -> Vec<Order> {
    vec![
        Order {
            id: "ORD-1001",
            coupon_code: Some("FLASH10"),
            customer: Customer {
                email: "alice@vip.example",
                country: "CN",
                vip: true,
                loyalty_points: 1800,
            },
            items: vec![
                LineItem {
                    sku: "LAPTOP-15",
                    category: "electronics",
                    quantity: 1,
                    unit_price_cents: 12_900,
                },
                LineItem {
                    sku: "MECH-KEYBOARD",
                    category: "electronics",
                    quantity: 1,
                    unit_price_cents: 4_000,
                },
            ],
        },
        Order {
            id: "ORD-1002",
            coupon_code: None,
            customer: Customer {
                email: "eve@maildrop.test",
                country: "BR",
                vip: false,
                loyalty_points: 90,
            },
            items: vec![
                LineItem {
                    sku: "BATTERY-PACK",
                    category: "hazmat",
                    quantity: 2,
                    unit_price_cents: 7_500,
                },
                LineItem {
                    sku: "CLOUD-CREDIT",
                    category: "digital",
                    quantity: 1,
                    unit_price_cents: 20_000,
                },
            ],
        },
        Order {
            id: "ORD-1003",
            coupon_code: Some("FLASH10"),
            customer: Customer {
                email: "ops@corp.example",
                country: "US",
                vip: false,
                loyalty_points: 120,
            },
            items: vec![
                LineItem {
                    sku: "GLASS-MUG",
                    category: "fragile",
                    quantity: 4,
                    unit_price_cents: 1_800,
                },
                LineItem {
                    sku: "CLOUD-CREDIT",
                    category: "digital",
                    quantity: 1,
                    unit_price_cents: 8_000,
                },
            ],
        },
    ]
}

fn build_order_table(vm: &mut LuaVM, order: &Order) -> AppResult<LuaValue> {
    let items = build_items_table(vm, &order.items)?;
    let customer = TableBuilder::new()
        .set("email", vm.create_string(order.customer.email)?)
        .set("country", vm.create_string(order.customer.country)?)
        .set("vip", LuaValue::boolean(order.customer.vip))
        .set(
            "loyalty_points",
            LuaValue::integer(order.customer.loyalty_points),
        )
        .build(vm)?;

    let coupon_value = match order.coupon_code {
        Some(code) => vm.create_string(code)?,
        None => LuaValue::nil(),
    };

    Ok(TableBuilder::new()
        .set("id", vm.create_string(order.id)?)
        .set("coupon_code", coupon_value)
        .set("total_cents", LuaValue::integer(order.total_cents()))
        .set("item_count", LuaValue::integer(order.item_count()))
        .set("customer", customer)
        .set("items", items)
        .build(vm)?)
}

fn build_items_table(vm: &mut LuaVM, items: &[LineItem]) -> AppResult<LuaValue> {
    let mut builder = TableBuilder::new();

    for item in items {
        let item_table = TableBuilder::new()
            .set("sku", vm.create_string(item.sku)?)
            .set("category", vm.create_string(item.category)?)
            .set("quantity", LuaValue::integer(item.quantity))
            .set("unit_price_cents", LuaValue::integer(item.unit_price_cents))
            .build(vm)?;
        builder = builder.push(item_table);
    }

    Ok(builder.build(vm)?)
}

fn evaluate_order(
    vm: &mut LuaVM,
    evaluator: &luars::LuaFunctionRef,
    order_value: LuaValue,
) -> AppResult<Decision> {
    let decision_value = evaluator.call1_raw(vec![order_value])?;
    let decision_table = vm
        .to_table_ref(decision_value)
        .ok_or_else(|| io::Error::other("evaluate_order must return a table"))?;

    let tags_value = decision_table.get("tags")?;
    let tags_table = vm
        .to_table_ref(tags_value)
        .ok_or_else(|| io::Error::other("decision.tags must be a table"))?;

    let mut tags = Vec::new();
    for index in 1..=tags_table.len()? {
        let value = tags_table.geti(index as i64)?;
        if let Some(tag) = value.as_str() {
            tags.push(tag.to_string());
        }
    }

    Ok(Decision {
        approved: decision_table.get_as("approved")?,
        action: decision_table.get_as("action")?,
        discount_cents: decision_table.get_as("discount_cents")?,
        shipping_tier: decision_table.get_as("shipping_tier")?,
        eta_days: decision_table.get_as("eta_days")?,
        reason: decision_table.get_as("reason")?,
        tags,
    })
}

fn print_decision(order: &Order, decision: &Decision) {
    println!("order {}", order.id);
    println!(
        "  customer: {} ({}) | total: {} | items: {}",
        order.customer.email,
        order.customer.country,
        format_money(order.total_cents()),
        order.item_count()
    );
    println!(
        "  result: {} | approved: {} | shipping: {} | eta: {} day(s)",
        decision.action, decision.approved, decision.shipping_tier, decision.eta_days
    );
    println!("  discount: {}", format_money(decision.discount_cents));
    println!("  reason: {}", decision.reason);
    println!("  tags: {}", decision.tags.join(", "));
    println!();
}

fn format_money(cents: i64) -> String {
    format!("${:.2}", cents as f64 / 100.0)
}

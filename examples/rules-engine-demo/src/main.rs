use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use luars::{Lua, SafeOption, Stdlib, Table};

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

    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;
    register_host_functions(&mut lua)?;
    lua.load(&script_source).set_name(&script_name).exec()?;

    println!("=== luars rules engine demo ===");
    println!("rules source: {script_name}");
    println!();

    for order in sample_orders() {
        let order_value = build_order_table(&mut lua, &order)?;
        let decision = evaluate_order(&mut lua, order_value)?;
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
                println!("rules-engine-demo [--script PATH]");
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

fn register_host_functions(lua: &mut Lua) -> AppResult<()> {
    lua.register_function(
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

    lua.register_function(
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

    lua.register_function("shipping_eta", |country: String, tier: String| -> i64 {
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

    lua.register_function("audit", |message: String| {
        println!("  [lua audit] {message}");
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
    ]
}

fn build_order_table(lua: &mut Lua, order: &Order) -> AppResult<Table> {
    let customer = lua.create_table()?;
    customer.set("email", order.customer.email)?;
    customer.set("country", order.customer.country)?;
    customer.set("vip", order.customer.vip)?;
    customer.set("loyalty_points", order.customer.loyalty_points)?;

    let items = lua.create_table()?;
    for item in &order.items {
        items.push(build_line_item(lua, item)?)?;
    }

    let table = lua.create_table()?;
    table.set("id", order.id)?;
    table.set("coupon_code", order.coupon_code)?;
    table.set("total_cents", order.total_cents())?;
    table.set("item_count", order.item_count())?;
    table.set("customer", customer)?;
    table.set("items", items)?;
    Ok(table)
}

fn build_line_item(lua: &mut Lua, item: &LineItem) -> AppResult<Table> {
    let table = lua.create_table()?;
    table.set("sku", item.sku)?;
    table.set("category", item.category)?;
    table.set("quantity", item.quantity)?;
    table.set("unit_price_cents", item.unit_price_cents)?;
    Ok(table)
}

fn evaluate_order(lua: &mut Lua, order: Table) -> AppResult<Decision> {
    let decision: Table = lua.call_global1("evaluate_order", order)?;
    let tags: Table = decision.get("tags")?;

    Ok(Decision {
        approved: decision.get("approved")?,
        action: decision.get("action")?,
        discount_cents: decision.get("discount_cents")?,
        shipping_tier: decision.get("shipping_tier")?,
        eta_days: decision.get("eta_days")?,
        reason: decision.get("reason")?,
        tags: tags.sequence_values::<String>()?,
    })
}

fn print_decision(order: &Order, decision: &Decision) {
    println!(
        "order={} total={} approved={}",
        order.id,
        order.total_cents(),
        decision.approved
    );
    println!("  action: {}", decision.action);
    println!("  discount_cents: {}", decision.discount_cents);
    println!("  shipping_tier: {}", decision.shipping_tier);
    println!("  eta_days: {}", decision.eta_days);
    println!("  reason: {}", decision.reason);
    println!("  tags: {}", decision.tags.join(", "));
    println!();
}

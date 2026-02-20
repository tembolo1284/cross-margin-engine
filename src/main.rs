use cross_margin_engine::engine::Engine;
use cross_margin_engine::events::EventType;
use cross_margin_engine::margin;
use cross_margin_engine::snapshot::Snapshot;
use cross_margin_engine::types::Market;

use rust_decimal_macros::dec;

fn main() {
    println!("=== Cross-Margin Perpetual Risk Engine Demo ===\n");

    let mut engine = Engine::new();

    // Configure markets
    engine.add_market(Market::new("BTC-PERP".into(), dec!(0.05), dec!(0.03)));
    engine.add_market(Market::new("ETH-PERP".into(), dec!(0.10), dec!(0.05)));

    // ─── Scenario 1: Healthy portfolio becomes liquidatable ─────────────────

    println!("--- Scenario 1: Liquidation after adverse price move ---\n");

    engine.process(EventType::Deposit {
        account_id: "alice".into(),
        amount: dec!(100000),
    });
    print_account(&engine, "alice", "Alice deposits 100,000");

    engine.process(EventType::MarkPriceUpdate {
        market_id: "BTC-PERP".into(),
        price: dec!(50000),
    });

    engine.process(EventType::TradeFill {
        account_id: "alice".into(),
        market_id: "BTC-PERP".into(),
        quantity: dec!(10),
        price: dec!(50000),
    });
    print_account(&engine, "alice", "Alice longs 10 BTC-PERP @ 50,000");

    engine.process(EventType::MarkPriceUpdate {
        market_id: "BTC-PERP".into(),
        price: dec!(42000),
    });
    print_account(&engine, "alice", "BTC drops to 42,000 — still healthy");

    engine.process(EventType::MarkPriceUpdate {
        market_id: "BTC-PERP".into(),
        price: dec!(41000),
    });
    print_account(&engine, "alice", "BTC drops to 41,000 — LIQUIDATED");

    // ─── Scenario 2: Trade rejected on margin ──────────────────────────────

    println!("--- Scenario 2: Trade rejected due to insufficient margin ---\n");

    engine.process(EventType::Deposit {
        account_id: "bob".into(),
        amount: dec!(10000),
    });
    print_account(&engine, "bob", "Bob deposits 10,000");

    engine.process(EventType::MarkPriceUpdate {
        market_id: "ETH-PERP".into(),
        price: dec!(3000),
    });

    engine.process(EventType::TradeFill {
        account_id: "bob".into(),
        market_id: "ETH-PERP".into(),
        quantity: dec!(20),
        price: dec!(3000),
    });
    print_account(&engine, "bob", "Bob longs 20 ETH-PERP @ 3,000 — accepted");

    engine.process(EventType::TradeFill {
        account_id: "bob".into(),
        market_id: "ETH-PERP".into(),
        quantity: dec!(20),
        price: dec!(3000),
    });
    print_account(&engine, "bob", "Bob tries 20 more ETH-PERP — REJECTED");

    // Show the rejection reason
    if let Some(event) = engine.event_log.iter().rev().find(|e| {
        matches!(e.event_type, EventType::TradeRejected { .. })
    }) {
        if let EventType::TradeRejected { reason, .. } = &event.event_type {
            println!("    Rejection reason: {reason}\n");
        }
    }

    // ─── Scenario 3: Cross-margin portfolio-level rejection ────────────────

    println!("--- Scenario 3: Cross-margin portfolio constraint ---\n");

    engine.process(EventType::Deposit {
        account_id: "charlie".into(),
        amount: dec!(20000),
    });
    print_account(&engine, "charlie", "Charlie deposits 20,000");

    // BTC-PERP mark is still 41,000 from scenario 1 — set it to 50,000 for clean math
    engine.process(EventType::MarkPriceUpdate {
        market_id: "BTC-PERP".into(),
        price: dec!(50000),
    });

    engine.process(EventType::TradeFill {
        account_id: "charlie".into(),
        market_id: "BTC-PERP".into(),
        quantity: dec!(5),
        price: dec!(50000),
    });
    print_account(&engine, "charlie", "Charlie longs 5 BTC-PERP @ 50,000 (IM: 12,500)");

    // This trade alone would need IM of 9,000 — but combined with BTC position,
    // total IM would be 21,500 which exceeds equity of 20,000
    engine.process(EventType::TradeFill {
        account_id: "charlie".into(),
        market_id: "ETH-PERP".into(),
        quantity: dec!(30),
        price: dec!(3000),
    });
    print_account(&engine, "charlie", "Charlie tries 30 ETH-PERP — REJECTED (combined IM too high)");

    if let Some(event) = engine.event_log.iter().rev().find(|e| {
        matches!(e.event_type, EventType::TradeRejected { .. })
    }) {
        if let EventType::TradeRejected { reason, .. } = &event.event_type {
            println!("    Rejection reason: {reason}");
            println!("    (Either position alone would pass — it's the cross-margin total that fails)\n");
        }
    }

    // Smaller ETH position that fits within remaining margin
    engine.process(EventType::TradeFill {
        account_id: "charlie".into(),
        market_id: "ETH-PERP".into(),
        quantity: dec!(15),
        price: dec!(3000),
    });
    print_account(&engine, "charlie", "Charlie longs 15 ETH-PERP — ACCEPTED (combined IM fits)");

    // ─── Scenario 4: Funding payment ───────────────────────────────────────

    println!("--- Scenario 4: Funding payment applied ---\n");

    print_account(&engine, "bob", "Bob before funding (long ETH)");

    engine.process(EventType::FundingUpdate {
        market_id: "ETH-PERP".into(),
        new_cumulative_index: dec!(1.50),
    });
    print_account(&engine, "bob", "After funding — Bob (long) pays");

    // ─── Replay Determinism Verification ───────────────────────────────────

    println!("--- Replay Determinism Verification ---\n");

    let original_log = engine.event_log.clone();
    let original_snapshots = engine.snapshots.clone();
    let original_state = engine.state.clone();

    let markets = vec![
        Market::new("BTC-PERP".into(), dec!(0.05), dec!(0.03)),
        Market::new("ETH-PERP".into(), dec!(0.10), dec!(0.05)),
    ];

    let (replay_state, replay_snapshots) = Engine::replay(&original_log, markets);

    let states_match = original_state == replay_state;
    println!(
        "  Final state match:  {}",
        if states_match { "✓ PASS" } else { "✗ FAIL" }
    );

    let snapshots_match = compare_snapshots(&original_snapshots, &replay_snapshots);
    println!(
        "  Path determinism ({} snapshots): {}",
        original_snapshots.len(),
        if snapshots_match { "✓ PASS" } else { "✗ FAIL" }
    );

    // ─── Event Log ─────────────────────────────────────────────────────────

    println!("\n--- Event Log ({} events) ---\n", original_log.len());
    for event in &original_log {
        let json = serde_json::to_string(event).unwrap();
        println!("  {json}");
    }

    // Write event log to file
    let log_path = "scenarios/demo.jsonl";
    std::fs::create_dir_all("scenarios").ok();
    let log_content: String = original_log
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(log_path, log_content).expect("Failed to write event log");
    println!("\n  Event log written to {log_path}");
}

fn print_account(engine: &Engine, account_id: &str, label: &str) {
    println!("  {label}");
    if let Some(account) = engine.state.accounts.get(account_id) {
        let eq = margin::equity(account, &engine.state);
        let im = margin::initial_margin_required(account, &engine.state);
        let mm = margin::maintenance_margin_required(account, &engine.state);
        let upnl = margin::total_unrealized_pnl(account, &engine.state);
        let liq = margin::is_liquidatable(account, &engine.state);

        println!("    Collateral:   {}", account.collateral);
        println!("    Unrealized:   {upnl}");
        println!("    Equity:       {eq}");
        println!("    IM Required:  {im}");
        println!("    MM Required:  {mm}");
        println!("    Liquidatable: {liq}");
        for (mid, pos) in &account.positions {
            println!(
                "    Position {mid}: qty={} cost_basis={}",
                pos.quantity, pos.cost_basis
            );
        }
    } else {
        println!("    (no account)");
    }
    println!();
}

fn compare_snapshots(a: &[Snapshot], b: &[Snapshot]) -> bool {
    if a.len() != b.len() {
        println!(
            "    Snapshot count mismatch: {} vs {}",
            a.len(),
            b.len()
        );
        return false;
    }
    for (i, (sa, sb)) in a.iter().zip(b.iter()).enumerate() {
        if sa != sb {
            println!(
                "    Snapshot mismatch at index {i} (after seq {})",
                sa.after_sequence
            );
            return false;
        }
    }
    true
}

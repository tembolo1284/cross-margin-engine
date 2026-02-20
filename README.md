# cross-margin-engine

Deterministic event-sourced cross-margin risk engine for perpetual futures.

## Overview

A minimal off-chain risk engine that maintains cross-margined perpetual positions across multiple markets sharing a single collateral pool. The engine processes an ordered event log and guarantees identical state reconstruction on replay.

See [DESIGN.md](DESIGN.md) for architectural decisions, tradeoffs, and detailed formulas.

## Quick Start
```bash
# Build
cargo build

# Run the demo
cargo run
```

The demo runs four scenarios followed by a full replay verification:

1. **Liquidation** — A healthy portfolio becomes liquidatable after adverse price movement
2. **Trade rejection** — A trade is rejected because it would violate initial margin
3. **Cross-margin rejection** — A trade passes in isolation but is rejected because the combined portfolio margin across two markets exceeds equity
4. **Funding** — A funding payment is applied, reducing a long position's collateral

After all scenarios, the engine replays the full event log from scratch and verifies that every intermediate state snapshot matches the original run.

## Demo Output
```
--- Replay Determinism Verification ---

  Final state match:  ✓ PASS
  Path determinism (N snapshots): ✓ PASS
```

The event log is written to `scenarios/demo.jsonl` for inspection.

## Architecture
```
src/
├── types.rs          Core data: Account, Position, Market
├── events.rs         Event enum with JSON serialization
├── state.rs          State container and accessors
├── margin.rs         Equity, margin, health — pure functions
├── risk.rs           Pre-trade simulation and validation
├── liquidation.rs    Detection and execution
├── engine.rs         Event processing, live mode, replay
├── snapshot.rs       State snapshots for determinism verification
├── lib.rs            Public re-exports
└── main.rs           Demo runner
```

**Data flow:**
```
Event Log → Engine.apply_event() → State mutation
                                        ↓
                              Engine.process() scans for liquidations
                                        ↓
                              LiquidationFill events appended to log
```

In replay mode, the same `apply_event()` function processes all events including `LiquidationFill` entries already in the log. No liquidation scanning occurs during replay — the log is a complete, self-contained record.

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Language | Rust | Type safety, exact decimal arithmetic via `rust_decimal`, no GC |
| Arithmetic | `rust_decimal` (96-bit) | Exact decimal math, deterministic across platforms |
| Position model | Signed quantity + cost basis | No side-enum branching, cost basis is additive |
| Funding | Cumulative index, eager settlement | O(1) per settlement, isolates funding logic |
| Cross-margin | Additive, no offsets | Conservative, standard base model |
| Liquidation | Full close, largest notional first, at mark price | Avoids partial-close solver and order book modeling |
| Determinism | BTreeMap ordering, sequence numbers, no external state | Deterministic by construction |

## Event Types

| Event | Description |
|---|---|
| `Deposit` | Add collateral to an account |
| `Withdraw` | Remove collateral (gated by initial margin) |
| `TradeFill` | Open, increase, reduce, close, or flip a position |
| `MarkPriceUpdate` | Update a market's mark price (triggers liquidation scan) |
| `FundingUpdate` | Update cumulative funding index (settles funding) |
| `LiquidationFill` | Engine-generated close of a liquidated position |
| `TradeRejected` | Informational — trade failed margin check |
| `WithdrawalRejected` | Informational — withdrawal failed margin check |

## Margin Model
```
Position Notional      = abs(mark_price × quantity)
Initial Margin (IM)    = Σ notional_i × im_fraction_i
Maintenance Margin (MM) = Σ notional_i × mm_fraction_i
Portfolio Equity        = collateral + Σ unrealized_pnl_i
Liquidatable when       equity ≤ MM
Trade allowed when      simulated_equity ≥ simulated_IM
```

## AI Usage

Claude (Anthropic) was used as a design collaborator throughout development. All architectural decisions, margin formulas, and tradeoffs were discussed interactively before implementation. Claude assisted with Rust scenarios creation and debugging only. All output was reviewed, tested, and validated by me the author!

# cross-margin-engine

Deterministic event-sourced cross-margin risk engine for perpetual futures.

## Overview

A minimal off-chain risk engine that maintains cross-margined perpetual positions across multiple markets sharing a single collateral pool. The engine processes an ordered event log and guarantees identical state reconstruction on replay.

The core risk metric is **margin excess** — the dollar buffer between portfolio equity and the maintenance margin threshold. When this goes to zero or negative, the exchange's credit exposure to the trader is at risk, and the engine intervenes via liquidation.

See [DESIGN.md](DESIGN.md) for architectural decisions, tradeoffs, and detailed formulas.

## Quick Start
```bash
# Build
cargo build

# Run the demo
cargo run
```

The demo runs five scenarios:

1. **Liquidation** — A healthy portfolio becomes liquidatable after adverse price movement
2. **Trade rejection** — A trade is rejected because it would violate initial margin
3. **Cross-margin rejection** — A trade passes in isolation but is rejected because the combined portfolio margin across two markets exceeds equity
4. **Funding** — A funding payment is applied, reducing a long position's collateral
5. **Replay determinism** — The full event log is replayed from scratch; every intermediate state snapshot is verified identical

## Demo Output
```
--- Replay Determinism Verification ---

  Final state match:  PASS
  Path determinism (18 snapshots): PASS
```

The event log is written to `scenarios/demo.jsonl` for inspection.

## Architecture
```
src/
├── types.rs          Core data: Account, Position, Market
├── events.rs         Event enum with explicit string-serialized Decimals
├── state.rs          State container and accessors
├── margin.rs         Equity, margin, health — pure functions
├── risk.rs           Pre-trade simulation, validation, trade application
├── liquidation.rs    Detection (largest notional first) and execution
├── engine.rs         Event processing, live mode, replay
├── snapshot.rs       State snapshots for determinism verification
├── lib.rs            Public re-exports
└── main.rs           Demo runner with five scenarios
```

**Data flow:**
```
Event Log → Engine.apply_event() → Pure state mutation
                                        ↓
                              Engine.process() scans for liquidations
                                        ↓
                              LiquidationFill events appended to log
```

`apply_event()` performs pure state mutation — no scanning, no event generation. In live mode, the orchestrator (`process`) handles liquidation scanning after each event. In replay mode, `apply_event()` processes all events including `LiquidationFill` entries already in the log. The same function, both paths.

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Language | Rust | Type safety, exact decimal arithmetic via `rust_decimal`, no GC |
| Arithmetic | `rust_decimal` (96-bit) with explicit `serde(with = "str")` | Exact decimal math, deterministic serialization |
| Position model | Signed quantity + cost basis | No side-enum branching, cost basis is additive |
| Funding | Cumulative index, eager settlement | O(1) per settlement, isolates funding logic |
| Cross-margin | Additive, no offsets | Conservative, standard base model |
| Liquidation | Full close, largest notional first (tie-break by market ID), at mark price | Deterministic ordering, avoids partial-close solver |
| Bankruptcy | Explicit `bankruptcy_deficit` field on Account | Auditable, replay-stable, no inference from negative collateral |
| Determinism | BTreeMap/BTreeSet ordering, sequence numbers, no external state | Deterministic by construction |
| Defensive lookups | `unwrap_or(ZERO)` for missing markets | Deterministic degradation instead of panics |

## Event Types

| Event | Description |
|---|---|
| `Deposit` | Add collateral to an account |
| `Withdraw` | Remove collateral (gated by initial margin) |
| `TradeFill` | Open, increase, reduce, close, or flip a position |
| `MarkPriceUpdate` | Update a market's mark price (triggers liquidation scan) |
| `FundingUpdate` | Update cumulative funding index (settles funding eagerly) |
| `LiquidationFill` | Engine-generated close of a liquidated position |
| `TradeRejected` | Informational — trade failed margin check |
| `WithdrawalRejected` | Informational — withdrawal failed margin check |

## Margin Model
```
Position Notional       = abs(mark_price × quantity)
Initial Margin (IM)     = sum over i notional_i × im_fraction_i
Maintenance Margin (MM) = sum over i notional_i × mm_fraction_i
Portfolio Equity        = collateral + sum over i unrealized_pnl_i
Margin Excess           = equity - MM  (core risk metric)
Liquidatable when       equity <= MM
Trade allowed when      simulated_equity >= simulated_IM
```

## AI Usage

Claude (Anthropic) was used as a design collaborator throughout development. All architectural decisions, margin formulas, and tradeoffs were discussed interactively before implementation. Claude assisted with Rust scenario generation and debugging. All output was reviewed, tested, and validated by the author.

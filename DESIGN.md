# Cross-Margin Perpetual Risk Engine — Design Document

## Overview

This is a deterministic, event-sourced cross-margin risk engine for perpetual futures. It maintains portfolio state across multiple markets sharing a single collateral pool, enforces margin requirements, detects and executes liquidations, and guarantees identical state reconstruction on replay.

**Language choice: Rust.** The type system encodes invariants (e.g., signed quantities, exhaustive event matching). The `rust_decimal` crate provides exact decimal arithmetic without IEEE 754 nondeterminism. There is no garbage collector to introduce timing variability. These properties make determinism natural rather than something bolted on after the fact.

**AI usage:** Claude (Anthropic) was used as a design collaborator throughout. All architectural decisions, formulas, and tradeoffs were discussed interactively before any code was written. Claude also assisted with Rust implementation. All output was reviewed, tested, and validated by the author.

---

## State Model

### Account

```
Account {
    account_id:    String,
    collateral:    Decimal,                         // realized cash balance
    positions:     BTreeMap<MarketId, Position>,     // open positions
    last_funding:  BTreeMap<MarketId, Decimal>,      // cumulative funding index at last settlement
}
```

`collateral` reflects all realized cash flows: deposits, withdrawals, realized PnL from closed trades, and settled funding. Unrealized PnL is never stored — it is always computed dynamically from mark prices.

### Position

```
Position {
    market_id:  String,
    quantity:   Decimal,    // signed: positive = long, negative = short
    cost_basis: Decimal,    // total cost, signed (quantity * avg_entry_price)
}
```

**Signed quantity** eliminates side-enum branching. PnL, trade application, and margin math all work through sign conventions without conditional logic.

**Cost basis instead of average entry price.** Cost basis is additive when increasing a position and proportionally reducible when decreasing. Average entry price is always recoverable as `cost_basis / quantity`. This avoids a class of rounding bugs that arise from recomputing averages on partial closes.

### Market

```
Market {
    market_id:                  String,
    mark_price:                 Decimal,
    initial_margin_fraction:    Decimal,    // e.g., 0.05 (5%)
    maintenance_margin_fraction: Decimal,   // e.g., 0.03 (3%)
    cumulative_funding_index:   Decimal,    // per-unit cumulative funding
}
```

Margin fractions are flat per market. A production system would tier these by position size — we use flat rates for simplicity.

The `cumulative_funding_index` enables efficient funding settlement. Instead of iterating every account on every funding tick, each account stores the index at its last settlement. The funding owed is `(last_index - current_index) * quantity`. Settlement is O(1) per account-market pair.

### Event Types

```
Deposit          { account_id, amount }
Withdraw         { account_id, amount }
TradeFill        { account_id, market_id, quantity, price }
MarkPriceUpdate  { market_id, price }
FundingUpdate    { market_id, funding_rate }
LiquidationFill  { account_id, market_id, quantity, price }
```

Every event carries a monotonically increasing `sequence` number. This is the sole ordering mechanism — the engine never branches on timestamps.

`LiquidationFill` is an engine-generated event. During live operation, the engine detects liquidatable accounts and emits these events into the log. During replay, they are consumed as regular events. This makes the log a complete, self-contained record.

---

## Equity and Margin Computation

### Unrealized PnL

Per position:

```
unrealized_pnl = (mark_price * quantity) - cost_basis
```

The signed convention handles all cases without branching:

| Position | Mark Move | Calculation | Result |
|---|---|---|---|
| Long 10 @ 50,000 cost | Mark → 52,000 | (52,000 × 10) − 500,000 | +20,000 |
| Short 10 @ 50,000 cost | Mark → 52,000 | (52,000 × −10) − (−500,000) | −20,000 |
| Short 10 @ 50,000 cost | Mark → 48,000 | (48,000 × −10) − (−500,000) | +20,000 |

### Portfolio Equity

```
equity = collateral + Σ unrealized_pnl_i
```

Funding is settled eagerly into collateral when `FundingUpdate` events arrive, so there is no unsettled funding term in the equity formula at evaluation time.

### Funding Settlement

On a `FundingUpdate` event for a market, for each account holding a position:

```
funding_delta = (account_last_index - new_index) * position_quantity
collateral += funding_delta
account_last_index = new_index
```

Sign convention: when the index increases, longs pay and shorts receive. The subtraction order `(last - current)` with signed quantity produces the correct sign automatically.

**Design choice: eager settlement.** Funding is applied to collateral immediately when the event arrives. This isolates funding logic to one event handler and keeps the equity formula simple. The cost is iterating affected accounts on each funding event, which is acceptable for this scope.

### Margin Requirements

```
position_notional_i        = abs(mark_price_i * quantity_i)
initial_margin_required    = Σ (notional_i * initial_margin_fraction_i)
maintenance_margin_required = Σ (notional_i * maintenance_margin_fraction_i)
```

This is an **additive cross-margin model**. Each position contributes independently to the total requirement, but all positions draw from the shared collateral pool. There are no offset credits for correlated or hedged positions.

This is conservative (it overstates requirements relative to portfolio-margining with offsets) and is the standard base model used by most perpetual exchanges.

### Health Evaluation

```
Healthy:       equity > maintenance_margin_required
Liquidatable:  equity <= maintenance_margin_required
```

The engine uses direct comparison rather than a margin ratio to avoid division-by-zero edge cases when equity is zero or negative. A `margin_ratio` (maintenance_margin / equity) is computed for display purposes only.

---

## Pre-Trade Risk Checks

### Principle

Every state mutation that could worsen margin health is validated before application. The engine simulates post-trade state without mutating anything, evaluates it against initial margin, and commits only if it passes.

### Trade Simulation

When a `TradeFill { account_id, market_id, quantity, price }` arrives:

**1. Compute the simulated position.**

```
new_quantity = old_quantity + fill_quantity
```

Four cases for cost basis:

**Increasing (same direction):**
```
new_cost_basis = old_cost_basis + (fill_quantity * fill_price)
```

**Reducing (opposite direction, partial close):**
```
close_ratio    = fill_quantity / old_quantity  (negative, since opposite directions)
realized_pnl   = -close_ratio * old_cost_basis + (fill_quantity * fill_price)
new_cost_basis = old_cost_basis * (1 + close_ratio)
```

**Closing exactly (new_quantity = 0):**
```
realized_pnl   = (fill_quantity * fill_price) + old_cost_basis
new_cost_basis = 0
```

**Flipping (crosses zero):** Decomposed into a full close of the old position followed by an open in the new direction. Realized PnL comes from the close; the remaining quantity starts a fresh position at the fill price.

**2. Compute simulated equity.**

```
simulated_collateral  = old_collateral + realized_pnl
simulated_unrealized  = Σ (mark_price_i * simulated_qty_i - simulated_cost_basis_i)
simulated_equity      = simulated_collateral + simulated_unrealized
```

**3. Compute simulated initial margin.**

```
simulated_im = Σ abs(mark_price_i * simulated_qty_i) * im_fraction_i
```

**4. Accept or reject.**

```
if simulated_equity >= simulated_im → apply trade
else → reject, state unchanged
```

### Risk-Reducing Trades

Trades that reduce absolute position size are always allowed, even if the account is below initial margin. An account between maintenance and initial margin cannot open new risk but must be able to close existing risk. Without this, trapped accounts could not de-risk without being liquidated.

A trade is risk-reducing when `abs(new_quantity) < abs(old_quantity)` and it does not flip the position.

### Withdrawal Check

```
allowed if: (equity - withdrawal_amount) >= initial_margin_required
            AND withdrawal_amount <= collateral
```

Unrealized profits are not withdrawable. This is a conservative simplification — some exchanges permit partial withdrawal against unrealized gains.

---

## Liquidation

### Detection

Liquidation checks run at **defined, deterministic points**:

| After Event | Scope |
|---|---|
| `MarkPriceUpdate` | All accounts with a position in that market |
| `FundingUpdate` | All accounts with a position in that market |
| `TradeFill` (applied) | The affected account only |
| `LiquidationFill` | No scan (prevents recursive liquidation) |
| `Deposit` | No scan (health only improves) |
| `Withdraw` (applied) | No scan (IM check already passed) |

When multiple accounts are liquidatable, they are processed in **account ID order** (BTreeMap iteration) for deterministic behavior.

### Execution

Simplified model: **full position closure at mark price, one position at a time, largest notional first.**

For a liquidatable account:

1. Rank positions by `abs(mark_price * quantity)`, descending.
2. Close the largest position at mark price:
   - `realized_pnl = (mark_price * quantity) - cost_basis`
   - `collateral += realized_pnl`
   - Remove position.
   - Emit a `LiquidationFill` event to the log.
3. Recheck equity vs. maintenance margin.
4. If still liquidatable and positions remain, continue to next position.
5. If all positions closed and collateral is negative, the account is bankrupt. The deficit is tracked.

### Why These Simplifications

**Mark price execution** removes the need to model an order book or auction mechanism. In production, the gap between mark and execution price is slippage, covered by insurance funds and liquidation penalties.

**Full position closure** avoids solving for the minimum close quantity. A partial liquidation requires solving a nonlinear equation (closing a portion changes both equity and margin simultaneously). The full-close-one-at-a-time approach is a reasonable middle ground for a demo.

**No insurance fund.** Deficits are tracked but not absorbed. A production system would maintain an insurance pool funded by liquidation penalties, with auto-deleveraging (ADL) as a backstop.

---

## Determinism

### Arithmetic

The engine uses `rust_decimal::Decimal` — a 96-bit integer mantissa with a decimal scale factor (0–28 places). This provides exact decimal arithmetic with no IEEE 754 representation error, deterministic results across platforms and replays, and up to 28 significant digits.

All numeric values in the event log are serialized as strings in JSON to avoid floating-point representation issues during deserialization.

### Rounding Policy

Even with exact decimal arithmetic, division can produce results that exceed representable precision. The rounding policy is conservative from a risk perspective:

- **Margin requirements:** Round up. Never understate required margin.
- **Equity / PnL in account's favor:** Round down. Never overstate account health.
- **Cost basis reduction on partial close:** Round toward zero on the realized portion.
- **All rounding is applied at state mutation boundaries**, not during intermediate computation.

In practice, with `rust_decimal`'s 28-digit precision, rounding is rarely triggered in this demo's value ranges. The policy exists as a documented safeguard.

### Structural Guarantees

1. **Total event ordering.** The `sequence` number is the sole ordering mechanism. No timestamps, no concurrency.
2. **No external state.** The engine reads only from the event log and its own accumulated state. No system clock, no randomness, no IO during processing.
3. **Deterministic iteration.** All maps are `BTreeMap`, giving deterministic traversal order. Account liquidation scanning and processing follows account ID order.
4. **Defined evaluation points.** Liquidation scans happen at exactly the points listed above — no ambiguity about when risk is evaluated.
5. **Liquidation events in the log.** Engine-generated liquidations are appended to the event log, making the log a complete record. Replay consumes them as regular events.

### Replay Architecture

The engine has a single event processing function used in both live and replay modes:

```
process_event(state, event) → state'
```

**Live mode:** External events are assigned sequence numbers and appended to the log. After processing, the engine scans for liquidations and appends any resulting `LiquidationFill` events to the log.

**Replay mode:** Events are read from the log in sequence order. The same `process_event` function is called. No liquidation scanning occurs — those events are already in the log.

The `process_event` function is identical in both paths. The only difference is the event source. Since the function is deterministic and the event sequence is identical, the output state is identical.

### Verification

Determinism is verified by:

1. Run the engine in live mode. Collect the event log and final state.
2. Reset to empty state.
3. Replay the full event log.
4. Assert the final state is identical: all collateral balances, position quantities, cost bases, and market state.

The demo additionally captures snapshots after every event and compares them across runs, proving path determinism — not just final-state equivalence.

---

## Demo Scenarios

### Scenario 1: Healthy Portfolio Becomes Liquidatable

An account deposits collateral, opens a leveraged long position, and is liquidated after an adverse price move.

```
1. Alice deposits 100,000
2. BTC-PERP mark price set to 50,000 (5% IM, 3% MM)
3. Alice longs 10 BTC-PERP at 50,000
   → Notional: 500,000 | IM: 25,000 | Equity: 100,000 ✓
4. BTC-PERP mark drops to 41,000
   → Unrealized PnL: -90,000 | Equity: 10,000
   → MM required: 12,300 | 10,000 < 12,300 → LIQUIDATABLE
5. Engine closes position at mark, emits LiquidationFill
   → Realized PnL: -90,000 | Collateral: 10,000 | No positions
```

### Scenario 2: Trade Rejected on Margin

An account attempts to open a position that would exceed initial margin.

```
1. Bob deposits 10,000
2. ETH-PERP mark price set to 3,000 (10% IM, 5% MM)
3. Bob longs 20 ETH-PERP at 3,000
   → Notional: 60,000 | IM: 6,000 | Equity: 10,000 ✓
4. Bob attempts to long 20 more ETH-PERP at 3,000
   → Simulated notional: 120,000 | Simulated IM: 12,000
   → Equity: 10,000 < 12,000 → REJECTED
```

### Scenario 3: Replay Determinism

Both scenarios above run together producing a combined event log. The engine resets and replays the log. State snapshots after every event are compared and shown to be identical.

---

## What Was Simplified

| Simplification | What Production Would Do |
|---|---|
| Flat margin fractions per market | Tiered by position size (larger positions require higher margin) |
| No position offsets or hedge credits | Portfolio margin with correlation-based reductions |
| Mark price as input event | Oracle aggregation from multiple price feeds |
| Funding index as input event | Funding rate computed from mark vs. index price and open interest |
| Liquidation at mark price | Order book execution or liquidation auction with slippage |
| Full position closure | Partial liquidation solving for minimum close quantity |
| No insurance fund | Insurance pool funded by liquidation penalties |
| No auto-deleveraging (ADL) | Force-close profitable counterparties when insurance is depleted |
| No fees | Trading fees, funding fees, liquidation penalties |
| Single-asset collateral | Multi-asset collateral with haircuts |
| No order book / matching | We consume fills, not orders |
| Single-threaded sequential processing | Consensus or sequencing layer for concurrent event sources |
| Rebuild from full log on replay | Periodic snapshots with log truncation |
| JSON event format | Binary serialization (FlatBuffers, Protobuf) for performance |
| No concurrent access | Lock-free structures or actor model for parallel account processing |

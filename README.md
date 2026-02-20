# cross-margin-engine

Deterministic event-sourced cross-margin risk engine for perpetual futures.

## Overview

This engine maintains portfolio state for a cross-margined perpetual exchange where accounts hold positions across multiple markets sharing a single collateral pool. It enforces initial and maintenance margin requirements, detects and executes liquidations, and guarantees identical state reconstruction on replay from an event log.

Built in Rust using exact decimal arithmetic (`rust_decimal`) — no floating point anywhere in the risk path.

## Key Properties

- **Deterministic replay**: same event log always produces identical state, verified at every intermediate step
- **Event-sourced**: the event log is the source of truth; all state is derived by processing events in sequence
- **Cross-margin**: positions across markets share a single collateral pool with portfolio-level risk evaluation
- **Conservative risk model**: additive margin (no hedge offsets), initial margin gates entry, maintenance margin triggers liquidation

## Quick Start
```bash
# Build
cargo build

# Run the demo (three scenarios + replay verification)
cargo run

# Run tests
cargo test
```

The demo runs three scenarios:

1. **Liquidation** — Alice deposits collateral, opens a leveraged BTC long, gets liquidated after a price drop
2. **Trade rejection** — Bob's second trade is rejected because it would exceed initial margin
3. **Funding** — A funding payment is applied, reducing Bob's collateral

After running all scenarios, the engine replays the full event log from scratch and verifies that every intermediate state snapshot matches the original run.

The generated event log is written to `scenarios/demo.jsonl`.

## Project Structure
```
src/
├── types.rs         # Account, Position, Market structs
├── events.rs        # Event enum and serialization
├── state.rs         # State container and accessors
├── margin.rs        # Equity, margin, PnL — pure computation
├── risk.rs          # Pre-trade simulation and validation
├── liquidation.rs   # Detection and execution
├── engine.rs        # Event processing loop, live + replay
├── snapshot.rs      # State snapshots for determinism verification
├── lib.rs           # Public re-exports
└── main.rs          # Demo runner
```

## Design

See [DESIGN.md](DESIGN.md) for full architectural documentation including state model, margin formulas, liquidation mechanics, determinism guarantees, and documented simplifications.

## AI Usage

Claude (Anthropic) was used as a design collaborator throughout. All architectural decisions, formulas, and tradeoffs were discussed interactively before any code was written. Claude also assisted with Rust tests only. All output was reviewed, tested, and validated by me the author!

use crate::events::{Event, EventType};
use crate::liquidation;
use crate::risk::{self, apply_trade_to, TradeCheck};
use crate::snapshot::{self, Snapshot};
use crate::state::State;
use crate::types::{AccountId, Market};

use rust_decimal::Decimal;

/// Result of applying a single event.
enum ApplyResult {
    Ok,
    Rejected(String),
}

pub struct Engine {
    pub state: State,
    pub event_log: Vec<Event>,
    pub snapshots: Vec<Snapshot>,
    next_sequence: u64,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            state: State::new(),
            event_log: Vec::new(),
            snapshots: Vec::new(),
            next_sequence: 1,
        }
    }

    /// Register a market (configuration, not an event).
    pub fn add_market(&mut self, market: Market) {
        self.state.markets.insert(market.market_id.clone(), market);
    }

    /// Process an external event in live mode.
    /// Assigns a sequence number, applies it, snapshots, then scans for liquidations.
    pub fn process(&mut self, event_type: EventType) {
        let event = Event {
            sequence: self.next_sequence,
            event_type,
        };
        self.next_sequence += 1;
        self.event_log.push(event.clone());

        let result = self.apply_event(&event);

        // Handle rejections
        if let ApplyResult::Rejected(reason) = result {
            // Snapshot the unchanged state for the primary event
            self.snapshots
                .push(snapshot::capture(&self.state, event.sequence));

            let reject_event = match &event.event_type {
                EventType::TradeFill {
                    account_id,
                    market_id,
                    quantity,
                    price,
                } => Event {
                    sequence: self.next_sequence,
                    event_type: EventType::TradeRejected {
                        account_id: account_id.clone(),
                        market_id: market_id.clone(),
                        quantity: *quantity,
                        price: *price,
                        reason,
                    },
                },
                EventType::Withdraw {
                    account_id,
                    amount,
                } => Event {
                    sequence: self.next_sequence,
                    event_type: EventType::WithdrawalRejected {
                        account_id: account_id.clone(),
                        amount: *amount,
                        reason,
                    },
                },
                _ => unreachable!("Only trades and withdrawals can be rejected"),
            };
            self.next_sequence += 1;
            self.event_log.push(reject_event.clone());
            self.snapshots
                .push(snapshot::capture(&self.state, reject_event.sequence));
            return;
        }

        // Snapshot BEFORE liquidation scanning — this is the state after just this event
        self.snapshots
            .push(snapshot::capture(&self.state, event.sequence));

        // Determine which accounts need liquidation scanning based on event type
        let accounts_to_scan: Vec<AccountId> = match &event.event_type {
            EventType::TradeFill { account_id, .. } => vec![account_id.clone()],
            EventType::MarkPriceUpdate { market_id, .. } => {
                self.state.accounts_with_position_in(market_id)
            }
            EventType::FundingUpdate { market_id, .. } => {
                self.state.accounts_with_position_in(market_id)
            }
            _ => vec![],
        };

        // Execute liquidations and snapshot after each
        for account_id in accounts_to_scan {
            let liq_events = liquidation::check_and_liquidate(
                &mut self.state,
                &account_id,
                &mut self.next_sequence,
            );
            for liq_event in liq_events {
                self.event_log.push(liq_event.clone());
                self.snapshots
                    .push(snapshot::capture(&self.state, liq_event.sequence));
            }
        }
    }

    /// Apply a single event to state. Pure state mutation — no liquidation scanning,
    /// no event generation. Used identically in live and replay modes.
    fn apply_event(&mut self, event: &Event) -> ApplyResult {
        match &event.event_type {
            EventType::Deposit { account_id, amount } => {
                let account = self.state.get_or_create_account(account_id);
                account.collateral += amount;
                ApplyResult::Ok
            }

            EventType::Withdraw {
                account_id,
                amount,
            } => match risk::check_withdrawal(&self.state, account_id, *amount) {
                TradeCheck::Accepted => {
                    let account = self.state.accounts.get_mut(account_id).unwrap();
                    account.collateral -= amount;
                    ApplyResult::Ok
                }
                TradeCheck::Rejected(reason) => ApplyResult::Rejected(reason),
            },

            EventType::TradeFill {
                account_id,
                market_id,
                quantity,
                price,
            } => match risk::check_trade(&self.state, account_id, market_id, *quantity, *price) {
                TradeCheck::Accepted => {
                    let account = self.state.accounts.get_mut(account_id).unwrap();
                    apply_trade_to(
                        &mut account.collateral,
                        &mut account.positions,
                        market_id,
                        *quantity,
                        *price,
                    );
                    ApplyResult::Ok
                }
                TradeCheck::Rejected(reason) => ApplyResult::Rejected(reason),
            },

            EventType::MarkPriceUpdate { market_id, price } => {
                if let Some(market) = self.state.markets.get_mut(market_id) {
                    market.mark_price = *price;
                }
                ApplyResult::Ok
            }

            EventType::FundingUpdate {
                market_id,
                new_cumulative_index,
            } => {
                let old_index = self
                    .state
                    .markets
                    .get(market_id)
                    .map(|m| m.cumulative_funding_index)
                    .unwrap_or(Decimal::ZERO);

                if let Some(market) = self.state.markets.get_mut(market_id) {
                    market.cumulative_funding_index = *new_cumulative_index;
                }

                let affected = self.state.accounts_with_position_in(market_id);
                for account_id in &affected {
                    let account = self.state.accounts.get_mut(account_id).unwrap();
                    let last_index = account
                        .last_funding
                        .get(market_id)
                        .copied()
                        .unwrap_or(old_index);
                    let quantity = account.positions[market_id].quantity;
                    let funding_delta = (last_index - new_cumulative_index) * quantity;
                    account.collateral += funding_delta;
                    account
                        .last_funding
                        .insert(market_id.clone(), *new_cumulative_index);
                }

                ApplyResult::Ok
            }

            EventType::LiquidationFill {
                account_id,
                market_id,
                quantity,
                price,
            } => {
                // Direct application — no risk check
                let account = self.state.get_or_create_account(account_id);
                apply_trade_to(
                    &mut account.collateral,
                    &mut account.positions,
                    market_id,
                    *quantity,
                    *price,
                );
                ApplyResult::Ok
            }

            // Rejection events are informational — no state mutation
            EventType::TradeRejected { .. } | EventType::WithdrawalRejected { .. } => {
                ApplyResult::Ok
            }
        }
    }

    /// Replay a full event log from scratch. Returns the final state and snapshots.
    pub fn replay(event_log: &[Event], markets: Vec<Market>) -> (State, Vec<Snapshot>) {
        let mut engine = Engine::new();
        for market in markets {
            engine.add_market(market);
        }

        let mut snapshots = Vec::new();

        for event in event_log {
            let _ = engine.apply_event(event);
            snapshots.push(snapshot::capture(&engine.state, event.sequence));
        }

        (engine.state, snapshots)
    }
}

use rust_decimal::Decimal;

use crate::events::{Event, EventType};
use crate::margin;
use crate::risk::apply_trade_to;
use crate::state::State;
use crate::types::AccountId;

/// Scan an account for liquidation. If liquidatable, close positions
/// largest-notional-first and return the generated LiquidationFill events.
///
/// Determinism notes:
/// - Positions are stored in a BTreeMap, so iteration is deterministic.
/// - When notionals tie, we break ties by market_id (lexicographic) explicitly.
/// - Bankruptcy deficit is updated deterministically when positions are fully closed.
pub fn check_and_liquidate(
    state: &mut State,
    account_id: &AccountId,
    next_sequence: &mut u64,
) -> Vec<Event> {
    let mut events = Vec::new();

    loop {
        // Immutable read to decide whether we should liquidate.
        let account = match state.accounts.get(account_id) {
            Some(a) => a,
            None => return events,
        };

        // Nothing to liquidate if there are no positions.
        if account.positions.is_empty() {
            // If your engine defines bankruptcy as "no positions and negative collateral",
            // ensure deficit is set deterministically.
            // (Requires Account.bankruptcy_deficit field.)
            // drop(account);
            let _ = account;
            if let Some(acct) = state.accounts.get_mut(account_id) {
                acct.bankruptcy_deficit = if acct.collateral < Decimal::ZERO {
                    -acct.collateral
                } else {
                    Decimal::ZERO
                };
            }
            return events;
        }

        // Only proceed if account is liquidatable.
        if !margin::is_liquidatable(account, state) {
            return events;
        }

        // Select the position with the largest notional (abs(mark * qty)).
        // Tie-break: market_id lexicographically (canonical).
        let mut chosen_market: Option<String> = None;
        let mut chosen_notional: Option<Decimal> = None;
        let mut chosen_mark: Option<Decimal> = None;

        for (mid, pos) in &account.positions {
            let market = match state.markets.get(mid) {
                Some(m) => m,
                None => continue, // deterministic skip for malformed state
            };

            let notional = margin::position_notional(pos.quantity, market.mark_price);

            match (&chosen_market, &chosen_notional) {
                (None, None) => {
                    chosen_market = Some(mid.clone());
                    chosen_notional = Some(notional);
                    chosen_mark = Some(market.mark_price);
                }
                (Some(best_mid), Some(best_notional)) => {
                    if notional > *best_notional || (notional == *best_notional && mid < best_mid) {
                        chosen_market = Some(mid.clone());
                        chosen_notional = Some(notional);
                        chosen_mark = Some(market.mark_price);
                    }
                }
                _ => unreachable!("chosen_market and chosen_notional should be set together"),
            }
        }

        let market_id = match chosen_market {
            Some(m) => m,
            None => {
                // No positions with known markets; update deficit if desired and stop.
                // drop(account);
                let _ = account;
                if let Some(acct) = state.accounts.get_mut(account_id) {
                    acct.bankruptcy_deficit = if acct.collateral < Decimal::ZERO {
                        -acct.collateral
                    } else {
                        Decimal::ZERO
                    };
                }
                return events;
            }
        };

        let mark_price = chosen_mark.unwrap_or(Decimal::ZERO);

        // Close the entire position: fill quantity is the negative of current quantity.
        let close_qty = {
            let acct = state.accounts.get(account_id).unwrap();
            let pos = acct.positions.get(&market_id).unwrap();
            -pos.quantity
        };

        // Apply the liquidation close directly (no risk check).
        {
            let acct = state.accounts.get_mut(account_id).unwrap();
            apply_trade_to(
                &mut acct.collateral,
                &mut acct.positions,
                &market_id,
                close_qty,
                mark_price,
            );

            // If liquidation finishes the account (no positions left), update deficit deterministically.
            if acct.positions.is_empty() {
                acct.bankruptcy_deficit = if acct.collateral < Decimal::ZERO {
                    -acct.collateral
                } else {
                    Decimal::ZERO
                };
            } else {
                // Optional: keep it zero until fully closed, but ensure determinism.
                acct.bankruptcy_deficit = Decimal::ZERO;
            }
        }

        // Emit liquidation event.
        let event = Event {
            sequence: *next_sequence,
            event_type: EventType::LiquidationFill {
                account_id: account_id.clone(),
                market_id: market_id.clone(),
                quantity: close_qty,
                price: mark_price,
            },
        };
        *next_sequence += 1;
        events.push(event);

        // Loop back to recheck — there may be more positions to close.
    }
}

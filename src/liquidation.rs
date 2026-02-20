use rust_decimal::Decimal;

use crate::events::{Event, EventType};
use crate::margin;
use crate::risk::apply_trade_to;
use crate::state::State;
use crate::types::AccountId;

/// Scan an account for liquidation. If liquidatable, close positions
/// largest-notional-first and return the generated LiquidationFill events.
pub fn check_and_liquidate(
    state: &mut State,
    account_id: &AccountId,
    next_sequence: &mut u64,
) -> Vec<Event> {
    let mut events = Vec::new();

    loop {
        let account = match state.accounts.get(account_id) {
            Some(a) => a,
            None => return events,
        };

        if !margin::is_liquidatable(account, state) {
            return events;
        }

        if account.positions.is_empty() {
            return events;
        }

        // Find the position with the largest notional value
        let (market_id, close_qty, mark_price) = {
            let account = &state.accounts[account_id];
            let mut largest: Option<(String, Decimal, Decimal)> = None;
            for (mid, pos) in &account.positions {
                let market = &state.markets[mid];
                let notional = margin::position_notional(pos.quantity, market.mark_price);
                if largest.is_none() || notional > largest.as_ref().unwrap().1 {
                    largest = Some((mid.clone(), notional, market.mark_price));
                }
            }
            let (mid, _, mark) = largest.unwrap();
            let close_qty = -state.accounts[account_id].positions[&mid].quantity;
            (mid, close_qty, mark)
        };

        // Apply the liquidation close
        let account = state.accounts.get_mut(account_id).unwrap();
        apply_trade_to(
            &mut account.collateral,
            &mut account.positions,
            &market_id,
            close_qty,
            mark_price,
        );

        // Emit liquidation event
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

        // Loop back to recheck — there may be more positions to close
    }
}

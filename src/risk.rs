use rust_decimal::Decimal;
use std::collections::BTreeMap;

use crate::margin;
use crate::state::State;
use crate::types::{AccountId, MarketId, Position};
use rust_decimal::prelude::Signed;

/// Result of a pre-trade risk check.
pub enum TradeCheck {
    Accepted,
    Rejected(String),
}

/// Determines if a trade reduces the absolute position size without flipping.
fn is_risk_reducing(current_qty: Decimal, fill_qty: Decimal) -> bool {
    if current_qty.is_zero() {
        return false;
    }
    let new_qty = current_qty + fill_qty;
    // Reducing: same sign as before, but smaller absolute size
    // Also allow closing exactly to zero
    new_qty.is_zero()
        || (new_qty.signum() == current_qty.signum() && new_qty.abs() < current_qty.abs())
}

/// Simulate post-trade state and check initial margin.
pub fn check_trade(
    state: &State,
    account_id: &AccountId,
    market_id: &MarketId,
    fill_quantity: Decimal,
    fill_price: Decimal,
) -> TradeCheck {
    let account = match state.accounts.get(account_id) {
        Some(a) => a,
        None => {
            // New account — will be created on deposit; reject trades with no account
            return TradeCheck::Rejected("Account does not exist".to_string());
        }
    };

    let current_qty = account
        .positions
        .get(market_id)
        .map(|p| p.quantity)
        .unwrap_or(Decimal::ZERO);

    // Risk-reducing trades are always allowed
    if is_risk_reducing(current_qty, fill_quantity) {
        return TradeCheck::Accepted;
    }

    // Simulate post-trade state
    let (sim_collateral, sim_positions) =
        simulate_trade(account, market_id, fill_quantity, fill_price);

    // Compute simulated equity and IM
    let sim_unrealized: Decimal = sim_positions
        .iter()
        .map(|(mid, pos)| {
            let mark = state.markets[mid].mark_price;
            margin::position_unrealized_pnl(pos.quantity, pos.cost_basis, mark)
        })
        .sum();

    let sim_equity = sim_collateral + sim_unrealized;

    let sim_im: Decimal = sim_positions
        .iter()
        .map(|(mid, pos)| {
            let market = &state.markets[mid];
            margin::position_notional(pos.quantity, market.mark_price)
                * market.initial_margin_fraction
        })
        .sum();

    if sim_equity >= sim_im {
        TradeCheck::Accepted
    } else {
        TradeCheck::Rejected(format!(
            "Insufficient margin: equity {sim_equity} < IM required {sim_im}"
        ))
    }
}

/// Check whether a withdrawal is allowed.
pub fn check_withdrawal(state: &State, account_id: &AccountId, amount: Decimal) -> TradeCheck {
    let account = match state.accounts.get(account_id) {
        Some(a) => a,
        None => return TradeCheck::Rejected("Account does not exist".to_string()),
    };

    if amount > account.collateral {
        return TradeCheck::Rejected("Withdrawal exceeds collateral balance".to_string());
    }

    let eq = margin::equity(account, state);
    let im = margin::initial_margin_required(account, state);

    if eq - amount >= im {
        TradeCheck::Accepted
    } else {
        TradeCheck::Rejected(format!(
            "Withdrawal would violate IM: equity after {eq_after} < IM {im}",
            eq_after = eq - amount
        ))
    }
}

/// Simulate the effect of a trade on an account's collateral and positions.
/// Returns (simulated_collateral, simulated_positions).
fn simulate_trade(
    account: &crate::types::Account,
    market_id: &MarketId,
    fill_quantity: Decimal,
    fill_price: Decimal,
) -> (Decimal, std::collections::BTreeMap<MarketId, Position>) {
    let mut sim_collateral = account.collateral;
    let mut sim_positions = account.positions.clone();

    apply_trade_to(
        &mut sim_collateral,
        &mut sim_positions,
        market_id,
        fill_quantity,
        fill_price,
    );

    (sim_collateral, sim_positions)
}

/// Core trade application logic, shared between simulation and actual execution.
pub fn apply_trade_to(
    collateral: &mut Decimal,
    positions: &mut BTreeMap<MarketId, Position>,
    market_id: &MarketId,
    fill_quantity: Decimal,
    fill_price: Decimal,
) {
    let current_qty = positions
        .get(market_id)
        .map(|p| p.quantity)
        .unwrap_or(Decimal::ZERO);
    let current_cost = positions
        .get(market_id)
        .map(|p| p.cost_basis)
        .unwrap_or(Decimal::ZERO);

    let new_qty = current_qty + fill_quantity;

    if new_qty.is_zero() {
        // Full close
        // PnL = (value at close price) - (what we paid)
        // Long 10 @ cost 500k, close at 41k: (41000*10) - 500000 = -90000 ✓
        // Short 10 @ cost -500k, close at 41k: (41000*-10) - (-500000) = +90000 ✓
        let realized_pnl = (fill_price * current_qty) - current_cost;
        *collateral += realized_pnl;
        positions.remove(market_id);
    } else if current_qty.is_zero() {
        // Fresh open — no PnL, just record the position
        positions.insert(
            market_id.clone(),
            Position {
                market_id: market_id.clone(),
                quantity: fill_quantity,
                cost_basis: fill_quantity * fill_price,
            },
        );
    } else if current_qty.signum() == fill_quantity.signum() {
        // Increasing position — add to quantity and cost basis
        let pos = positions.get_mut(market_id).unwrap();
        pos.quantity = new_qty;
        pos.cost_basis += fill_quantity * fill_price;
    } else if new_qty.signum() == current_qty.signum() {
        // Partial close — realize PnL on the closed portion
        // close_ratio is negative (e.g., closing 3 of long 10: -3/10 = -0.3)
        let close_ratio = fill_quantity / current_qty;
        // Cost of closed portion = close_ratio * current_cost (negative of a portion)
        // Proceeds from close = fill_quantity * fill_price
        // Long 10 cost 500k, close 3 @ 48k: (-0.3 * 500k) - (-3 * 48k) = -150k + 144k = -6k ✓
        let realized_pnl = (close_ratio * current_cost) - (fill_quantity * fill_price);
        *collateral += realized_pnl;
        let pos = positions.get_mut(market_id).unwrap();
        pos.quantity = new_qty;
        pos.cost_basis = current_cost * (Decimal::ONE + close_ratio);
    } else {
        // Flip: close entire old position, open new in opposite direction
        // Same formula as full close for the PnL
        let close_pnl = (fill_price * current_qty) - current_cost;
        *collateral += close_pnl;
        positions.insert(
            market_id.clone(),
            Position {
                market_id: market_id.clone(),
                quantity: new_qty,
                cost_basis: new_qty * fill_price,
            },
        );
    }
}

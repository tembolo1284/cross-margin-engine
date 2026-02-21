use rust_decimal::prelude::Signed;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use crate::margin;
use crate::state::State;
use crate::types::{AccountId, MarketId, Position};

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

    // Reducing: same sign as before, but smaller absolute size (or closes exactly to zero).
    // Flips are NOT considered risk-reducing for the "always allow" rule.
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

    // Market must exist (configured out-of-band)
    if !state.markets.contains_key(market_id) {
        return TradeCheck::Rejected(format!("Unknown market_id: {market_id}"));
    }

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

    // Compute simulated equity and IM over the FULL portfolio (cross-margin)
    let mut sim_unrealized = Decimal::ZERO;
    let mut sim_im = Decimal::ZERO;

    for (mid, pos) in sim_positions.iter() {
        let market = match state.markets.get(mid) {
            Some(m) => m,
            None => {
                // Deterministic rejection instead of panicking
                return TradeCheck::Rejected(format!("Unknown market in portfolio: {mid}"));
            }
        };

        sim_unrealized += margin::position_unrealized_pnl(pos.quantity, pos.cost_basis, market.mark_price);
        sim_im += margin::position_notional(pos.quantity, market.mark_price) * market.initial_margin_fraction;
    }

    let sim_equity = sim_collateral + sim_unrealized;

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

    let eq_after = eq - amount;

    if eq_after >= im {
        TradeCheck::Accepted
    } else {
        TradeCheck::Rejected(format!(
            "Withdrawal would violate IM: equity after {eq_after} < IM {im}"
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
) -> (Decimal, BTreeMap<MarketId, Position>) {
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
    // Pull current position once (avoid double map lookups).
    let (current_qty, current_cost) = match positions.get(market_id) {
        Some(p) => (p.quantity, p.cost_basis),
        None => (Decimal::ZERO, Decimal::ZERO),
    };

    let new_qty = current_qty + fill_quantity;

    if new_qty.is_zero() {
        // Full close
        // PnL = (value at close price) - (what we paid)
        // Long 10 @ cost 500k, close at 41k: (41000*10) - 500000 = -90000 ✓
        // Short 10 @ cost -500k, close at 41k: (41000*-10) - (-500000) = +90000 ✓
        let realized_pnl = (fill_price * current_qty) - current_cost;
        *collateral += realized_pnl;
        positions.remove(market_id);
        return;
    }

    if current_qty.is_zero() {
        // Fresh open — no PnL, just record the position
        positions.insert(
            market_id.clone(),
            Position {
                market_id: market_id.clone(),
                quantity: fill_quantity,
                cost_basis: fill_quantity * fill_price,
            },
        );
        return;
    }

    // If fill is same direction as current position => increase
    if current_qty.signum() == fill_quantity.signum() {
        let pos = positions.get_mut(market_id).expect("position must exist");
        pos.quantity = new_qty;
        pos.cost_basis += fill_quantity * fill_price;
        return;
    }

    // Opposite direction: either partial close or flip
    if new_qty.signum() == current_qty.signum() {
        // Partial close: closes a fraction of the existing position (no flip).
        //
        // Use a sign-safe fraction-based realization:
        //   closed_fraction = |fill| / |current|
        //   closed_qty      = current_qty * closed_fraction   (same sign as current)
        //   closed_cost     = current_cost * closed_fraction  (same sign as current_cost)
        //   realized_pnl    = closed_qty*price - closed_cost
        //
        // This works for both longs and shorts without relying on negative ratios.
        let closed_fraction = fill_quantity.abs() / current_qty.abs();

        // Defensive: if precision causes slight overshoot, cap at 1.
        let closed_fraction = if closed_fraction > Decimal::ONE {
            Decimal::ONE
        } else {
            closed_fraction
        };

        let closed_qty = current_qty * closed_fraction;
        let closed_cost = current_cost * closed_fraction;

        let realized_pnl = (closed_qty * fill_price) - closed_cost;
        *collateral += realized_pnl;

        let pos = positions.get_mut(market_id).expect("position must exist");
        pos.quantity = new_qty;
        pos.cost_basis = current_cost - closed_cost;

        // If we somehow ended up at (very close to) zero, remove the position.
        // (Normally the earlier new_qty.is_zero() branch handles exact closure.)
        if pos.quantity.is_zero() {
            positions.remove(market_id);
        }

        return;
    }

    // Flip: close entire old position, open remainder in opposite direction
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

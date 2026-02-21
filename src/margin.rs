use rust_decimal::Decimal;

use crate::state::State;
use crate::types::Account;

/// Unrealized PnL for a single position.
pub fn position_unrealized_pnl(
    quantity: Decimal,
    cost_basis: Decimal,
    mark_price: Decimal,
) -> Decimal {
    mark_price * quantity - cost_basis
}

/// Notional value of a single position.
pub fn position_notional(quantity: Decimal, mark_price: Decimal) -> Decimal {
    (mark_price * quantity).abs()
}

/// Total unrealized PnL across all positions in an account.
///
/// Note: Markets are configured out-of-band. If a market is missing, we treat it as
/// having zero contribution (deterministic + avoids panics). In production you'd
/// likely hard-error.
pub fn total_unrealized_pnl(account: &Account, state: &State) -> Decimal {
    account
        .positions
        .values()
        .map(|pos| {
            let mark = state
                .markets
                .get(&pos.market_id)
                .map(|m| m.mark_price)
                .unwrap_or(Decimal::ZERO);
            position_unrealized_pnl(pos.quantity, pos.cost_basis, mark)
        })
        .sum()
}

/// Portfolio equity = collateral + total unrealized PnL.
pub fn equity(account: &Account, state: &State) -> Decimal {
    account.collateral + total_unrealized_pnl(account, state)
}

/// Initial margin required across all positions.
pub fn initial_margin_required(account: &Account, state: &State) -> Decimal {
    account
        .positions
        .values()
        .map(|pos| {
            let market = match state.markets.get(&pos.market_id) {
                Some(m) => m,
                None => return Decimal::ZERO,
            };
            position_notional(pos.quantity, market.mark_price) * market.initial_margin_fraction
        })
        .sum()
}

/// Maintenance margin required across all positions.
pub fn maintenance_margin_required(account: &Account, state: &State) -> Decimal {
    account
        .positions
        .values()
        .map(|pos| {
            let market = match state.markets.get(&pos.market_id) {
                Some(m) => m,
                None => return Decimal::ZERO,
            };
            position_notional(pos.quantity, market.mark_price) * market.maintenance_margin_fraction
        })
        .sum()
}

/// Returns true if the account is liquidatable under the engine's definition:
/// liquidatable when equity <= maintenance margin AND there is at least one open position.
pub fn is_liquidatable(account: &Account, state: &State) -> bool {
    if account.positions.is_empty() {
        return false;
    }
    let eq = equity(account, state);
    let mm = maintenance_margin_required(account, state);
    eq <= mm
}

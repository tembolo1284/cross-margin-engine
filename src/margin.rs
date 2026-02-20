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

/// Total unrealized PnL across all positions in an account.
pub fn total_unrealized_pnl(account: &Account, state: &State) -> Decimal {
    account
        .positions
        .values()
        .map(|pos| {
            let mark = state.markets[&pos.market_id].mark_price;
            position_unrealized_pnl(pos.quantity, pos.cost_basis, mark)
        })
        .sum()
}

/// Portfolio equity = collateral + total unrealized PnL.
pub fn equity(account: &Account, state: &State) -> Decimal {
    account.collateral + total_unrealized_pnl(account, state)
}

/// Notional value of a single position.
pub fn position_notional(quantity: Decimal, mark_price: Decimal) -> Decimal {
    (mark_price * quantity).abs()
}

/// Initial margin required across all positions.
pub fn initial_margin_required(account: &Account, state: &State) -> Decimal {
    account
        .positions
        .values()
        .map(|pos| {
            let market = &state.markets[&pos.market_id];
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
            let market = &state.markets[&pos.market_id];
            position_notional(pos.quantity, market.mark_price) * market.maintenance_margin_fraction
        })
        .sum()
}

/// Returns true if the account is liquidatable (equity <= maintenance margin).
pub fn is_liquidatable(account: &Account, state: &State) -> bool {
    let eq = equity(account, state);
    let mm = maintenance_margin_required(account, state);
    eq <= mm && mm > Decimal::ZERO
}

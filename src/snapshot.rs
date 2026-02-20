use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::margin;
use crate::state::State;
use crate::types::{AccountId, MarketId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub after_sequence: u64,
    pub accounts: BTreeMap<AccountId, AccountSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub collateral: Decimal,
    pub equity: Decimal,
    pub unrealized_pnl: Decimal,
    pub initial_margin_required: Decimal,
    pub maintenance_margin_required: Decimal,
    pub liquidatable: bool,
    pub positions: BTreeMap<MarketId, PositionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PositionSnapshot {
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub mark_price: Decimal,
    pub unrealized_pnl: Decimal,
    pub notional: Decimal,
}

pub fn capture(state: &State, after_sequence: u64) -> Snapshot {
    let mut accounts = BTreeMap::new();

    for (account_id, account) in &state.accounts {
        let eq = margin::equity(account, state);
        let upnl = margin::total_unrealized_pnl(account, state);
        let im = margin::initial_margin_required(account, state);
        let mm = margin::maintenance_margin_required(account, state);

        let mut positions = BTreeMap::new();
        for (market_id, pos) in &account.positions {
            let mark = state.markets[market_id].mark_price;
            positions.insert(
                market_id.clone(),
                PositionSnapshot {
                    quantity: pos.quantity,
                    cost_basis: pos.cost_basis,
                    mark_price: mark,
                    unrealized_pnl: margin::position_unrealized_pnl(
                        pos.quantity,
                        pos.cost_basis,
                        mark,
                    ),
                    notional: margin::position_notional(pos.quantity, mark),
                },
            );
        }

        accounts.insert(
            account_id.clone(),
            AccountSnapshot {
                collateral: account.collateral,
                equity: eq,
                unrealized_pnl: upnl,
                initial_margin_required: im,
                maintenance_margin_required: mm,
                liquidatable: margin::is_liquidatable(account, state),
                positions,
            },
        );
    }

    Snapshot {
        after_sequence,
        accounts,
    }
}

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type AccountId = String;
pub type MarketId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub market_id: MarketId,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub account_id: AccountId,
    pub collateral: Decimal,
    pub positions: BTreeMap<MarketId, Position>,
    pub last_funding: BTreeMap<MarketId, Decimal>,
}

impl Account {
    pub fn new(account_id: AccountId) -> Self {
        Self {
            account_id,
            collateral: Decimal::ZERO,
            positions: BTreeMap::new(),
            last_funding: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Market {
    pub market_id: MarketId,
    pub mark_price: Decimal,
    pub initial_margin_fraction: Decimal,
    pub maintenance_margin_fraction: Decimal,
    pub cumulative_funding_index: Decimal,
}

impl Market {
    pub fn new(
        market_id: MarketId,
        initial_margin_fraction: Decimal,
        maintenance_margin_fraction: Decimal,
    ) -> Self {
        Self {
            market_id,
            mark_price: Decimal::ZERO,
            initial_margin_fraction,
            maintenance_margin_fraction,
            cumulative_funding_index: Decimal::ZERO,
        }
    }
}

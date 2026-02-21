use rust_decimal::Decimal;
use rust_decimal::serde::str;
use serde::{Deserialize, Serialize};

use crate::types::{AccountId, MarketId};

/// A fully ordered, replayable event.
/// The event log is the sole source of truth for state reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub event_type: EventType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum EventType {
    Deposit {
        account_id: AccountId,
        #[serde(with = "str")]
        amount: Decimal,
    },
    Withdraw {
        account_id: AccountId,
        #[serde(with = "str")]
        amount: Decimal,
    },
    TradeFill {
        account_id: AccountId,
        market_id: MarketId,
        #[serde(with = "str")]
        quantity: Decimal,
        #[serde(with = "str")]
        price: Decimal,
    },
    MarkPriceUpdate {
        market_id: MarketId,
        #[serde(with = "str")]
        price: Decimal,
    },
    FundingUpdate {
        market_id: MarketId,
        #[serde(with = "str")]
        new_cumulative_index: Decimal,
    },
    LiquidationFill {
        account_id: AccountId,
        market_id: MarketId,
        #[serde(with = "str")]
        quantity: Decimal,
        #[serde(with = "str")]
        price: Decimal,
    },
    TradeRejected {
        account_id: AccountId,
        market_id: MarketId,
        #[serde(with = "str")]
        quantity: Decimal,
        #[serde(with = "str")]
        price: Decimal,
        reason: String,
    },
    WithdrawalRejected {
        account_id: AccountId,
        #[serde(with = "str")]
        amount: Decimal,
        reason: String,
    },
}

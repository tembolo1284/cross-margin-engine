use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{AccountId, MarketId};

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
        amount: Decimal,
    },
    Withdraw {
        account_id: AccountId,
        amount: Decimal,
    },
    TradeFill {
        account_id: AccountId,
        market_id: MarketId,
        quantity: Decimal,
        price: Decimal,
    },
    MarkPriceUpdate {
        market_id: MarketId,
        price: Decimal,
    },
    FundingUpdate {
        market_id: MarketId,
        new_cumulative_index: Decimal,
    },
    LiquidationFill {
        account_id: AccountId,
        market_id: MarketId,
        quantity: Decimal,
        price: Decimal,
    },
    TradeRejected {
        account_id: AccountId,
        market_id: MarketId,
        quantity: Decimal,
        price: Decimal,
        reason: String,
    },
    WithdrawalRejected {
        account_id: AccountId,
        amount: Decimal,
        reason: String,
    },
}

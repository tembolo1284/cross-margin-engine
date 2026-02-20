use std::collections::BTreeMap;

use crate::types::{Account, AccountId, Market, MarketId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub accounts: BTreeMap<AccountId, Account>,
    pub markets: BTreeMap<MarketId, Market>,
}

use serde::{Deserialize, Serialize};

impl State {
    pub fn new() -> Self {
        Self {
            accounts: BTreeMap::new(),
            markets: BTreeMap::new(),
        }
    }

    pub fn get_or_create_account(&mut self, account_id: &str) -> &mut Account {
        self.accounts
            .entry(account_id.to_string())
            .or_insert_with(|| Account::new(account_id.to_string()))
    }

    pub fn accounts_with_position_in(&self, market_id: &str) -> Vec<AccountId> {
        self.accounts
            .iter()
            .filter(|(_, acc)| acc.positions.contains_key(market_id))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

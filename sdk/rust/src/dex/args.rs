use crate::Pubkey;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct Rates {
    pub trade_bps: u16,
    pub protocol_share: u16,
    pub creator_share: u16,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct ConfigInit {
    pub treasury: Pubkey,
    pub rates: Rates,
    pub distributor: Option<Pubkey>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct PoolInit {
    pub amount_a: u64,
    pub amount_b: u64,
    pub minimum_lp: u64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct CurveInit {
    pub mode: u8,
    pub tax_bps: u16,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct SwapArgs {
    pub a_to_b: bool,
    pub amount: u64,
    pub limit: u64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct LiquidityArgs {
    pub amount: u64,
    // empty direct pools use both bounds as exact gross deposits; amount remains exact lp.
    pub bound_a: u64,
    pub bound_b: u64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct CollectArgs {
    pub protocol: bool,
    pub minimum_a: u64,
    pub minimum_b: u64,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct GlobalConfig {
    pub treasury: Pubkey,
    pub distributor: Pubkey,
    pub usdc_mint: Pubkey,
}

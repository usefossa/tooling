use super::{address, args::Rates};
use crate::{
    Pubkey, Result,
    accounts::{Decode, RawAccount, flag, key, n16, n64, state},
    ensure,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum PoolStatus {
    #[default]
    Curve = 0,
    Amm = 1,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum PoolMode {
    #[default]
    Standard = 0,
    Reward = 1,
    Community = 2,
    Model = 3,
}

fn n128(d: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(d[offset..offset + 16].try_into().unwrap())
}

pub const GLOBAL_LEN: usize = 144;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Global {
    #[serde(with = "crate::serde_pubkey")]
    pub council: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub treasury: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub distributor: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub usdc_mint: Pubkey,
}
impl Decode for Global {
    const MAGIC: &'static [u8; 8] = &[167, 232, 232, 177, 200, 108, 114, 127];
    fn decode(at: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self> {
        let d = state(account, program, Self::MAGIC, GLOBAL_LEN)?;
        ensure(
            d[136..144].iter().all(|&b| b == 0),
            "reserved account bytes",
        )?;
        let value = Self {
            council: key(d, 8),
            treasury: key(d, 40),
            distributor: key(d, 72),
            usdc_mint: key(d, 104),
        };
        ensure(address::global(program).0 == *at, "DEX global PDA")?;
        Ok(value)
    }
}

pub const CONFIG_LEN: usize = 272;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Config {
    #[serde(with = "crate::serde_pubkey")]
    pub council: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub treasury: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub quote_mint: Pubkey,
    pub trade_bps: u16,
    pub protocol_share: u16,
    pub creator_share: u16,
    #[serde(with = "crate::serde_pubkey")]
    pub distributor: Pubkey,
    pub enabled: bool,
    #[serde(with = "crate::serde_pubkey")]
    pub reference_pool: Pubkey,
    pub oracle_cumulative: u128,
    pub oracle_timestamp: u64,
    pub quote_per_usdc_x32: u128,
    pub quote_price_timestamp: u64,
}
impl Decode for Config {
    const MAGIC: &'static [u8; 8] = &[155, 12, 170, 224, 30, 250, 204, 130];
    fn decode(at: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self> {
        let d = state(account, program, Self::MAGIC, CONFIG_LEN)?;
        ensure(
            d[110..112].iter().all(|&b| b == 0)
                && d[225..257].iter().all(|&b| b == 0)
                && d[257..272].iter().all(|&b| b == 0),
            "reserved account bytes",
        )?;
        let value = Self {
            council: key(d, 8),
            treasury: key(d, 40),
            quote_mint: key(d, 72),
            trade_bps: n16(d, 104),
            protocol_share: n16(d, 106),
            creator_share: n16(d, 108),
            distributor: key(d, 112),
            enabled: flag(d, 144)?,
            reference_pool: key(d, 145),
            oracle_cumulative: n128(d, 177),
            oracle_timestamp: n64(d, 193),
            quote_per_usdc_x32: n128(d, 201),
            quote_price_timestamp: n64(d, 217),
        };
        ensure(
            address::config(program, &value.quote_mint).0 == *at,
            "DEX config PDA",
        )?;
        Rates {
            trade_bps: value.trade_bps,
            protocol_share: value.protocol_share,
            creator_share: value.creator_share,
        }
        .validate()?;
        Ok(value)
    }
}

pub const POOL_LEN: usize = 510;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Pool {
    #[serde(with = "crate::serde_pubkey")]
    pub config: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub creator: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub treasury: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint_a: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint_b: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub vault_a: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub vault_b: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub lp_mint: Pubkey,
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub protocol_a: u64,
    pub protocol_b: u64,
    pub creator_a: u64,
    pub creator_b: u64,
    pub virtual_a: u64,
    pub virtual_b: u64,
    pub target_b: u64,
    pub minimum_a: u64,
    pub locked_lp: u64,
    pub trade_bps: u16,
    pub protocol_share: u16,
    pub creator_share: u16,
    pub bump: u8,
    pub status: PoolStatus,
    pub mode: PoolMode,
    pub tax_bps: u16,
    #[serde(with = "crate::serde_pubkey")]
    pub distributor: Pubkey,
    pub tax_collected: u128,
    pub initial_base: u64,
    pub price_cumulative: u128,
    pub price_timestamp: u64,
}
impl Decode for Pool {
    const MAGIC: &'static [u8; 8] = &[241, 154, 109, 4, 17, 177, 109, 188];
    fn decode(at: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self> {
        let d = state(account, program, Self::MAGIC, POOL_LEN)?;
        ensure(
            d[352..360].iter().all(|&b| b == 0)
                && d[371..374].iter().all(|&b| b == 0)
                && d[454..486].iter().all(|&b| b == 0)
                && d[486..510].iter().all(|&b| b == 0),
            "reserved account bytes",
        )?;
        let value = Self {
            config: key(d, 8),
            creator: key(d, 40),
            treasury: key(d, 72),
            mint_a: key(d, 104),
            mint_b: key(d, 136),
            vault_a: key(d, 168),
            vault_b: key(d, 200),
            lp_mint: key(d, 232),
            reserve_a: n64(d, 264),
            reserve_b: n64(d, 272),
            protocol_a: n64(d, 280),
            protocol_b: n64(d, 288),
            creator_a: n64(d, 296),
            creator_b: n64(d, 304),
            virtual_a: n64(d, 312),
            virtual_b: n64(d, 320),
            target_b: n64(d, 328),
            minimum_a: n64(d, 336),
            locked_lp: n64(d, 344),
            trade_bps: n16(d, 360),
            protocol_share: n16(d, 362),
            creator_share: n16(d, 364),
            bump: d[366],
            status: match d[367] {
                0 => PoolStatus::Curve,
                1 => PoolStatus::Amm,
                _ => return Err(crate::Error::Invalid("pool status")),
            },
            mode: match d[368] {
                0 => PoolMode::Standard,
                1 => PoolMode::Reward,
                2 => PoolMode::Community,
                3 => PoolMode::Model,
                _ => return Err(crate::Error::Invalid("pool mode")),
            },
            tax_bps: n16(d, 369),
            distributor: key(d, 374),
            tax_collected: n128(d, 406),
            initial_base: n64(d, 422),
            price_cumulative: n128(d, 430),
            price_timestamp: n64(d, 446),
        };
        ensure(
            address::pool(program, &value.mint_a, &value.mint_b)?.0 == *at
                && address::authority(program, at).1 == value.bump
                && address::lp_mint(program, at).0 == value.lp_mint
                && address::config(program, &value.mint_b).0 == value.config
                && value.vault_a != value.vault_b,
            "pool PDA or account bindings",
        )?;
        Rates {
            trade_bps: value.trade_bps,
            protocol_share: value.protocol_share,
            creator_share: value.creator_share,
        }
        .validate()?;
        ensure(
            match value.mode {
                PoolMode::Reward | PoolMode::Community => matches!(value.tax_bps, 100 | 300),
                _ => value.tax_bps == 0,
            },
            "pool transfer tax",
        )?;
        Ok(value)
    }
}

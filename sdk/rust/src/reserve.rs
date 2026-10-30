use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{
    Instruction, Pubkey, Result,
    accounts::ModelGroup,
    address::{self, SYSTEM_PROGRAM, TOKEN_2022_PROGRAM, TOKEN_PROGRAM},
    ensure,
    instruction::{Protocol, r, s, w, ws},
};

pub const REDEMPTION_FEE_LAMPORTS: u64 = 25_000_000;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct FeeConfig {
    #[serde(with = "crate::serde_pubkey")]
    pub bank: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub creation_mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub creation_token_program: Pubkey,
    pub creation_net_price: u64,
    pub creation_enabled: bool,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ListFees {
    pub fee_bps: u16,
    pub protocol_share: u16,
}
impl ListFees {
    pub fn validate(self) -> Result<()> {
        ensure(
            (10..=300).contains(&self.fee_bps) && (5000..=10_000).contains(&self.protocol_share),
            "list fees",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub name: String,
    pub symbol: String,
    pub uri: String,
}
impl TokenMetadata {
    pub fn validate(&self) -> Result<()> {
        ensure(
            (1..=32).contains(&self.name.len())
                && (1..=10).contains(&self.symbol.len())
                && (1..=256).contains(&self.uri.len())
                && !self.name.bytes().any(|b| b.is_ascii_control())
                && !self.symbol.bytes().any(|b| b.is_ascii_control())
                && !self
                    .uri
                    .bytes()
                    .any(|b| b.is_ascii_control() || b.is_ascii_whitespace()),
            "token metadata",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct GroupInit {
    pub tax_bps: u16,
    pub maximum_tax: u64,
    pub metadata: TokenMetadata,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ReserveConfig {
    pub route_to_reserve: bool,
    pub tax_bps: u16,
    pub maximum_tax: u64,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct Redeem {
    pub amount: u64,
    pub minimum_output: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupKeys {
    #[serde(with = "crate::serde_pubkey")]
    pub group: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub backing: Pubkey,
}
impl GroupKeys {
    pub fn from_account(group: Pubkey, state: &ModelGroup) -> Self {
        Self {
            group,
            mint: state.fixed_token_mint,
            backing: state.backing_vault,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeModelGroup {
    #[serde(with = "crate::serde_pubkey")]
    pub council: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub payer: Pubkey,
    pub group_id: [u8; 32],
    #[serde(with = "crate::serde_pubkey")]
    pub backing: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub council_tokens: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub dex_program: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub dex_treasury: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub curve_quote_source: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub curve_base_vault: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub curve_quote_vault: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub curve_lp: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedemptionKeys {
    pub group: GroupKeys,
    #[serde(with = "crate::serde_pubkey")]
    pub holder: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub source: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub destination: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub council: Pubkey,
}

pub(crate) fn encode(value: &impl BorshSerialize) -> Result<Vec<u8>> {
    borsh::to_vec(value).map_err(Into::into)
}

impl Protocol {
    pub fn configure_fees(&self, council: Pubkey, config: FeeConfig) -> Result<Instruction> {
        ensure(config.bank != Pubkey::default(), "fee bank")?;
        let mut keys = vec![w(self.market), s(council)];
        if config.creation_enabled {
            ensure(
                config.creation_net_price > 0 && config.creation_mint != Pubkey::default(),
                "list creation payment",
            )?;
            crate::token::validate_program(config.creation_token_program)?;
            keys.extend([r(config.creation_mint), r(config.creation_token_program)]);
        }
        Ok(self.ix("configure_fees", encode(&config)?, keys))
    }
    pub fn configure_list_fees(
        &self,
        list: Pubkey,
        authority: Pubkey,
        fees: ListFees,
    ) -> Result<Instruction> {
        fees.validate()?;
        Ok(self.ix(
            "configure_list_fees",
            encode(&fees)?,
            vec![r(self.market), w(list), s(authority)],
        ))
    }
    pub fn initialize_model_group(
        &self,
        keys: InitializeModelGroup,
        config: &GroupInit,
    ) -> Result<Instruction> {
        config.metadata.validate()?;
        ensure(config.tax_bps <= 10_000, "model transfer tax")?;
        let group = address::model_group(&self.program, &self.market, &keys.group_id).0;
        let mint = address::reserve_mint(&self.program, &group).0;
        let dex = crate::dex::Dex {
            program: keys.dex_program,
        };
        let pool = dex.pool_address(mint, self.mint)?.0;
        Ok(self.ix(
            "initialize_model_group",
            encode(&(keys.group_id, config))?,
            vec![
                r(self.market),
                ws(keys.council),
                ws(keys.payer),
                w(group),
                w(mint),
                ws(keys.backing),
                w(keys.council_tokens),
                r(self.mint),
                r(TOKEN_PROGRAM),
                r(TOKEN_2022_PROGRAM),
                r(dex.program),
                r(dex.config_address(self.mint).0),
                w(pool),
                r(dex.authority_address(pool).0),
                w(keys.curve_quote_source),
                w(keys.curve_base_vault),
                w(keys.curve_quote_vault),
                w(dex.lp_mint_address(pool).0),
                ws(keys.curve_lp),
                r(dex.global_address().0),
                w(keys.dex_treasury),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn assign_model_group(
        &self,
        council: Pubkey,
        model: Pubkey,
        group: Option<Pubkey>,
    ) -> Instruction {
        self.ix(
            "assign_model_group",
            [],
            vec![
                r(self.market),
                s(council),
                w(model),
                r(group.unwrap_or(self.program)),
            ],
        )
    }
    pub fn update_group_metadata(
        &self,
        council: Pubkey,
        payer: Pubkey,
        group: GroupKeys,
        metadata: &TokenMetadata,
    ) -> Result<Instruction> {
        metadata.validate()?;
        Ok(self.ix(
            "update_group_metadata",
            encode(metadata)?,
            vec![
                r(self.market),
                s(council),
                ws(payer),
                r(group.group),
                w(group.mint),
                r(TOKEN_2022_PROGRAM),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn configure_reserve(
        &self,
        council: Pubkey,
        group: GroupKeys,
        config: ReserveConfig,
    ) -> Result<Instruction> {
        ensure(config.tax_bps <= 10_000, "model transfer tax")?;
        Ok(self.ix(
            "configure_reserve",
            encode(&config)?,
            vec![
                r(self.market),
                w(group.group),
                s(council),
                w(group.mint),
                r(TOKEN_2022_PROGRAM),
            ],
        ))
    }
    pub fn redeem_reserve(&self, keys: RedemptionKeys, redeem: Redeem) -> Result<Instruction> {
        ensure(redeem.amount > 0, "redemption amount")?;
        Ok(self.ix(
            "redeem_reserve",
            encode(&redeem)?,
            vec![
                r(self.market),
                r(keys.group.group),
                w(keys.group.mint),
                w(keys.source),
                ws(keys.holder),
                w(keys.group.backing),
                w(keys.destination),
                r(self.mint),
                w(keys.council),
                r(TOKEN_PROGRAM),
                r(TOKEN_2022_PROGRAM),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn collect_reserve_tax(&self, group: GroupKeys, destination: Pubkey) -> Instruction {
        self.ix(
            "collect_reserve_tax",
            [],
            vec![
                r(self.market),
                r(group.group),
                w(group.mint),
                w(destination),
                r(TOKEN_2022_PROGRAM),
            ],
        )
    }
    pub fn sweep_reserve(&self, group: GroupKeys, destination: Pubkey) -> Instruction {
        self.ix(
            "sweep_reserve",
            [],
            vec![
                r(self.market),
                r(group.group),
                r(group.mint),
                w(group.backing),
                w(destination),
                r(self.mint),
                r(TOKEN_PROGRAM),
            ],
        )
    }
    pub fn configure_dex(&self, council: Pubkey, dex: crate::dex::Dex) -> Instruction {
        self.ix(
            "configure_dex",
            [],
            vec![
                w(self.market),
                s(council),
                r(dex.program),
                r(dex.config_address(self.mint).0),
                r(dex.global_address().0),
            ],
        )
    }
    pub fn rotate_council(&self, current: Pubkey, next: Pubkey) -> Instruction {
        self.ix(
            "rotate_council",
            [],
            vec![w(self.market), s(current), s(next)],
        )
    }
}

pub fn redemption_output(backing_atoms: u64, supply_atoms: u64, burn_atoms: u64) -> Result<u64> {
    ensure(
        burn_atoms > 0 && burn_atoms <= supply_atoms,
        "redemption supply",
    )?;
    let output = crate::dex::quote::mul_div(backing_atoms, burn_atoms, supply_atoms, false)?;
    ensure(output > 0, "redemption dust")?;
    Ok(output)
}

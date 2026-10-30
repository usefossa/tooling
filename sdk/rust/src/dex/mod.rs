pub mod accounts;
pub mod address;
pub mod args;
pub mod quote;

use crate::{
    AccountMeta, Instruction, Pubkey, Result,
    address::{SYSTEM_PROGRAM, TOKEN_2022_PROGRAM, TOKEN_PROGRAM},
    ensure,
    instruction::{discriminator, r, s, w, ws},
    reserve::encode,
};
use args::*;

pub const ACTION_FEE_LAMPORTS: u64 = 25_000_000;
pub const ORACLE_WINDOW_SECONDS: u64 = 1_800;
pub const PUBLIC_RAISE_USDC_ATOMS: u64 = 8_500_000_000;
pub const PUBLIC_TOKEN_DECIMALS: u8 = 6;
pub const PUBLIC_SUPPLY_ATOMS: u64 = 1_000_000_000_000_000;
pub const PUBLIC_CURVE_SALE_ATOMS: u64 = 793_100_000_000_000;
pub const PUBLIC_LIQUIDITY_ATOMS: u64 = PUBLIC_SUPPLY_ATOMS - PUBLIC_CURVE_SALE_ATOMS;
pub const MODEL_TOKEN_DECIMALS: u8 = 6;
pub const MODEL_SUPPLY_ATOMS: u64 = 100_000_000_000_000;
pub const MODEL_TREASURY_ATOMS: u64 = MODEL_SUPPLY_ATOMS / 10;
pub const MODEL_CURVE_SUPPLY_ATOMS: u64 = MODEL_SUPPLY_ATOMS - MODEL_TREASURY_ATOMS;
pub const MODEL_RAISE_USDC_ATOMS: u64 = 50_000_000_000;
pub const MODEL_VIRTUAL_USDC_ATOMS: u64 = 120_710_678_119;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dex {
    pub program: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolKeys {
    pub pool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub lp_mint: Pubkey,
    pub token_a: Pubkey,
    pub token_b: Pubkey,
}
impl PoolKeys {
    pub fn from_account(
        pool: Pubkey,
        state: &accounts::Pool,
        token_a: Pubkey,
        token_b: Pubkey,
    ) -> Self {
        Self {
            pool,
            mint_a: state.mint_a,
            mint_b: state.mint_b,
            vault_a: state.vault_a,
            vault_b: state.vault_b,
            lp_mint: state.lp_mint,
            token_a,
            token_b,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchKeys {
    pub payer: Pubkey,
    pub treasury: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub token_a: Pubkey,
    pub token_b: Pubkey,
    pub source_a: Pubkey,
    pub source_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub user_lp: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeKeys {
    pub pool: PoolKeys,
    pub user: Pubkey,
    pub user_a: Pubkey,
    pub user_b: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiquidityKeys {
    pub trade: TradeKeys,
    pub treasury: Pubkey,
    pub user_lp: Pubkey,
}

impl Dex {
    pub fn global_address(&self) -> (Pubkey, u8) {
        address::global(&self.program)
    }
    pub fn config_address(&self, quote_mint: Pubkey) -> (Pubkey, u8) {
        address::config(&self.program, &quote_mint)
    }
    pub fn pool_address(&self, mint_a: Pubkey, mint_b: Pubkey) -> Result<(Pubkey, u8)> {
        address::pool(&self.program, &mint_a, &mint_b)
    }
    pub fn authority_address(&self, pool: Pubkey) -> (Pubkey, u8) {
        address::authority(&self.program, &pool)
    }
    pub fn lp_mint_address(&self, pool: Pubkey) -> (Pubkey, u8) {
        address::lp_mint(&self.program, &pool)
    }
    fn ix(
        &self,
        name: &str,
        args: &impl borsh::BorshSerialize,
        accounts: Vec<AccountMeta>,
    ) -> Result<Instruction> {
        let mut data = discriminator(name).to_vec();
        data.extend(encode(args)?);
        Ok(Instruction {
            program_id: self.program,
            accounts,
            data,
        })
    }
    fn launch(&self, k: LaunchKeys) -> Result<Vec<AccountMeta>> {
        crate::token::validate_program(k.token_a)?;
        crate::token::validate_program(k.token_b)?;
        let pool = self.pool_address(k.mint_a, k.mint_b)?.0;
        Ok(vec![
            ws(k.payer),
            r(self.global_address().0),
            w(k.treasury),
            r(self.config_address(k.mint_b).0),
            r(k.token_a),
            r(k.token_b),
            r(TOKEN_PROGRAM),
            w(k.mint_a),
            r(k.mint_b),
            w(pool),
            r(self.authority_address(pool).0),
            w(k.source_a),
            w(k.source_b),
            w(k.vault_a),
            w(k.vault_b),
            w(self.lp_mint_address(pool).0),
            ws(k.user_lp),
            r(SYSTEM_PROGRAM),
        ])
    }
    fn validate_pool(&self, k: PoolKeys) -> Result<()> {
        crate::token::validate_program(k.token_a)?;
        crate::token::validate_program(k.token_b)?;
        ensure(
            self.pool_address(k.mint_a, k.mint_b)?.0 == k.pool
                && self.lp_mint_address(k.pool).0 == k.lp_mint,
            "pool PDA or LP mint",
        )
    }
    fn trade(&self, k: TradeKeys) -> Result<Vec<AccountMeta>> {
        self.validate_pool(k.pool)?;
        let p = k.pool;
        Ok(vec![
            s(k.user),
            w(p.pool),
            r(self.authority_address(p.pool).0),
            r(p.token_a),
            r(p.token_b),
            r(p.mint_a),
            r(p.mint_b),
            w(k.user_a),
            w(k.user_b),
            w(p.vault_a),
            w(p.vault_b),
        ])
    }
    fn liquidity(&self, k: LiquidityKeys) -> Result<Vec<AccountMeta>> {
        self.validate_pool(k.trade.pool)?;
        let p = k.trade.pool;
        Ok(vec![
            ws(k.trade.user),
            r(self.global_address().0),
            w(k.treasury),
            r(SYSTEM_PROGRAM),
            w(p.pool),
            r(self.authority_address(p.pool).0),
            r(p.token_a),
            r(p.token_b),
            r(TOKEN_PROGRAM),
            r(p.mint_a),
            r(p.mint_b),
            w(k.trade.user_a),
            w(k.trade.user_b),
            w(p.vault_a),
            w(p.vault_b),
            w(p.lp_mint),
            w(k.user_lp),
        ])
    }
    pub fn initialize_global(
        &self,
        council: Pubkey,
        program_data: Pubkey,
        config: GlobalConfig,
    ) -> Result<Instruction> {
        config.validate(council)?;
        self.ix(
            "initialize_global",
            &config,
            vec![
                ws(council),
                w(self.global_address().0),
                r(self.program),
                r(program_data),
                r(SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn update_global(
        &self,
        council: Pubkey,
        new_council: Pubkey,
        config: GlobalConfig,
    ) -> Result<Instruction> {
        config.validate(new_council)?;
        self.ix(
            "update_global",
            &config,
            vec![w(self.global_address().0), s(council), s(new_council)],
        )
    }
    pub fn initialize_config(
        &self,
        council: Pubkey,
        mint: Pubkey,
        token: Pubkey,
        reference_pool: Option<Pubkey>,
        config: ConfigInit,
    ) -> Result<Instruction> {
        crate::token::validate_program(token)?;
        ensure(
            config.rates == Rates::default() && config.treasury != Pubkey::default(),
            "quote configuration",
        )?;
        self.ix(
            "initialize_config",
            &config,
            vec![
                r(self.global_address().0),
                ws(council),
                r(token),
                r(mint),
                r(reference_pool.unwrap_or(self.program)),
                w(self.config_address(mint).0),
                r(SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn update_config(
        &self,
        council: Pubkey,
        mint: Pubkey,
        token: Pubkey,
        enabled: bool,
    ) -> Result<Instruction> {
        crate::token::validate_program(token)?;
        self.ix(
            "update_config",
            &enabled,
            vec![
                r(self.global_address().0),
                s(council),
                r(token),
                r(mint),
                w(self.config_address(mint).0),
            ],
        )
    }
    pub fn initialize_pool(
        &self,
        keys: LaunchKeys,
        council: Option<Pubkey>,
        config: PoolInit,
    ) -> Result<Instruction> {
        ensure(config.amount_a > 0 && config.amount_b > 0, "pool deposits")?;
        let mut accounts = self.launch(keys)?;
        accounts.push(council.map_or_else(|| r(self.program), s));
        self.ix("initialize_pool", &config, accounts)
    }
    pub fn initialize_curve(&self, keys: LaunchKeys, config: CurveInit) -> Result<Instruction> {
        config.validate()?;
        self.ix("initialize_curve", &config, self.launch(keys)?)
    }
    pub fn initialize_model_curve(&self, keys: LaunchKeys, council: Pubkey) -> Result<Instruction> {
        let mut accounts = self.launch(keys)?;
        accounts.push(s(council));
        self.ix("initialize_model_curve", &(), accounts)
    }
    pub fn swap(&self, keys: TradeKeys, args: SwapArgs) -> Result<Instruction> {
        ensure(args.amount > 0, "swap amount")?;
        self.ix("swap", &args, self.trade(keys)?)
    }
    pub fn swap_exact_out(&self, keys: TradeKeys, args: SwapArgs) -> Result<Instruction> {
        ensure(args.amount > 0, "swap amount")?;
        self.ix("swap_exact_out", &args, self.trade(keys)?)
    }
    pub fn graduate(&self, p: PoolKeys, operator: Pubkey) -> Result<Instruction> {
        self.validate_pool(p)?;
        self.ix(
            "graduate",
            &(),
            vec![
                w(p.pool),
                r(self.authority_address(p.pool).0),
                r(p.token_a),
                r(p.token_b),
                r(TOKEN_PROGRAM),
                w(p.mint_a),
                r(p.mint_b),
                w(p.vault_a),
                w(p.vault_b),
                r(p.lp_mint),
                s(operator),
            ],
        )
    }
    pub fn add_liquidity(&self, keys: LiquidityKeys, args: LiquidityArgs) -> Result<Instruction> {
        ensure(args.amount > 0, "liquidity amount")?;
        self.ix("add_liquidity", &args, self.liquidity(keys)?)
    }
    pub fn remove_liquidity(
        &self,
        keys: LiquidityKeys,
        args: LiquidityArgs,
    ) -> Result<Instruction> {
        ensure(args.amount > 0, "liquidity amount")?;
        self.ix("remove_liquidity", &args, self.liquidity(keys)?)
    }
    pub fn collect_fees(&self, keys: TradeKeys, args: CollectArgs) -> Result<Instruction> {
        self.ix("collect_fees", &args, self.trade(keys)?)
    }
    pub fn collect_transfer_tax(
        &self,
        pool: Pubkey,
        mint: Pubkey,
        destination: Pubkey,
        sources: &[Pubkey],
    ) -> Result<Instruction> {
        ensure(
            sources.len() <= 16
                && sources
                    .iter()
                    .enumerate()
                    .all(|(i, key)| *key != destination && !sources[..i].contains(key)),
            "tax source accounts",
        )?;
        let mut accounts = vec![
            w(pool),
            r(self.authority_address(pool).0),
            r(TOKEN_2022_PROGRAM),
            w(mint),
            w(destination),
        ];
        accounts.extend(sources.iter().copied().map(w));
        self.ix("collect_transfer_tax", &(), accounts)
    }
    pub fn refresh_quote(&self, config: Pubkey, reference_pool: Pubkey) -> Result<Instruction> {
        self.ix("refresh_quote", &(), vec![w(config), r(reference_pool)])
    }
    pub fn update_recipients(&self, council: Pubkey, pool: Pubkey) -> Result<Instruction> {
        self.ix(
            "update_recipients",
            &(),
            vec![r(self.global_address().0), s(council), w(pool)],
        )
    }
}
impl Default for Rates {
    fn default() -> Self {
        Self {
            trade_bps: 125,
            protocol_share: 10_000,
            creator_share: 0,
        }
    }
}
impl Rates {
    pub fn validate(self) -> Result<()> {
        ensure(
            self.trade_bps < 10_000
                && u32::from(self.protocol_share) + u32::from(self.creator_share) <= 10_000,
            "DEX fee rates",
        )
    }
}
impl CurveInit {
    pub fn validate(self) -> Result<()> {
        ensure(
            match self.mode {
                0 => self.tax_bps == 0,
                1 | 2 => matches!(self.tax_bps, 100 | 300),
                _ => false,
            },
            "public curve mode or tax",
        )
    }
}
impl GlobalConfig {
    pub fn validate(self, council: Pubkey) -> Result<()> {
        ensure(
            self.treasury != Pubkey::default()
                && self.distributor != Pubkey::default()
                && self.usdc_mint != Pubkey::default()
                && self.treasury != council,
            "DEX global configuration",
        )
    }
}

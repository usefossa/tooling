use crate::{
    Error, Instruction, Pubkey, Result,
    accounts::RawAccount,
    address::{NATIVE_MINT, TOKEN_2022_PROGRAM, TOKEN_PROGRAM},
    ensure,
};
use serde::{Deserialize, Serialize};
use solana_program_pack::Pack;
use spl_token_2022_interface::{
    extension::{
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
        transfer_fee::{TransferFeeAmount, TransferFeeConfig},
    },
    state::{Account, AccountState, Mint},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferFee {
    pub basis_points: u16,
    pub maximum_fee: u64,
}
impl TransferFee {
    pub fn fee(self, amount: u64) -> Result<u64> {
        ensure(self.basis_points <= 10_000, "transfer fee basis points")?;
        Ok(
            crate::dex::quote::mul_div(amount, u64::from(self.basis_points), 10_000, true)?
                .min(self.maximum_fee),
        )
    }
    pub fn net(self, amount: u64) -> Result<u64> {
        amount
            .checked_sub(self.fee(amount)?)
            .ok_or(Error::Arithmetic)
    }
    pub fn gross(self, net: u64) -> Result<u64> {
        ensure(self.basis_points <= 10_000, "transfer fee basis points")?;
        if net == 0 || self.basis_points == 0 || self.maximum_fee == 0 {
            return Ok(net);
        }
        let capped = net.checked_add(self.maximum_fee);
        let uncapped = if self.basis_points == 10_000 {
            None
        } else {
            crate::dex::quote::mul_div(net, 10_000, 10_000 - u64::from(self.basis_points), true)
                .ok()
        };
        match (capped, uncapped) {
            (Some(a), Some(b)) => Ok(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Ok(a),
            _ => Err(Error::Arithmetic),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeSchedule {
    pub epoch: u64,
    pub fee: TransferFee,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenMint {
    #[serde(with = "crate::serde_pubkey")]
    pub program: Pubkey,
    pub supply: u64,
    pub decimals: u8,
    #[serde(serialize_with = "optional_key")]
    pub mint_authority: Option<Pubkey>,
    #[serde(serialize_with = "optional_key")]
    pub freeze_authority: Option<Pubkey>,
    #[serde(serialize_with = "optional_key")]
    pub transfer_fee_authority: Option<Pubkey>,
    #[serde(serialize_with = "optional_key")]
    pub withdraw_withheld_authority: Option<Pubkey>,
    pub withheld_amount: u64,
    pub older_fee: Option<FeeSchedule>,
    pub newer_fee: Option<FeeSchedule>,
    pub metadata: Option<crate::reserve::TokenMetadata>,
}
fn decode_error(_: impl std::fmt::Display) -> Error {
    Error::Invalid("SPL token state")
}
pub fn validate_program(program: Pubkey) -> Result<()> {
    ensure(
        matches!(program, TOKEN_PROGRAM | TOKEN_2022_PROGRAM),
        "SPL token program",
    )
}
impl TokenMint {
    pub fn decode(account: &RawAccount) -> Result<Self> {
        Self::decode_for(account, false)
    }
    pub fn decode_payment(account: &RawAccount) -> Result<Self> {
        Self::decode_for(account, true)
    }
    fn decode_for(account: &RawAccount, allow_close_authority: bool) -> Result<Self> {
        validate_program(account.owner)?;
        ensure(!account.executable, "nonexecutable mint")?;
        if account.owner == TOKEN_PROGRAM {
            ensure(account.data.len() == Mint::LEN, "legacy mint length")?;
        }
        let state = StateWithExtensions::<Mint>::unpack(&account.data).map_err(decode_error)?;
        let extensions = state.get_extension_types().map_err(decode_error)?;
        ensure(
            extensions.iter().all(|extension| {
                (allow_close_authority && *extension == ExtensionType::MintCloseAuthority)
                    || matches!(
                        extension,
                        ExtensionType::TransferFeeConfig
                            | ExtensionType::MetadataPointer
                            | ExtensionType::TokenMetadata
                    )
            }),
            "supported mint extensions",
        )?;
        let mut mint = Self {
            program: account.owner,
            supply: state.base.supply,
            decimals: state.base.decimals,
            mint_authority: state
                .base
                .mint_authority
                .map(|key| Pubkey::new_from_array(key.to_bytes()))
                .into(),
            freeze_authority: state
                .base
                .freeze_authority
                .map(|key| Pubkey::new_from_array(key.to_bytes()))
                .into(),
            transfer_fee_authority: None,
            withdraw_withheld_authority: None,
            withheld_amount: 0,
            older_fee: None,
            newer_fee: None,
            metadata: None,
        };
        if extensions.contains(&ExtensionType::TransferFeeConfig) {
            let config = state
                .get_extension::<TransferFeeConfig>()
                .map_err(decode_error)?;
            let optional =
                |bytes: [u8; 32]| (bytes != [0; 32]).then_some(Pubkey::new_from_array(bytes));
            mint.transfer_fee_authority =
                optional(config.transfer_fee_config_authority.0.to_bytes());
            mint.withdraw_withheld_authority =
                optional(config.withdraw_withheld_authority.0.to_bytes());
            mint.withheld_amount = config.withheld_amount.into();
            let schedule =
                |fee: &spl_token_2022_interface::extension::transfer_fee::TransferFee| {
                    FeeSchedule {
                        epoch: fee.epoch.into(),
                        fee: TransferFee {
                            basis_points: fee.transfer_fee_basis_points.into(),
                            maximum_fee: fee.maximum_fee.into(),
                        },
                    }
                };
            mint.older_fee = Some(schedule(&config.older_transfer_fee));
            mint.newer_fee = Some(schedule(&config.newer_transfer_fee));
            ensure(
                mint.older_fee.unwrap().fee.basis_points <= 10_000
                    && mint.newer_fee.unwrap().fee.basis_points <= 10_000,
                "transfer fee basis points",
            )?;
        }
        if extensions.contains(&ExtensionType::TokenMetadata) {
            let metadata = state
                .get_variable_len_extension::<spl_token_metadata_interface::state::TokenMetadata>()
                .map_err(decode_error)?;
            mint.metadata = Some(crate::reserve::TokenMetadata {
                name: metadata.name,
                symbol: metadata.symbol,
                uri: metadata.uri,
            });
        }
        Ok(mint)
    }
    pub fn transfer_fee(&self, epoch: u64) -> TransferFee {
        match (self.older_fee, self.newer_fee) {
            (Some(older), Some(newer)) => {
                if epoch >= newer.epoch {
                    newer.fee
                } else {
                    older.fee
                }
            }
            _ => TransferFee::default(),
        }
    }
    pub fn validate_dex_base(&self) -> Result<()> {
        ensure(
            self.mint_authority.is_none() && self.freeze_authority.is_none(),
            "immutable DEX base mint",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenAccount {
    #[serde(with = "crate::serde_pubkey")]
    pub program: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    pub amount: u64,
    pub frozen: bool,
    #[serde(serialize_with = "optional_key")]
    pub delegate: Option<Pubkey>,
    #[serde(serialize_with = "optional_key")]
    pub close_authority: Option<Pubkey>,
    pub native_reserve: Option<u64>,
    pub withheld_amount: u64,
}
impl TokenAccount {
    pub fn decode(account: &RawAccount) -> Result<Self> {
        validate_program(account.owner)?;
        ensure(!account.executable, "nonexecutable token account")?;
        if account.owner == TOKEN_PROGRAM {
            ensure(
                account.data.len() == Account::LEN,
                "legacy token account length",
            )?;
        }
        let state = StateWithExtensions::<Account>::unpack(&account.data).map_err(decode_error)?;
        let extensions = state.get_extension_types().map_err(decode_error)?;
        ensure(
            extensions.iter().all(|extension| {
                matches!(
                    extension,
                    ExtensionType::TransferFeeAmount | ExtensionType::ImmutableOwner
                )
            }),
            "supported token account extensions",
        )?;
        let withheld_amount = if extensions.contains(&ExtensionType::TransferFeeAmount) {
            state
                .get_extension::<TransferFeeAmount>()
                .map_err(decode_error)?
                .withheld_amount
                .into()
        } else {
            0
        };
        Ok(Self {
            program: account.owner,
            mint: Pubkey::new_from_array(state.base.mint.to_bytes()),
            owner: Pubkey::new_from_array(state.base.owner.to_bytes()),
            amount: state.base.amount,
            frozen: state.base.state == AccountState::Frozen,
            delegate: state
                .base
                .delegate
                .map(|key| Pubkey::new_from_array(key.to_bytes()))
                .into(),
            close_authority: state
                .base
                .close_authority
                .map(|key| Pubkey::new_from_array(key.to_bytes()))
                .into(),
            native_reserve: state.base.is_native.into(),
            withheld_amount,
        })
    }
    pub fn validate(&self, mint: Pubkey, authority: Pubkey, vault: bool) -> Result<()> {
        ensure(
            self.mint == mint
                && self.owner == authority
                && !self.frozen
                && (!vault || (self.delegate.is_none() && self.close_authority.is_none())),
            "token account bindings",
        )
    }
}

pub fn create_associated_token(
    payer: Pubkey,
    owner: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
) -> Result<Instruction> {
    validate_program(token_program)?;
    let mut ix = crate::instruction::create_associated_token(payer, owner, mint);
    ix.accounts[1].pubkey =
        crate::address::associated_token_with_program(&mint, &owner, &token_program);
    ix.accounts[5].pubkey = token_program;
    Ok(ix)
}
pub fn initialize_account(
    account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    program: Pubkey,
) -> Result<Instruction> {
    validate_program(program)?;
    spl_token_2022_interface::instruction::initialize_account3(
        &program.to_bytes().into(),
        &account.to_bytes().into(),
        &mint.to_bytes().into(),
        &owner.to_bytes().into(),
    )
    .map_err(decode_error)
}
pub fn create_wrapped_native(
    payer: Pubkey,
    account: Pubkey,
    owner: Pubkey,
    lamports: u64,
    rent_lamports: u64,
) -> Result<Vec<Instruction>> {
    Ok(vec![
        crate::instruction::create_account(
            payer,
            account,
            TOKEN_PROGRAM,
            lamports
                .checked_add(rent_lamports)
                .ok_or(Error::Arithmetic)?,
            Account::LEN,
        )?,
        initialize_account(account, NATIVE_MINT, owner, TOKEN_PROGRAM)?,
    ])
}
pub fn sync_native(account: Pubkey) -> Result<Instruction> {
    spl_token_interface::instruction::sync_native(
        &TOKEN_PROGRAM.to_bytes().into(),
        &account.to_bytes().into(),
    )
    .map_err(decode_error)
}
pub fn close_account(
    account: Pubkey,
    destination: Pubkey,
    owner: Pubkey,
    program: Pubkey,
) -> Result<Instruction> {
    validate_program(program)?;
    spl_token_2022_interface::instruction::close_account(
        &program.to_bytes().into(),
        &account.to_bytes().into(),
        &destination.to_bytes().into(),
        &owner.to_bytes().into(),
        &[],
    )
    .map_err(decode_error)
}
pub fn harvest_transfer_tax(mint: Pubkey, accounts: &[Pubkey]) -> Result<Instruction> {
    let keys: Vec<_> = accounts.iter().map(|key| key.to_bytes().into()).collect();
    spl_token_2022_interface::extension::transfer_fee::instruction::harvest_withheld_tokens_to_mint(
        &TOKEN_2022_PROGRAM.to_bytes().into(),
        &mint.to_bytes().into(),
        &keys.iter().collect::<Vec<_>>(),
    )
    .map_err(decode_error)
}
pub fn transfer_fee_account_len() -> Result<usize> {
    ExtensionType::try_calculate_account_len::<Account>(&[ExtensionType::TransferFeeAmount])
        .map_err(decode_error)
}

pub fn validate_uninitialized_fee_account(account: &RawAccount) -> Result<()> {
    ensure(
        account.owner == TOKEN_2022_PROGRAM
            && !account.executable
            && account.data.len() == transfer_fee_account_len()?
            && account.data.iter().all(|&byte| byte == 0),
        "uninitialized transfer fee token account",
    )
}

pub fn curve_mint_len(config: crate::dex::args::CurveInit) -> Result<usize> {
    config.validate()?;
    let extensions = if config.tax_bps == 0 {
        &[][..]
    } else {
        &[ExtensionType::TransferFeeConfig][..]
    };
    ExtensionType::try_calculate_account_len::<Mint>(extensions).map_err(decode_error)
}
pub fn create_curve_mint(
    payer: Pubkey,
    mint: Pubkey,
    authority: Pubkey,
    program: Pubkey,
    config: crate::dex::args::CurveInit,
    rent_lamports: u64,
) -> Result<Vec<Instruction>> {
    validate_program(program)?;
    config.validate()?;
    ensure(
        config.tax_bps == 0 || program == TOKEN_2022_PROGRAM,
        "taxed curve token program",
    )?;
    let mut instructions = vec![crate::instruction::create_account(
        payer,
        mint,
        program,
        rent_lamports,
        curve_mint_len(config)?,
    )?];
    if config.tax_bps > 0 {
        instructions.push(spl_token_2022_interface::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &program.to_bytes().into(), &mint.to_bytes().into(), None, Some(&authority.to_bytes().into()), config.tax_bps, u64::MAX).map_err(decode_error)?);
    }
    instructions.push(
        spl_token_2022_interface::instruction::initialize_mint2(
            &program.to_bytes().into(),
            &mint.to_bytes().into(),
            &authority.to_bytes().into(),
            None,
            crate::dex::PUBLIC_TOKEN_DECIMALS,
        )
        .map_err(decode_error)?,
    );
    Ok(instructions)
}

fn optional_key<S: serde::Serializer>(
    key: &Option<Pubkey>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    key.map(|key| key.to_string()).serialize(serializer)
}

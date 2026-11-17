use std::sync::atomic::Ordering;

use fossa_sdk::{
    accounts::{Decode, RawAccount},
    dex::{
        Dex, PoolKeys, TradeKeys,
        accounts::{Pool, PoolStatus},
        args::SwapArgs,
        quote,
    },
    token::{TokenAccount, TokenMint},
};
use jupiter_amm_interface::{
    AccountProvider, Amm, AmmContext, AmmError, AmmLabel, ClockRef, KeyedAccount, Quote,
    QuoteParams, SingleProgramAmm, Swap, SwapAndAccountMetas, SwapMode, SwapParams,
    single_program_amm,
};
use rust_decimal::Decimal;
use solana_account::ReadableAccount;
use solana_pubkey::{Pubkey, pubkey};

pub const PROGRAM_ID: Pubkey = pubkey!("dexXxx8t1AZqsAZ3dLdMcfvF3mMeEGBdbEPjAYXy5n9");
type Result<T> = std::result::Result<T, AmmError>;

fn sdk<T>(result: fossa_sdk::Result<T>) -> Result<T> {
    result.map_err(|error| AmmError::from(error.to_string()))
}

fn raw(account: &impl ReadableAccount) -> RawAccount {
    RawAccount {
        owner: *account.owner(),
        lamports: account.lamports(),
        executable: account.executable(),
        data: account.data().to_vec(),
    }
}

#[derive(Clone)]
pub struct FossaAmm {
    key: Pubkey,
    pool: Pool,
    clock: ClockRef,
    mints: Option<[TokenMint; 2]>,
}

single_program_amm!(FossaAmm, PROGRAM_ID, "Fossa");

impl FossaAmm {
    fn direction(&self, input: Pubkey, output: Pubkey) -> Result<bool> {
        if input == self.pool.mint_a && output == self.pool.mint_b {
            Ok(true)
        } else if input == self.pool.mint_b && output == self.pool.mint_a {
            Ok(false)
        } else {
            Err("Swap mints do not match the pool".into())
        }
    }

    fn ready(&self) -> Result<&[TokenMint; 2]> {
        let mints = self.mints.as_ref().ok_or("Pool accounts need updating")?;
        let now = self.clock.unix_timestamp.load(Ordering::Relaxed);
        if now < 0 || (now as u64) < self.pool.price_timestamp {
            return Err("Pool timestamp is ahead of the quote clock".into());
        }
        Ok(mints)
    }
}

impl Amm for FossaAmm {
    fn from_keyed_account(account: &KeyedAccount, context: &AmmContext) -> Result<Self> {
        Ok(Self {
            key: account.key,
            pool: sdk(Pool::decode(
                &account.key,
                &raw(&account.account),
                &PROGRAM_ID,
            ))?,
            clock: context.clock_ref.clone(),
            mints: None,
        })
    }

    fn label(&self) -> AmmLabel {
        Self::LABEL
    }

    fn program_id(&self) -> Pubkey {
        PROGRAM_ID
    }

    fn key(&self) -> Pubkey {
        self.key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![self.pool.mint_a, self.pool.mint_b]
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        vec![
            self.key,
            self.pool.mint_a,
            self.pool.mint_b,
            self.pool.vault_a,
            self.pool.vault_b,
        ]
    }

    fn update(&mut self, accounts: impl AccountProvider) -> Result<()> {
        self.mints = None;
        let pool = sdk(Pool::decode(
            &self.key,
            &raw(&accounts.try_get(&self.key)?),
            &PROGRAM_ID,
        ))?;
        if pool.mint_a != self.pool.mint_a
            || pool.mint_b != self.pool.mint_b
            || pool.vault_a != self.pool.vault_a
            || pool.vault_b != self.pool.vault_b
        {
            return Err("Pool account bindings changed".into());
        }
        let mints = [
            sdk(TokenMint::decode(&raw(&accounts.try_get(&pool.mint_a)?)))?,
            sdk(TokenMint::decode(&raw(&accounts.try_get(&pool.mint_b)?)))?,
        ];
        sdk(mints[0].validate_dex_base())?;
        let authority = Dex {
            program: PROGRAM_ID,
        }
        .authority_address(self.key)
        .0;
        for (mint, address, vault, reserve, protocol, creator) in [
            (
                &mints[0],
                pool.mint_a,
                pool.vault_a,
                pool.reserve_a,
                pool.protocol_a,
                pool.creator_a,
            ),
            (
                &mints[1],
                pool.mint_b,
                pool.vault_b,
                pool.reserve_b,
                pool.protocol_b,
                pool.creator_b,
            ),
        ] {
            let vault = sdk(TokenAccount::decode(&raw(&accounts.try_get(&vault)?)))?;
            sdk(vault.validate(address, authority, true))?;
            let balance = reserve
                .checked_add(protocol)
                .and_then(|n| n.checked_add(creator));
            if vault.program != mint.program || balance.is_none_or(|n| vault.amount < n) {
                return Err("Pool vault balance or token program is invalid".into());
            }
        }
        self.pool = pool;
        self.mints = Some(mints);
        Ok(())
    }

    fn quote(&self, params: &QuoteParams) -> Result<Quote> {
        let mints = self.ready()?;
        let epoch = self.clock.epoch.load(Ordering::Relaxed);
        let fees = [mints[0].transfer_fee(epoch), mints[1].transfer_fee(epoch)];
        let args = SwapArgs {
            a_to_b: self.direction(params.input_mint, params.output_mint)?,
            amount: params.amount,
            limit: if params.swap_mode == SwapMode::ExactIn {
                0
            } else {
                u64::MAX
            },
        };
        let quote = sdk(match params.swap_mode {
            SwapMode::ExactIn => quote::swap_exact_input(&self.pool, fees, args),
            SwapMode::ExactOut => quote::swap_exact_output(&self.pool, fees, args),
        })?;
        if quote.unused_input != 0 || quote.net_output == 0 {
            return Err("Partial fills and zero-output swaps are not routable".into());
        }
        let fee = quote
            .protocol_fee
            .checked_add(quote.creator_fee)
            .ok_or("Trade fee overflow")?;
        Ok(Quote {
            in_amount: quote.gross_input,
            out_amount: quote.net_output,
            fee_amount: fee,
            fee_mint: params.input_mint,
            fee_pct: Decimal::from(fee) / Decimal::from(quote.received_input),
        })
    }

    fn get_swap_and_account_metas(&self, params: &SwapParams) -> Result<SwapAndAccountMetas> {
        let mints = self.ready()?;
        let a_to_b = self.direction(params.source_mint, params.destination_mint)?;
        let pool = PoolKeys::from_account(self.key, &self.pool, mints[0].program, mints[1].program);
        let keys = TradeKeys {
            pool,
            user: params.token_transfer_authority,
            user_a: if a_to_b {
                params.source_token_account
            } else {
                params.destination_token_account
            },
            user_b: if a_to_b {
                params.destination_token_account
            } else {
                params.source_token_account
            },
        };
        let exact_out = params.swap_mode == SwapMode::ExactOut;
        let args = SwapArgs {
            a_to_b,
            amount: if exact_out {
                params.out_amount
            } else {
                params.in_amount
            },
            limit: if exact_out {
                params.in_amount
            } else {
                params.out_amount
            },
        };
        let dex = Dex {
            program: PROGRAM_ID,
        };
        let instruction = sdk(if exact_out {
            dex.swap_exact_out(keys, args)
        } else {
            dex.swap(keys, args)
        })?;
        Ok(SwapAndAccountMetas {
            swap: Swap::Placeholder {
                data: Some(vec![u8::from(exact_out), u8::from(a_to_b)]),
            },
            account_metas: instruction.accounts,
        })
    }

    fn supports_exact_out(&self) -> bool {
        true
    }

    fn get_accounts_len(&self) -> usize {
        11
    }

    fn is_active(&self) -> bool {
        self.ready().is_ok()
            && self.pool.reserve_a > 0
            && if self.pool.status == PoolStatus::Amm {
                self.pool.reserve_b > 0
            } else {
                self.pool.reserve_b < self.pool.target_b
            }
    }
}

pub fn swap_instruction_data(swap: &Swap, amount: u64, limit: u64) -> Result<Vec<u8>> {
    let Swap::Placeholder { data: Some(bits) } = swap else {
        return Err("Expected a Fossa swap placeholder".into());
    };
    let [mode @ 0..=1, direction @ 0..=1] = bits.as_slice() else {
        return Err("Invalid Fossa swap decision bits".into());
    };
    let name = if *mode == 0 { "swap" } else { "swap_exact_out" };
    let mut data = fossa_sdk::instruction::discriminator(name).to_vec();
    data.extend(
        borsh::to_vec(&SwapArgs {
            a_to_b: *direction == 1,
            amount,
            limit,
        })
        .map_err(|error| AmmError::from(error.to_string()))?,
    );
    Ok(data)
}

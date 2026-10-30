use fossa_sdk::{
    Pubkey,
    accounts::RawAccount,
    dex::{
        self,
        accounts::{Config, Pool, PoolMode, PoolStatus},
        args::{CurveInit, LiquidityArgs, SwapArgs},
        quote::*,
    },
    reserve::redemption_output,
    token::{self, TokenAccount, TokenMint, TransferFee},
};
use solana_program_pack::Pack;
use spl_token_2022_interface::{
    extension::{
        BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        transfer_fee::TransferFeeConfig,
    },
    state::{Account, AccountState, Mint},
};
fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn untaxed() -> [TransferFee; 2] {
    [TransferFee::default(); 2]
}
fn curve(preset: CurvePreset, mode: PoolMode) -> Pool {
    Pool {
        reserve_a: preset.initial_base,
        initial_base: preset.initial_base,
        virtual_a: preset.virtual_a,
        virtual_b: preset.virtual_b,
        target_b: preset.target_b,
        minimum_a: preset.minimum_a,
        mode,
        ..Pool::default()
    }
}
#[test]
fn transfer_fee_inverse_matches_official_spl_and_is_minimal() {
    for bps in [0, 1, 100, 300, 3333, 9999, 10000] {
        for cap in [0, 1, 10, 1000, u64::MAX] {
            let ours = TransferFee {
                basis_points: bps,
                maximum_fee: cap,
            };
            let spl = spl_token_2022_interface::extension::transfer_fee::TransferFee {
                epoch: 0.into(),
                maximum_fee: cap.into(),
                transfer_fee_basis_points: bps.into(),
            };
            for amount in [
                0,
                1,
                2,
                13,
                9999,
                10000,
                1_000_000,
                u64::MAX / 10000,
                u64::MAX,
            ] {
                assert_eq!(ours.fee(amount).ok(), spl.calculate_fee(amount));
                if let Ok(gross) = ours.gross(amount) {
                    assert_eq!(ours.net(gross).unwrap(), amount);
                    if gross > 0 {
                        assert!(ours.net(gross - 1).unwrap() < amount);
                    }
                }
            }
        }
    }
    assert!(
        TransferFee {
            basis_points: 10001,
            maximum_fee: 1
        }
        .gross(0)
        .is_err()
    );
}
#[test]
fn constant_product_rounding_preserves_value_and_inverse_minimality() {
    for a in [1, 2, 113, 1_000_000, u32::MAX as u64] {
        for b in [2, 7, 359, 10_000_000, u32::MAX as u64] {
            for input in [1, 2, 11, 100_000] {
                let out = constant_product_output(a, b, input).unwrap();
                assert!(
                    u128::from(a + input) * u128::from(b - out) >= u128::from(a) * u128::from(b)
                );
                if out > 0 {
                    let needed = constant_product_input(a, b, out).unwrap();
                    assert!(needed <= input);
                    assert!(constant_product_output(a, b, needed).unwrap() >= out);
                    if needed > 1 {
                        assert!(constant_product_output(a, b, needed - 1).unwrap() < out);
                    }
                }
            }
        }
    }
    assert!(constant_product_output(u64::MAX, 100, 1).is_err());
    assert!(constant_product_input(100, 100, 100).is_err());
    for n in [0, 1, 2, 3, 4, 15, 16, 17, u64::MAX as u128, u128::MAX] {
        let root = u128::from(integer_sqrt(n));
        assert!(root * root <= n);
        assert!(root == u64::MAX as u128 || (root + 1) * (root + 1) > n);
    }
}
#[test]
fn model_curve_partial_fill_matches_terminal_supply_and_graduation() {
    let mut pool = curve(model_preset(), PoolMode::Model);
    let quote = swap_exact_input(
        &pool,
        untaxed(),
        SwapArgs {
            a_to_b: false,
            amount: 100_000_000_000,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(quote.gross_output, 52_720_779_386_314);
    assert_eq!(quote.effective_input, dex::MODEL_RAISE_USDC_ATOMS);
    assert!(quote.unused_input > 0);
    assert_eq!(quote.gross_input + quote.unused_input, 100_000_000_000);
    assert_eq!(quote.creator_fee, 0);
    pool.reserve_a = quote.reserve_a_after;
    pool.reserve_b = quote.reserve_b_after;
    let graduation = graduation(&pool).unwrap();
    assert_eq!(graduation.reserve_b, dex::MODEL_RAISE_USDC_ATOMS);
    assert_eq!(graduation.burn_base_atoms, 151);
    assert!(graduation.locked_lp > 1000);
    assert!(
        swap_exact_input(
            &pool,
            untaxed(),
            SwapArgs {
                a_to_b: true,
                amount: 1000,
                limit: 0
            }
        )
        .is_err()
    );
    assert!(
        u128::from(graduation.reserve_b) * u128::from(pool.reserve_a + pool.virtual_a)
            <= u128::from(pool.reserve_b + pool.virtual_b) * u128::from(graduation.reserve_a)
    );
}
#[test]
fn public_curve_buy_limit_and_partial_fill_keep_unused_input() {
    assert!(public_preset_for_target(1).is_err());
    let mut pool = curve(
        public_preset_for_target(8_500_000_000).unwrap(),
        PoolMode::Standard,
    );
    let oversized = SwapArgs {
        a_to_b: false,
        amount: 20_000_000_000,
        limit: 0,
    };
    assert!(swap_exact_input(&pool, untaxed(), oversized).is_err());
    let first = swap_exact_input(
        &pool,
        untaxed(),
        SwapArgs {
            amount: 4_000_000_000,
            ..oversized
        },
    )
    .unwrap();
    assert!(first.creator_fee > 0);
    assert!(first.gross_output <= dex::PUBLIC_SUPPLY_ATOMS * 3 / 4);
    pool.reserve_a = first.reserve_a_after;
    pool.reserve_b = first.reserve_b_after;
    let last = swap_exact_input(&pool, untaxed(), oversized).unwrap();
    assert!(last.unused_input > 0);
    assert_eq!(last.reserve_b_after, pool.target_b);
    assert!(last.reserve_a_after >= dex::PUBLIC_LIQUIDITY_ATOMS);
    assert!(
        swap_exact_input(
            &pool,
            untaxed(),
            SwapArgs {
                limit: last.net_output + 1,
                ..oversized
            }
        )
        .is_err()
    );
}
#[test]
fn exact_output_accounts_for_taxes_and_independent_trade_fee_rounding() {
    let pool = Pool {
        status: PoolStatus::Amm,
        reserve_a: 10_000_000,
        reserve_b: 20_000_000,
        ..Pool::default()
    };
    let fees = [
        TransferFee {
            basis_points: 300,
            maximum_fee: 10000,
        },
        TransferFee {
            basis_points: 100,
            maximum_fee: u64::MAX,
        },
    ];
    for direction in [true, false] {
        let exact = swap_exact_output(
            &pool,
            fees,
            SwapArgs {
                a_to_b: direction,
                amount: 12345,
                limit: u64::MAX,
            },
        )
        .unwrap();
        assert_eq!(exact.net_output, 12345);
        assert!(exact.input_transfer_tax > 0);
        assert!(exact.output_transfer_tax > 0);
        assert_eq!(
            exact.received_input,
            exact.effective_input + exact.protocol_fee
        );
        let input = swap_exact_input(
            &pool,
            fees,
            SwapArgs {
                a_to_b: direction,
                amount: exact.gross_input,
                limit: 12345,
            },
        )
        .unwrap();
        assert!(input.net_output >= exact.net_output);
        assert!(
            swap_exact_output(
                &pool,
                fees,
                SwapArgs {
                    a_to_b: direction,
                    amount: 12345,
                    limit: exact.gross_input - 1
                }
            )
            .is_err()
        );
    }
}
#[test]
fn liquidity_preserves_full_direct_withdrawal_and_locked_founders() {
    let fees = [
        TransferFee {
            basis_points: 100,
            maximum_fee: u64::MAX,
        },
        TransferFee::default(),
    ];
    let initial = initial_liquidity(1_000_000, 4_000_000, fees).unwrap();
    let pool = Pool {
        status: PoolStatus::Amm,
        reserve_a: initial.net_a,
        reserve_b: initial.net_b,
        ..Pool::default()
    };
    let all = remove_liquidity(
        &pool,
        fees,
        initial.lp_amount,
        LiquidityArgs {
            amount: initial.lp_amount,
            bound_a: 0,
            bound_b: 0,
        },
    )
    .unwrap();
    assert_eq!((all.gross_a, all.gross_b), (pool.reserve_a, pool.reserve_b));
    assert!(all.net_a < all.gross_a);
    let locked = Pool {
        locked_lp: initial.lp_amount,
        ..pool.clone()
    };
    let partial = remove_liquidity(
        &locked,
        fees,
        initial.lp_amount,
        LiquidityArgs {
            amount: initial.lp_amount,
            bound_a: 0,
            bound_b: 0,
        },
    )
    .unwrap();
    assert_eq!(
        (partial.gross_a, partial.gross_b),
        (pool.reserve_a / 2, pool.reserve_b / 2)
    );
    assert!(
        remove_liquidity(
            &locked,
            fees,
            initial.lp_amount,
            LiquidityArgs {
                amount: initial.lp_amount + 1,
                bound_a: 0,
                bound_b: 0
            }
        )
        .is_err()
    );
    let deposit = add_liquidity(
        &pool,
        fees,
        initial.lp_amount,
        LiquidityArgs {
            amount: initial.lp_amount,
            bound_a: 1_000_000,
            bound_b: 4_000_000,
        },
    )
    .unwrap();
    assert_eq!((deposit.gross_a, deposit.gross_b), (1_000_000, 4_000_000));
    let empty = Pool {
        status: PoolStatus::Amm,
        ..Pool::default()
    };
    assert_eq!(
        add_liquidity(
            &empty,
            fees,
            0,
            LiquidityArgs {
                amount: initial.lp_amount,
                bound_a: 1_000_000,
                bound_b: 4_000_000
            }
        )
        .unwrap(),
        initial
    );
    assert!(
        add_liquidity(
            &empty,
            fees,
            0,
            LiquidityArgs {
                amount: initial.lp_amount + 1,
                bound_a: 1_000_000,
                bound_b: 4_000_000
            }
        )
        .is_err()
    );
}
#[test]
fn oracle_requires_window_and_preserves_wrap_and_expiry() {
    let pool = Pool {
        mint_a: key(1),
        mode: PoolMode::Model,
        status: PoolStatus::Amm,
        locked_lp: 1,
        reserve_a: 100,
        reserve_b: 200,
        price_timestamp: 100,
        price_cumulative: u128::MAX - (1u128 << 31),
        ..Pool::default()
    };
    let config = Config {
        enabled: true,
        quote_mint: key(1),
        reference_pool: key(2),
        ..Config::default()
    };
    let checkpoint = refresh_quote(&config, &pool, 100).unwrap();
    assert!(public_target(&checkpoint, 100).is_err());
    assert!(refresh_quote(&checkpoint, &pool, 1899).is_err());
    let price = refresh_quote(&checkpoint, &pool, 1900).unwrap();
    assert_eq!(price.quote_per_usdc_x32, 1u128 << 31);
    assert_eq!(
        public_target(&price, 3700).unwrap(),
        dex::PUBLIC_RAISE_USDC_ATOMS / 2
    );
    assert!(public_target(&price, 3701).is_err());
    let expired = refresh_quote(&price, &pool, 5501).unwrap();
    assert_eq!(expired.quote_per_usdc_x32, 0);
    assert!(public_target(&expired, 5501).is_err());
    assert!(
        cumulative_price(
            &Pool {
                locked_lp: 0,
                ..pool.clone()
            },
            1900
        )
        .is_err()
    );
    assert!(
        cumulative_price(
            &Pool {
                status: PoolStatus::Curve,
                ..pool
            },
            1900
        )
        .is_err()
    );
}
#[test]
fn canonical_pair_and_fee_exemptions_are_explicit() {
    let program = key(100);
    assert_eq!(
        dex::address::pool(&program, &key(1), &key(2)).unwrap(),
        dex::address::pool(&program, &key(2), &key(1)).unwrap()
    );
    assert!(dex::address::pool(&program, &key(1), &key(1)).is_err());
    assert_ne!(
        dex::address::pool(&program, &key(1), &key(2)).unwrap(),
        dex::address::pool(&program, &key(1), &key(3)).unwrap()
    );
    assert_eq!(action_fee(key(1), key(1)), 0);
    assert_eq!(action_fee(key(1), key(2)), 25_000_000);
    assert_eq!(redemption_output(11, 3, 2).unwrap(), 7);
    assert_eq!(redemption_output(11, 3, 3).unwrap(), 11);
    assert!(redemption_output(1, 3, 1).is_err());
    assert!(redemption_output(11, 3, 4).is_err());
}
#[test]
fn token2022_schedule_and_extension_decoders_match_spl() {
    let mut data =
        vec![
            0;
            ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::TransferFeeConfig])
                .unwrap()
        ];
    let mut state = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let fee = state.init_extension::<TransferFeeConfig>(true).unwrap();
    fee.older_transfer_fee.epoch = 10.into();
    fee.older_transfer_fee.maximum_fee = 1000.into();
    fee.older_transfer_fee.transfer_fee_basis_points = 100.into();
    fee.newer_transfer_fee.epoch = 12.into();
    fee.newer_transfer_fee.maximum_fee = 2000.into();
    fee.newer_transfer_fee.transfer_fee_basis_points = 300.into();
    fee.withdraw_withheld_authority = Some(key(4).to_bytes().into()).try_into().unwrap();
    state.base = Mint {
        is_initialized: true,
        supply: 123456789,
        decimals: 6,
        ..Mint::default()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    let account = RawAccount {
        owner: fossa_sdk::address::TOKEN_2022_PROGRAM,
        lamports: 1,
        executable: false,
        data,
    };
    let mint = TokenMint::decode(&account).unwrap();
    assert_eq!(mint.supply, 123456789);
    assert_eq!(mint.decimals, 6);
    assert_eq!(mint.withdraw_withheld_authority, Some(key(4)));
    assert_eq!(mint.transfer_fee(11).basis_points, 100);
    assert_eq!(mint.transfer_fee(12).basis_points, 300);
    mint.validate_dex_base().unwrap();
    let mut bad = account.clone();
    bad.data.pop();
    assert!(TokenMint::decode(&bad).is_err());
    bad = account;
    bad.owner = key(99);
    assert!(TokenMint::decode(&bad).is_err());
    let mut data =
        vec![
            0;
            ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::MintCloseAuthority])
                .unwrap()
        ];
    let mut state = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    state.init_extension::<spl_token_2022_interface::extension::mint_close_authority::MintCloseAuthority>(true).unwrap();
    state.base = Mint {
        is_initialized: true,
        ..Mint::default()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    let with_close = RawAccount {
        owner: fossa_sdk::address::TOKEN_2022_PROGRAM,
        lamports: 1,
        executable: false,
        data,
    };
    assert!(TokenMint::decode(&with_close).is_err());
    assert!(TokenMint::decode_payment(&with_close).is_ok());
}
#[test]
fn native_tokens_preserve_reserve_and_initialization_order() {
    let mut data = vec![0; Account::LEN];
    Account::pack(
        Account {
            mint: fossa_sdk::address::NATIVE_MINT.to_bytes().into(),
            owner: key(1).to_bytes().into(),
            amount: 500,
            state: AccountState::Initialized,
            is_native: Some(2039280).into(),
            ..Account::default()
        },
        &mut data,
    )
    .unwrap();
    let token = TokenAccount::decode(&RawAccount {
        owner: fossa_sdk::address::TOKEN_PROGRAM,
        lamports: 2039780,
        executable: false,
        data,
    })
    .unwrap();
    assert_eq!(token.native_reserve, Some(2039280));
    assert_eq!(token.amount, 500);
    token
        .validate(fossa_sdk::address::NATIVE_MINT, key(1), true)
        .unwrap();
    let setup = token::create_wrapped_native(key(1), key(2), key(1), 500, 2039280).unwrap();
    assert_eq!(setup.len(), 2);
    assert_eq!(setup[0].accounts[1].pubkey, key(2));
    assert!(setup[0].accounts[1].is_signer);
    assert_eq!(setup[1].accounts[1].pubkey, fossa_sdk::address::NATIVE_MINT);
    let sync = token::sync_native(key(2)).unwrap();
    assert_eq!(sync.data, vec![17]);
    assert_eq!(
        token::close_account(key(2), key(1), key(1), fossa_sdk::address::TOKEN_PROGRAM)
            .unwrap()
            .accounts[1]
            .pubkey,
        key(1)
    );
    let config = CurveInit {
        mode: 1,
        tax_bps: 300,
    };
    let curve = token::create_curve_mint(
        key(1),
        key(2),
        key(3),
        fossa_sdk::address::TOKEN_2022_PROGRAM,
        config,
        1000,
    )
    .unwrap();
    assert_eq!(curve.len(), 3);
    assert!(
        token::create_curve_mint(
            key(1),
            key(2),
            key(3),
            fossa_sdk::address::TOKEN_PROGRAM,
            config,
            1000
        )
        .is_err()
    );
}

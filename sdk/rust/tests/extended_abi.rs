use base64::{Engine, engine::general_purpose::STANDARD};
use fossa_sdk::{
    Instruction, Protocol, Pubkey,
    accounts::{Decode, ModelGroup, RawAccount},
    address::{self, TOKEN_2022_PROGRAM, TOKEN_PROGRAM},
    dex::{self, Dex, LaunchKeys, LiquidityKeys, PoolKeys, TradeKeys, args::*},
    instruction::ListCreation,
    reserve::*,
};
use serde_json::{Value, json};
fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/extended-abi.json")).unwrap()
}
fn check(ix: Instruction, expected: &Value) {
    assert_eq!(STANDARD.encode(&ix.data), expected["dataBase64"]);
    assert_eq!(json!(ix.accounts.iter().map(|account| json!({"address":account.pubkey.to_string(),"signer":account.is_signer,"writable":account.is_writable})).collect::<Vec<_>>()), expected["accounts"]);
}
fn protocol(f: &Value) -> Protocol {
    Protocol {
        program: f["routerProgram"].as_str().unwrap().parse().unwrap(),
        market: key(1),
        mint: key(2),
    }
}
fn dex(f: &Value) -> Dex {
    Dex {
        program: f["dexProgram"].as_str().unwrap().parse().unwrap(),
    }
}
#[test]
fn every_additional_router_builder_matches_anchor_generated_client() {
    let f = fixture();
    let p = protocol(&f);
    let dex = dex(&f);
    let group = address::model_group(&p.program, &p.market, &key(21).to_bytes()).0;
    let group_keys = GroupKeys {
        group,
        mint: address::reserve_mint(&p.program, &group).0,
        backing: key(22),
    };
    let metadata = TokenMetadata {
        name: "Example Group".into(),
        symbol: "GROUP".into(),
        uri: "https://fixture.invalid/group.json".into(),
    };
    let config = FeeConfig {
        bank: key(16),
        creation_mint: key(27),
        creation_token_program: TOKEN_2022_PROGRAM,
        creation_net_price: 1234567,
        creation_enabled: true,
    };
    let model = address::model(&p.program, &p.market, &key(6).to_bytes()).0;
    let cases = [
        ("configure_fees", p.configure_fees(key(3), config).unwrap()),
        (
            "configure_fees_disabled",
            p.configure_fees(
                key(3),
                FeeConfig {
                    creation_enabled: false,
                    ..config
                },
            )
            .unwrap(),
        ),
        (
            "configure_list_fees",
            p.configure_list_fees(
                key(13),
                key(3),
                ListFees {
                    fee_bps: 125,
                    protocol_share: 7500,
                },
            )
            .unwrap(),
        ),
        (
            "initialize_list",
            p.initialize_list(
                key(13),
                key(3),
                ListCreation {
                    limit: 8,
                    fee_bps: 125,
                    protocol_share: 7500,
                    maximum_payment: 1234567,
                    source: key(14),
                    destination: key(15),
                    mint: key(27),
                    token_program: TOKEN_2022_PROGRAM,
                },
            )
            .unwrap(),
        ),
        (
            "initialize_model_group",
            p.initialize_model_group(
                InitializeModelGroup {
                    council: key(3),
                    payer: key(12),
                    group_id: key(21).to_bytes(),
                    backing: key(22),
                    council_tokens: key(23),
                    dex_program: dex.program,
                    dex_treasury: key(17),
                    curve_quote_source: key(14),
                    curve_base_vault: key(24),
                    curve_quote_vault: key(25),
                    curve_lp: key(26),
                },
                &GroupInit {
                    tax_bps: 300,
                    maximum_tax: 9999,
                    metadata: metadata.clone(),
                },
            )
            .unwrap(),
        ),
        (
            "assign_model_group",
            p.assign_model_group(key(3), model, Some(group)),
        ),
        (
            "clear_model_group",
            p.assign_model_group(key(3), model, None),
        ),
        (
            "update_group_metadata",
            p.update_group_metadata(key(3), key(12), group_keys, &metadata)
                .unwrap(),
        ),
        (
            "configure_reserve",
            p.configure_reserve(
                key(3),
                group_keys,
                ReserveConfig {
                    route_to_reserve: false,
                    tax_bps: 100,
                    maximum_tax: 777,
                },
            )
            .unwrap(),
        ),
        (
            "redeem_reserve",
            p.redeem_reserve(
                RedemptionKeys {
                    group: group_keys,
                    holder: key(4),
                    source: key(14),
                    destination: key(15),
                    council: key(3),
                },
                Redeem {
                    amount: 123,
                    minimum_output: 45,
                },
            )
            .unwrap(),
        ),
        (
            "collect_reserve_tax",
            p.collect_reserve_tax(group_keys, key(15)),
        ),
        ("sweep_reserve", p.sweep_reserve(group_keys, key(15))),
        ("configure_dex", p.configure_dex(key(3), dex)),
        ("rotate_council", p.rotate_council(key(3), key(19))),
    ];
    assert_eq!(cases.len(), f["router"].as_object().unwrap().len());
    for (name, ix) in cases {
        eprintln!("{name}");
        check(ix, &f["router"][name]);
    }
}
#[test]
fn every_dex_builder_matches_anchor_generated_client() {
    let f = fixture();
    let dex = dex(&f);
    let protocol = protocol(&f);
    let group = address::model_group(&protocol.program, &protocol.market, &key(21).to_bytes()).0;
    let reference = dex
        .pool_address(address::reserve_mint(&protocol.program, &group).0, key(2))
        .unwrap()
        .0;
    let pool = dex.pool_address(key(27), key(2)).unwrap().0;
    let launch = LaunchKeys {
        payer: key(12),
        treasury: key(17),
        mint_a: key(27),
        mint_b: key(2),
        token_a: TOKEN_2022_PROGRAM,
        token_b: TOKEN_PROGRAM,
        source_a: key(14),
        source_b: key(15),
        vault_a: key(24),
        vault_b: key(25),
        user_lp: key(26),
    };
    let pool_keys = PoolKeys {
        pool,
        mint_a: launch.mint_a,
        mint_b: launch.mint_b,
        vault_a: launch.vault_a,
        vault_b: launch.vault_b,
        lp_mint: dex.lp_mint_address(pool).0,
        token_a: launch.token_a,
        token_b: launch.token_b,
    };
    let trade = TradeKeys {
        pool: pool_keys,
        user: key(4),
        user_a: key(14),
        user_b: key(15),
    };
    let liquidity = LiquidityKeys {
        trade,
        treasury: key(17),
        user_lp: key(26),
    };
    let global = GlobalConfig {
        treasury: key(17),
        distributor: key(18),
        usdc_mint: key(2),
    };
    let config = ConfigInit {
        treasury: key(17),
        rates: Rates::default(),
        distributor: Some(key(18)),
    };
    let pool_init = PoolInit {
        amount_a: 1234,
        amount_b: 5678,
        minimum_lp: 901,
    };
    let liquidity_args = LiquidityArgs {
        amount: 1234,
        bound_a: 567,
        bound_b: 890,
    };
    let cases = [
        (
            "initialize_global",
            dex.initialize_global(key(3), key(20), global).unwrap(),
        ),
        (
            "update_global",
            dex.update_global(key(3), key(19), global).unwrap(),
        ),
        (
            "initialize_config",
            dex.initialize_config(key(3), key(2), TOKEN_PROGRAM, Some(reference), config)
                .unwrap(),
        ),
        (
            "initialize_config_none",
            dex.initialize_config(
                key(3),
                key(2),
                TOKEN_PROGRAM,
                None,
                ConfigInit {
                    distributor: None,
                    ..config
                },
            )
            .unwrap(),
        ),
        (
            "update_config",
            dex.update_config(key(3), key(2), TOKEN_PROGRAM, false)
                .unwrap(),
        ),
        (
            "initialize_pool",
            dex.initialize_pool(launch, Some(key(3)), pool_init)
                .unwrap(),
        ),
        (
            "initialize_pool_none",
            dex.initialize_pool(launch, None, pool_init).unwrap(),
        ),
        (
            "initialize_curve",
            dex.initialize_curve(
                launch,
                CurveInit {
                    mode: 2,
                    tax_bps: 300,
                },
            )
            .unwrap(),
        ),
        (
            "initialize_model_curve",
            dex.initialize_model_curve(launch, key(3)).unwrap(),
        ),
        (
            "swap",
            dex.swap(
                trade,
                SwapArgs {
                    a_to_b: true,
                    amount: 1234,
                    limit: 567,
                },
            )
            .unwrap(),
        ),
        (
            "swap_exact_out",
            dex.swap_exact_out(
                trade,
                SwapArgs {
                    a_to_b: false,
                    amount: 1234,
                    limit: 5678,
                },
            )
            .unwrap(),
        ),
        ("graduate", dex.graduate(pool_keys, key(18)).unwrap()),
        (
            "add_liquidity",
            dex.add_liquidity(liquidity, liquidity_args).unwrap(),
        ),
        (
            "remove_liquidity",
            dex.remove_liquidity(liquidity, liquidity_args).unwrap(),
        ),
        (
            "collect_fees",
            dex.collect_fees(
                trade,
                CollectArgs {
                    protocol: true,
                    minimum_a: 123,
                    minimum_b: 456,
                },
            )
            .unwrap(),
        ),
        (
            "collect_transfer_tax",
            dex.collect_transfer_tax(pool, key(27), key(15), &[key(14), key(5)])
                .unwrap(),
        ),
        (
            "refresh_quote",
            dex.refresh_quote(dex.config_address(key(2)).0, reference)
                .unwrap(),
        ),
        (
            "update_recipients",
            dex.update_recipients(key(3), pool).unwrap(),
        ),
    ];
    assert_eq!(cases.len(), f["dex"].as_object().unwrap().len());
    for (name, ix) in cases {
        eprintln!("{name}");
        check(ix, &f["dex"][name]);
    }
}
fn raw(f: &Value, name: &str, owner: Pubkey) -> (Pubkey, RawAccount) {
    let value = &f["states"][name];
    (
        value["address"].as_str().unwrap().parse().unwrap(),
        RawAccount {
            owner,
            lamports: 1,
            executable: false,
            data: STANDARD
                .decode(value["dataBase64"].as_str().unwrap())
                .unwrap(),
        },
    )
}
fn rejects_corruption<T: Decode>(address: Pubkey, account: &RawAccount, program: Pubkey) {
    T::decode(&address, account, &program).unwrap_or_else(|_| panic!("valid fixture rejected"));
    assert!(T::decode(&key(99), account, &program).is_err());
    let mut bad = account.clone();
    bad.owner = key(98);
    assert!(T::decode(&address, &bad, &program).is_err());
    bad = account.clone();
    bad.executable = true;
    assert!(T::decode(&address, &bad, &program).is_err());
    bad = account.clone();
    bad.data[0] ^= 1;
    assert!(T::decode(&address, &bad, &program).is_err());
    bad = account.clone();
    bad.data.push(0);
    assert!(T::decode(&address, &bad, &program).is_err());
    bad = account.clone();
    bad.data.pop();
    assert!(T::decode(&address, &bad, &program).is_err());
    bad = account.clone();
    *bad.data.last_mut().unwrap() = 1;
    assert!(T::decode(&address, &bad, &program).is_err());
}
#[test]
fn decoders_match_anchor_state_and_reject_invalid_bindings() {
    let f = fixture();
    let protocol = protocol(&f);
    let dex = dex(&f);
    let (at, account) = raw(&f, "ModelGroup", protocol.program);
    rejects_corruption::<ModelGroup>(at, &account, protocol.program);
    let state = ModelGroup::decode(&at, &account, &protocol.program).unwrap();
    assert_eq!(state.market, key(1));
    assert_eq!(state.group_id, key(21).to_bytes());
    assert_eq!(state.backing_vault, key(22));
    assert!(state.route_to_reserve);
    assert_eq!(
        state.fixed_token_mint,
        address::reserve_mint(&protocol.program, &at).0
    );
    let mut bad = account.clone();
    bad.data[104] ^= 1;
    assert!(ModelGroup::decode(&at, &bad, &protocol.program).is_err());
    bad = account;
    bad.data[136] = 2;
    assert!(ModelGroup::decode(&at, &bad, &protocol.program).is_err());
    let (at, account) = raw(&f, "Global", dex.program);
    rejects_corruption::<dex::accounts::Global>(at, &account, dex.program);
    let state = dex::accounts::Global::decode(&at, &account, &dex.program).unwrap();
    assert_eq!(
        (
            state.council,
            state.treasury,
            state.distributor,
            state.usdc_mint
        ),
        (key(3), key(17), key(18), key(2))
    );
    let (at, account) = raw(&f, "Config", dex.program);
    rejects_corruption::<dex::accounts::Config>(at, &account, dex.program);
    let config = dex::accounts::Config::decode(&at, &account, &dex.program).unwrap();
    assert_eq!(
        (
            config.council,
            config.treasury,
            config.quote_mint,
            config.distributor
        ),
        (key(3), key(17), key(2), key(18))
    );
    assert_eq!(
        (
            config.trade_bps,
            config.protocol_share,
            config.creator_share
        ),
        (125, 10000, 0)
    );
    assert!(config.enabled);
    assert_eq!(
        (
            config.oracle_cumulative,
            config.oracle_timestamp,
            config.quote_per_usdc_x32,
            config.quote_price_timestamp
        ),
        ((1u128 << 100) + 123, 1800, (1u128 << 70) + 321, 3600)
    );
    let (at, account) = raw(&f, "Pool", dex.program);
    rejects_corruption::<dex::accounts::Pool>(at, &account, dex.program);
    let pool = dex::accounts::Pool::decode(&at, &account, &dex.program).unwrap();
    assert_eq!(
        (
            pool.creator,
            pool.treasury,
            pool.mint_a,
            pool.mint_b,
            pool.vault_a,
            pool.vault_b,
            pool.distributor
        ),
        (key(12), key(17), key(27), key(2), key(24), key(25), key(18))
    );
    assert_eq!(pool.config, dex.config_address(key(2)).0);
    assert_eq!(pool.lp_mint, dex.lp_mint_address(at).0);
    assert_eq!(
        [
            pool.reserve_a,
            pool.reserve_b,
            pool.protocol_a,
            pool.protocol_b,
            pool.creator_a,
            pool.creator_b,
            pool.virtual_a,
            pool.virtual_b,
            pool.target_b,
            pool.minimum_a,
            pool.locked_lp
        ],
        [101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111]
    );
    assert_eq!(
        (
            pool.trade_bps,
            pool.protocol_share,
            pool.creator_share,
            pool.tax_bps
        ),
        (125, 10000, 0, 300)
    );
    assert_eq!(pool.status, dex::accounts::PoolStatus::Amm);
    assert_eq!(pool.mode, dex::accounts::PoolMode::Community);
    assert_eq!(
        (
            pool.tax_collected,
            pool.initial_base,
            pool.price_cumulative,
            pool.price_timestamp
        ),
        ((1u128 << 99) + 112, 113, (1u128 << 98) + 114, 115)
    );
}

use fossa_sdk::{
    Pubkey,
    accounts::{Decode, RawAccount},
    instruction::Protocol,
    provider_limits::*,
    scoped::ListTerms,
};
use serde_json::{Value, json};

#[test]
fn provider_limits_match_generated_anchor_abi() {
    let native: Value = serde_json::from_str(include_str!("fixtures/native-abi.json")).unwrap();
    let rows = include_str!("fixtures/provider-limits-abi.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let key = |name: &str| {
        native["keys"][name]
            .as_str()
            .unwrap()
            .parse::<Pubkey>()
            .unwrap()
    };
    let p = Protocol {
        program: native["program"].as_str().unwrap().parse().unwrap(),
        market: key("market"),
        mint: key("mint"),
    };
    let limits = ProviderLimits {
        global: RequestLimits {
            concurrency: 100,
            quota: 200,
            quota_window_seconds: 60,
        },
        wallet: RequestLimits {
            concurrency: 10,
            quota: 20,
            quota_window_seconds: 60,
        },
    };
    let cases = [
        (
            "activate_provider_limits",
            p.activate_provider_limits(key("payer")),
        ),
        (
            "initialize_provider_limit_page",
            p.initialize_provider_limit_page(key("payer"), 0, 8)
                .unwrap(),
        ),
        (
            "grow_provider_limit_page",
            p.grow_provider_limit_page(key("payer"), 0, 16).unwrap(),
        ),
        (
            "initialize_provider_limits",
            p.initialize_provider_limits(key("provider"), key("payer"), 0, limits)
                .unwrap(),
        ),
        (
            "configure_provider_limits",
            p.configure_provider_limits(key("provider"), 0, limits)
                .unwrap(),
        ),
    ];
    for (name, ix) in cases {
        let expected = rows.iter().find(|r| r["name"] == name).unwrap();
        assert_eq!(json!(ix.data), expected["data"], "{name} arguments");
        assert_eq!(json!(ix.accounts.iter().map(|a| json!({"address":a.pubkey.to_string(),"writable":a.is_writable,"signer":a.is_signer})).collect::<Vec<_>>()), expected["accounts"], "{name} accounts");
    }
    for fixture in rows.iter().filter(|r| r["layout"].is_string()) {
        let at = fixture["address"].as_str().unwrap().parse().unwrap();
        let raw = RawAccount {
            owner: p.program,
            lamports: 1,
            executable: false,
            data: serde_json::from_value(fixture["data"].clone()).unwrap(),
        };
        macro_rules! check {
            ($ty:ty) => {{
                assert!(<$ty>::decode(&at, &raw, &p.program).is_ok());
                assert!(<$ty>::decode(&Pubkey::new_unique(), &raw, &p.program).is_err());
                let mut foreign = raw.clone();
                foreign.owner = Pubkey::new_unique();
                assert!(<$ty>::decode(&at, &foreign, &p.program).is_err());
            }};
        }
        match fixture["layout"].as_str().unwrap() {
            "ProviderLimitPage" => check!(ProviderLimitPage),
            "ProviderLimitIndex" => check!(ProviderLimitIndex),
            "ListTerms" => {
                check!(ListTerms);
                assert_eq!(
                    ListTerms::decode(&at, &raw, &p.program)
                        .unwrap()
                        .limits_version,
                    1
                );
            }
            _ => unreachable!(),
        }
    }
}

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn request() -> fossa_sdk::instruction::BookRequest {
    fossa_sdk::instruction::BookRequest {
        minimum_wallet_capacity: Some(10),
        min_context: 0,
        limits_configured: false,
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        model_id: [9; 32],
        provider_list: key(5),
        rate_caps: fossa_sdk::amount::Rates {
            input: 10_000,
            cached: 1000,
            output: 10_000,
        },
        min_speed: 1,
        max_input: 100,
        max_output: 100,
        private: false,
        max_spend: 1000,
        estimated_input: 10,
        estimated_cached: 0,
        estimated_output: 20,
        cache_provider: Pubkey::default(),
    }
}
#[test]
fn preview_shares_four_limits_preserves_ties_and_rejects_foreign_or_duplicate_state() {
    use fossa_sdk::{
        accounts::{BookingLimits, Offer, OfferTerms, ProviderList},
        amount::Rates,
        scoped::{ListOffer, ListRouting, PolicyMirror, ProviderUsage, ProviderWallet},
    };
    let terms = OfferTerms {
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        context_window: 1000,
        max_input: 1000,
        max_output: 1000,
        rates: Rates {
            input: 1000,
            cached: 100,
            output: 1000,
        },
        speed_tps: 10,
        private: false,
        active: true,
        capacity: 1,
    };
    let mut book = ListTerms {
        limits_version: 1,
        model: key(9),
        list: key(5),
        capacity_address: key(6),
        limit: 2,
        next_order: 2,
        entries: vec![
            ListOffer {
                offer: Offer {
                    provider: key(1),
                    payout: key(1),
                    terms,
                    provider_policy: 0,
                },
                order: 1,
            },
            ListOffer {
                offer: Offer {
                    provider: key(2),
                    payout: key(2),
                    terms,
                    provider_policy: 7,
                },
                order: 0,
            },
        ],
    };
    let row = ProviderLimitEntry {
        concurrency_limit: 100,
        quota_limit: 300,
        quota_window_seconds: 60,
        quota_window_end: 160,
        quota_used: 99,
        active_bookings: 99,
        wallet_concurrency_limit: 10,
        wallet_quota_limit: 20,
        wallet_quota_window_seconds: 60,
    };
    let mut pages = vec![
        ProviderLimitPage {
            market: key(3),
            page: 0,
            capacity: 1,
            entries: vec![row.clone()],
        },
        ProviderLimitPage {
            market: key(3),
            page: 7,
            capacity: 1,
            entries: vec![row],
        },
    ];
    let list = ProviderList {
        market: key(3),
        authority: key(4),
        limit: 2,
        fee_bps: 100,
        protocol_share: 10000,
        routing_enabled: true,
        providers: vec![key(1), key(2)],
    };
    let mut routing = ListRouting {
        market: key(3),
        list: key(5),
        capacity: 2,
        book_count: 1,
        entries: [key(1), key(2)]
            .map(|provider| PolicyMirror {
                provider,
                limits: BookingLimits {
                    wallet_window_seconds: 60,
                    wallet_bookings_per_window: 1,
                    wallet_booking_capacity: 1,
                },
                epoch: 1,
                active: true,
            })
            .to_vec(),
    };
    let mut wallet = ProviderWallet {
        market: key(3),
        owner: key(4),
        rent_payer: key(4),
        capacity: 2,
        entries: vec![ProviderUsage {
            provider: key(2),
            window_end: 160,
            bookings: 19,
            outstanding: 9,
        }],
    };
    let request = request();
    let choose = |book: &ListTerms,
                  pages: &[ProviderLimitPage],
                  list: &ProviderList,
                  routing: &ListRouting,
                  wallet: Option<&ProviderWallet>,
                  now| {
        request
            .select_provider_limits(book, pages, list, routing, wallet, 1000, now)
            .unwrap()
            .map(|s| s.slot)
    };
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 100),
        Some(1)
    );
    pages[1].entries[0].active_bookings = 100;
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 100),
        Some(0)
    );
    pages[1].entries[0].active_bookings = 99;
    pages[1].entries[0].quota_used = 300;
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 100),
        Some(0)
    );
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 160),
        Some(1)
    );
    pages[1].entries[0].quota_used = 99;
    wallet.entries[0].outstanding = 10;
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 100),
        Some(0)
    );
    wallet.entries[0].outstanding = 9;
    wallet.entries[0].bookings = 20;
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 100),
        Some(0)
    );
    assert_eq!(
        choose(&book, &pages, &list, &routing, Some(&wallet), 160),
        Some(1)
    );
    assert_eq!(choose(&book, &pages, &list, &routing, None, 100), Some(1));
    routing.entries[1].active = false;
    assert_eq!(choose(&book, &pages, &list, &routing, None, 100), Some(0));
    assert!(
        request
            .select_provider_limits(&book, &pages[..1], &list, &routing, None, 1000, 100)
            .is_err()
    );
    let mut foreign = list.clone();
    foreign.market = key(99);
    assert!(
        request
            .select_provider_limits(&book, &pages, &foreign, &routing, None, 1000, 100)
            .is_err()
    );
    let mut duplicate = pages.clone();
    duplicate.push(pages[0].clone());
    assert!(
        request
            .select_provider_limits(&book, &duplicate, &list, &routing, None, 1000, 100)
            .is_err()
    );
    book.entries[1].offer.provider_policy = 0;
    assert!(
        request
            .select_provider_limits(&book, &pages, &list, &routing, None, 1000, 100)
            .is_err()
    );
    book.entries[1].offer.provider_policy = 7;
    book.limits_version = 0;
    assert!(
        request
            .select_provider_limits(&book, &pages, &list, &routing, None, 1000, 100)
            .is_err()
    );
    pages[0].entries[0].concurrency_limit = u32::MAX;
    assert_eq!(pages[0].entries[0].limits().global.concurrency, 0);
}

#[test]
fn thirty_two_page_booking_fits_v0_and_preserves_signed_journal_and_lookup_binding() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use fossa_sdk::{
        Hash, Keypair, PendingTransaction, Signer,
        instruction::{BookingMode, ModelKeys, UserKeys},
        transaction::{PreparedTransaction, PreparedVersionedTransaction, compute_unit_limit},
    };
    use solana_message::{AddressLookupTableAccount, VersionedMessage};
    let owner = Keypair::new_from_array([31; 32]);
    let booking = Keypair::new_from_array([32; 32]);
    let p = Protocol {
        program: key(1),
        market: key(3),
        mint: key(4),
    };
    let model = ModelKeys {
        model: fossa_sdk::address::model(&p.program, &p.market, &request().model_id).0,
        terms: key(10),
        capacity: key(11),
        group: Pubkey::default(),
    };
    let user = UserKeys {
        owner: owner.pubkey(),
        user: key(12),
        vault: key(13),
    };
    let reserve = p
        .book_with_provider_limits(
            model,
            user,
            booking.pubkey(),
            owner.pubkey(),
            &request(),
            BookingMode::OneShot,
            &(0..32).collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(reserve.accounts.len(), 43);
    let instructions = vec![
        compute_unit_limit(1_400_000),
        p.initialize_reusable_booking(user, booking.pubkey(), owner.pubkey()),
        reserve,
    ];
    let latest = fossa_sdk::rpc::LatestBlockhash {
        blockhash: Hash::new_from_array([44; 32]),
        last_valid_block_height: 100,
        min_context_slot: 9,
    };
    assert!(
        PreparedTransaction::new(
            &instructions,
            owner.pubkey(),
            latest.blockhash.clone(),
            100,
            9
        )
        .is_err()
    );
    let table = AddressLookupTableAccount {
        key: key(100),
        addresses: (0..32)
            .map(|page| fossa_sdk::address::provider_limit_page(&p.program, &p.market, page).0)
            .collect(),
    };
    let build = || {
        PreparedVersionedTransaction::new(
            &instructions,
            owner.pubkey(),
            std::slice::from_ref(&table),
            &latest,
        )
        .unwrap()
    };
    assert!(build().sign(&[&owner]).is_err());
    let pending = build().sign(&[&owner, &booking]).unwrap();
    assert!(STANDARD.decode(&pending.wire_base64).unwrap().len() <= 1232);
    assert!(pending.transaction().is_err());
    let transaction = pending.versioned_transaction().unwrap();
    let VersionedMessage::V0(message) = &transaction.message else {
        panic!("v0 expected")
    };
    let loaded = &message.address_table_lookups[0];
    assert_eq!(loaded.account_key, table.key);
    assert_eq!(loaded.writable_indexes.len(), 32);
    assert!(loaded.readonly_indexes.is_empty());
    let mut keys = message.account_keys.clone();
    keys.extend(
        loaded
            .writable_indexes
            .iter()
            .map(|&i| table.addresses[i as usize]),
    );
    let compiled = message.instructions.last().unwrap();
    assert_eq!(
        compiled
            .accounts
            .iter()
            .map(|&i| keys[i as usize])
            .collect::<Vec<_>>(),
        instructions
            .last()
            .unwrap()
            .accounts
            .iter()
            .map(|a| a.pubkey)
            .collect::<Vec<_>>()
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pending.json");
    pending.save(&path).unwrap();
    let recovered = PendingTransaction::load(&path).unwrap();
    assert_eq!(recovered.wire_base64, pending.wire_base64);
    assert_eq!(recovered.validate().unwrap(), transaction.signatures[0]);
    for change in 0..4 {
        let mut invalid = transaction.clone();
        match change {
            0 => invalid.signatures[0] = Default::default(),
            1 => {
                let VersionedMessage::V0(m) = &mut invalid.message else {
                    unreachable!()
                };
                m.address_table_lookups[0].writable_indexes.swap(0, 1);
            }
            2 => {
                let VersionedMessage::V0(m) = &mut invalid.message else {
                    unreachable!()
                };
                m.instructions[2].accounts[0] = 255;
            }
            _ => invalid.signatures.clear(),
        }
        let mut corrupt = pending.clone();
        corrupt.wire_base64 = STANDARD.encode(bincode::serialize(&invalid).unwrap());
        assert!(corrupt.validate().is_err());
    }
    let mut trailing = pending.clone();
    let mut bytes = STANDARD.decode(&trailing.wire_base64).unwrap();
    bytes.push(0);
    trailing.wire_base64 = STANDARD.encode(bytes);
    assert!(trailing.validate().is_err());
    assert!(
        PreparedVersionedTransaction::new(
            &instructions,
            owner.pubkey(),
            &[table.clone(), table],
            &latest
        )
        .is_err()
    );
    let legacy = PreparedTransaction::new(
        &[compute_unit_limit(1000)],
        owner.pubkey(),
        latest.blockhash,
        100,
        9,
    )
    .unwrap()
    .sign(&[&owner])
    .unwrap();
    assert!(matches!(
        legacy.versioned_transaction().unwrap().message,
        VersionedMessage::Legacy(_)
    ));
    assert_eq!(
        bincode::serialize(&legacy.transaction().unwrap()).unwrap(),
        STANDARD.decode(legacy.wire_base64).unwrap()
    );
}

#[test]
fn marker_three_settlement_uses_original_provider_page_and_wallet_owner() {
    use fossa_sdk::instruction::{ModelKeys, OfferKeys, SettlementKeys, UserKeys};
    let p = Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    };
    let keys = SettlementKeys {
        wallet_booking_counted: 3,
        provider_list: key(4),
        offer: OfferKeys {
            model: ModelKeys {
                model: key(5),
                terms: key(6),
                capacity: key(7),
                group: Pubkey::default(),
            },
            provider: key(8),
            payout: key(9),
            slot: (19 << 5) | 27,
        },
        user: UserKeys {
            user: key(10),
            owner: key(11),
            vault: key(12),
        },
        booking: key(13),
        fee_destinations: Some([key(14), key(15)]),
    };
    for ix in [
        p.release(keys, Some(44)).unwrap(),
        p.expire(keys, Some(44)).unwrap(),
        p.claim(
            keys,
            fossa_sdk::amount::Usage {
                uncached: 1,
                cached: 0,
                output: 2,
            },
            None,
        )
        .unwrap(),
    ] {
        assert_eq!(
            ix.accounts[2].pubkey,
            fossa_sdk::address::provider_limit_page(&p.program, &p.market, 27).0
        );
        assert!(ix.accounts[2].is_writable);
        assert!(ix.accounts.iter().any(|a| a.is_writable
            && a.pubkey
                == fossa_sdk::address::provider_wallet(&p.program, &p.market, &keys.user.owner).0));
    }
    let claim = p
        .claim(
            keys,
            fossa_sdk::amount::Usage {
                uncached: 1,
                cached: 0,
                output: 2,
            },
            None,
        )
        .unwrap();
    assert_eq!(claim.accounts[12].pubkey, key(14));
    assert_eq!(claim.accounts[13].pubkey, key(15));
    let mut legacy = keys;
    legacy.wallet_booking_counted = 2;
    assert_eq!(
        p.release(legacy, Some(44)).unwrap().accounts[2].pubkey,
        fossa_sdk::address::list_capacity(&p.program, &keys.provider_list, &keys.offer.model.model)
            .0
    );
}

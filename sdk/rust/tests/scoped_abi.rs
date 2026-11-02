use fossa_sdk::{
    Instruction, Pubkey,
    accounts::{BookingLimits, Decode, OfferTerms, RawAccount},
    amount::{Rates, Usage},
    instruction::{
        BookRequest, BookingMode, ModelKeys, OfferKeys, Protocol, SettlementKeys, UserKeys,
    },
    scoped::*,
};
use serde_json::{Value, json};

#[test]
fn scoped_builders_and_decoders_match_native_anchor_fixtures() {
    let old: Value = serde_json::from_str(include_str!("fixtures/native-abi.json")).unwrap();
    let rows = include_str!("fixtures/scoped-abi.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let k = |name: &str| {
        old["keys"][name]
            .as_str()
            .unwrap()
            .parse::<Pubkey>()
            .unwrap()
    };
    let p = Protocol {
        program: old["program"].as_str().unwrap().parse().unwrap(),
        market: k("market"),
        mint: k("mint"),
    };
    let m = ModelKeys {
        model: k("model"),
        terms: k("terms"),
        capacity: k("capacity"),
        group: Pubkey::default(),
    };
    let u = UserKeys {
        user: k("user"),
        owner: k("owner"),
        vault: k("vault"),
    };
    let provider = k("provider");
    let payer = k("payer");
    let list = k("list");
    let limits = BookingLimits {
        wallet_window_seconds: 60,
        wallet_bookings_per_window: 5,
        wallet_booking_capacity: 2,
    };
    let terms = OfferTerms {
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        context_window: 1_000_000,
        max_input: 1_000_000,
        max_output: 1_000_000,
        rates: Rates {
            input: 1000,
            cached: 100,
            output: 2000,
        },
        speed_tps: 32,
        private: true,
        active: true,
        capacity: 10,
    };
    let request = BookRequest {
        minimum_wallet_capacity: None,
        min_context: 0,
        limits_configured: false,
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        model_id: k("model_id").to_bytes(),
        provider_list: list,
        rate_caps: Rates {
            input: 10_000,
            cached: 1000,
            output: 20_000,
        },
        min_speed: 32,
        max_input: 100,
        max_output: 200,
        private: true,
        max_spend: 50_000,
        estimated_input: 10,
        estimated_cached: 3,
        estimated_output: 20,
        cache_provider: provider,
    };
    let generation = old["generation"].as_str().unwrap().parse().unwrap();
    let settlement = SettlementKeys {
        wallet_booking_counted: 2,
        provider_list: list,
        offer: OfferKeys {
            model: m,
            provider,
            payout: k("payout"),
            slot: 0,
        },
        user: u,
        booking: k("booking"),
        fee_destinations: None,
    };
    let mut demand = request.clone();
    demand.minimum_wallet_capacity = Some(1000);
    let constrained = p
        .book_scoped(
            m,
            u,
            k("booking"),
            payer,
            &demand,
            BookingMode::Reusable {
                generation,
                fresh: true,
            },
        )
        .unwrap();
    assert_eq!(
        &constrained.data[constrained.data.len() - 5..],
        &[1, 232, 3, 0, 0]
    );
    demand.minimum_wallet_capacity = Some(0);
    assert!(
        p.book_scoped(m, u, k("booking"), payer, &demand, BookingMode::OneShot)
            .is_err()
    );
    let mut unsupported = settlement;
    unsupported.wallet_booking_counted = 1;
    assert!(p.release(unsupported, Some(generation)).is_err());
    let cases: Vec<(&str, Instruction)> = vec![
        ("activate_list_routing", p.activate_list_routing(payer)),
        (
            "initialize_provider_policy",
            p.initialize_provider_policy(provider, payer, limits)
                .unwrap(),
        ),
        (
            "begin_provider_policy_update",
            p.begin_provider_policy_update(provider),
        ),
        (
            "commit_provider_policy_update",
            p.commit_provider_policy_update(provider, limits).unwrap(),
        ),
        (
            "initialize_list_routing",
            p.initialize_list_routing(list, payer, payer, 8).unwrap(),
        ),
        (
            "grow_list_routing",
            p.grow_list_routing(list, payer, payer, 16),
        ),
        (
            "update_routing_list",
            p.update_routing_list(list, payer, provider, false, true),
        ),
        (
            "enable_policy_mirror",
            p.enable_policy_mirror(list, provider),
        ),
        (
            "freeze_policy_mirror",
            p.freeze_policy_mirror(list, provider, provider),
        ),
        ("close_list_routing", p.close_list_routing(list, payer)),
        (
            "initialize_model_capacity",
            p.initialize_model_capacity(m, provider, payer, 20),
        ),
        (
            "configure_model_capacity",
            p.configure_model_capacity(m, provider, 30),
        ),
        ("sync_legacy_capacity", p.sync_legacy_capacity(m, provider)),
        (
            "initialize_list_books",
            p.initialize_list_books(list, m.model, payer, payer, 8)
                .unwrap(),
        ),
        (
            "grow_list_books",
            p.grow_list_books(list, m.model, payer, payer, 16),
        ),
        ("close_list_books", p.close_list_books(list, m.model, payer)),
        (
            "set_list_offer",
            p.set_list_offer(list, m.model, provider, k("payout"), terms)
                .unwrap(),
        ),
        (
            "prune_list_offer",
            p.prune_list_offer(list, m.model, provider),
        ),
        (
            "initialize_provider_wallet",
            p.initialize_provider_wallet(u.owner, payer, 8).unwrap(),
        ),
        (
            "grow_provider_wallet",
            p.grow_provider_wallet(u.owner, payer, 16),
        ),
        ("prune_provider_wallet", p.prune_provider_wallet(u.owner)),
        (
            "close_provider_wallet",
            p.close_provider_wallet(u.owner, payer),
        ),
        (
            "reserve_list_booking",
            p.book_scoped(
                m,
                u,
                k("booking"),
                payer,
                &request,
                BookingMode::Reusable {
                    generation,
                    fresh: true,
                },
            )
            .unwrap(),
        ),
        (
            "claim_list_booking",
            p.claim(
                settlement,
                Usage {
                    uncached: 7,
                    cached: 0,
                    output: 20,
                },
                None,
            )
            .unwrap(),
        ),
        (
            "release_list_booking",
            p.release(settlement, Some(generation)).unwrap(),
        ),
        (
            "expire_list_booking",
            p.expire(settlement, Some(generation)).unwrap(),
        ),
    ];
    for (name, ix) in cases {
        let expected = rows.iter().find(|row| row["name"] == name).unwrap();
        assert_eq!(json!(ix.data), expected["data"], "{name} data");
        assert_eq!(json!(ix.accounts.iter().map(|meta|json!({"address":meta.pubkey.to_string(),"writable":meta.is_writable,"signer":meta.is_signer})).collect::<Vec<_>>()),expected["accounts"],"{name} accounts");
    }
    for fixture in rows.iter().filter(|row| row["layout"].is_string()) {
        let at = fixture["address"].as_str().unwrap().parse().unwrap();
        let raw = RawAccount {
            owner: p.program,
            lamports: 1,
            executable: false,
            data: serde_json::from_value(fixture["data"].clone()).unwrap(),
        };
        macro_rules! check {
            ($ty:ty) => {{
                assert!(
                    <$ty>::decode(&at, &raw, &p.program).is_ok(),
                    "{}",
                    fixture["layout"]
                );
                assert!(<$ty>::decode(&Pubkey::new_unique(), &raw, &p.program).is_err());
            }};
        }
        match fixture["layout"].as_str().unwrap() {
            "ProviderPolicy" => check!(ProviderPolicy),
            "ProviderModelCapacity" => check!(ProviderModelCapacity),
            "ListRouting" => check!(ListRouting),
            "ListTerms" => check!(ListTerms),
            "ListCapacity" => check!(ListCapacity),
            "ProviderWallet" => check!(ProviderWallet),
            _ => unreachable!(),
        }
    }
}

#[test]
fn scoped_selection_keeps_original_ties_and_falls_back_when_wallet_limit_is_full() {
    use fossa_sdk::accounts::{Offer, ProviderList};
    let k = |n| Pubkey::new_from_array([n; 32]);
    let limits = BookingLimits {
        wallet_window_seconds: 60,
        wallet_bookings_per_window: 5,
        wallet_booking_capacity: 2,
    };
    let offer = |provider, order| ListOffer {
        offer: Offer {
            provider_policy: u32::MAX,
            provider,
            payout: k(8),
            terms: OfferTerms {
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
                speed_tps: 32,
                private: false,
                active: true,
                capacity: 10,
            },
        },
        order,
    };
    let terms = ListTerms {
        model: k(4),
        list: k(5),
        capacity_address: k(6),
        limit: 2,
        next_order: 2,
        entries: vec![offer(k(1), 1), offer(k(2), 0)],
    };
    let mut capacity = ListCapacity {
        model: k(4),
        list: k(5),
        terms_address: k(7),
        limit: 2,
        entries: vec![
            ListCapacityEntry {
                provider: k(1),
                active: 0,
            },
            ListCapacityEntry {
                provider: k(2),
                active: 0,
            },
        ],
    };
    let list = ProviderList {
        market: k(3),
        authority: k(8),
        limit: 2,
        fee_bps: 10,
        protocol_share: 10_000,
        routing_enabled: true,
        providers: vec![k(1), k(2)],
    };
    let mut routing = ListRouting {
        market: k(3),
        list: k(5),
        capacity: 2,
        book_count: 1,
        entries: vec![
            PolicyMirror {
                provider: k(1),
                limits,
                epoch: 1,
                active: true,
            },
            PolicyMirror {
                provider: k(2),
                limits,
                epoch: 1,
                active: true,
            },
        ],
    };
    let request = BookRequest {
        minimum_wallet_capacity: None,
        min_context: 0,
        limits_configured: false,
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        model_id: k(9).to_bytes(),
        provider_list: k(5),
        rate_caps: Rates {
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
    };
    let choose =
        |capacity: &ListCapacity, routing: &ListRouting, wallet: Option<&ProviderWallet>, now| {
            request
                .select_scoped(&terms, capacity, &list, routing, wallet, 1000, now)
                .unwrap()
                .map(|selection| selection.slot)
        };
    assert_eq!(choose(&capacity, &routing, None, 100), Some(1));
    let mut wallet = ProviderWallet {
        market: k(3),
        owner: k(10),
        rent_payer: k(10),
        capacity: 1,
        entries: vec![ProviderUsage {
            provider: k(2),
            window_end: 160,
            bookings: 1,
            outstanding: 2,
        }],
    };
    assert_eq!(choose(&capacity, &routing, Some(&wallet), 100), Some(0));
    wallet.entries[0].outstanding = 0;
    wallet.entries[0].bookings = 5;
    assert_eq!(choose(&capacity, &routing, Some(&wallet), 100), Some(0));
    assert_eq!(choose(&capacity, &routing, Some(&wallet), 160), Some(1));
    routing.entries[1].active = false;
    assert_eq!(choose(&capacity, &routing, None, 160), Some(0));
    capacity.entries[0].active = 10;
    assert_eq!(choose(&capacity, &routing, None, 160), None);
    capacity.entries[0].provider = k(2);
    assert!(
        request
            .select_scoped(&terms, &capacity, &list, &routing, None, 1000, 160)
            .is_err()
    );
    capacity.entries[0].provider = k(1);
    capacity.entries[0].active = 9;
    routing.entries[0].limits.wallet_booking_capacity = 0;
    let mut demand = request;
    demand.minimum_wallet_capacity = Some(10);
    assert_eq!(
        demand
            .select_scoped(&terms, &capacity, &list, &routing, None, 1000, 160)
            .unwrap()
            .unwrap()
            .slot,
        0
    );
    routing.entries[0].limits.wallet_booking_capacity = 9;
    assert!(
        demand
            .select_scoped(&terms, &capacity, &list, &routing, None, 1000, 160)
            .unwrap()
            .is_none()
    );
    demand.minimum_wallet_capacity = Some(0);
    assert!(
        demand
            .select_scoped(&terms, &capacity, &list, &routing, None, 1000, 160)
            .is_err()
    );
}

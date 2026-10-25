use fossa_sdk::{
    accounts::*,
    address,
    amount::*,
    instruction::*,
    metadata::{Document, Kind},
    receipt::*,
    *,
};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn context() -> (Protocol, ModelKeys, UserKeys, OfferKeys, SettlementKeys) {
    let p = Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    };
    let m = ModelKeys {
        model: address::model(&p.program, &p.market, &[4; 32]).0,
        terms: key(5),
        capacity: key(6),
    };
    let u = UserKeys {
        user: key(7),
        owner: key(8),
        vault: key(9),
    };
    let o = OfferKeys {
        model: m,
        provider: key(10),
        payout: key(11),
        slot: 2,
    };
    (
        p,
        m,
        u,
        o,
        SettlementKeys {
            offer: o,
            user: u,
            booking: key(13),
        },
    )
}
fn terms() -> OfferTerms {
    OfferTerms {
        rates: Rates {
            input: 1_234_567,
            cached: 345_678,
            output: 9_876_543,
        },
        speed_tps: 300,
        private: true,
        active: true,
        capacity: 5,
    }
}
fn request() -> BookRequest {
    BookRequest {
        model_id: [4; 32],
        provider_list: key(30),
        rate_caps: Rates {
            input: u32::MAX,
            cached: u32::MAX,
            output: u32::MAX,
        },
        min_speed: 1,
        max_input: 100,
        max_output: 50,
        private: false,
        max_spend: 10_000,
        estimated_input: 20,
        estimated_cached: 5,
        estimated_output: 10,
        cache_provider: key(10),
    }
}
fn doc() -> DocumentRef {
    DocumentRef::new([42; 32], "https://example.com/m.json").unwrap()
}
fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
    d[at..at + bytes.len()].copy_from_slice(bytes);
}
fn raw(program: Pubkey, magic: &[u8; 8], len: usize) -> RawAccount {
    let mut data = vec![0; len];
    data[..8].copy_from_slice(magic);
    RawAccount {
        owner: program,
        lamports: 123,
        executable: false,
        data,
    }
}

#[test]
fn instruction_account_order_flags_and_payload_lengths_cover_every_opcode() {
    let (p, m, u, o, k) = context();
    let d = doc();
    let instructions = vec![
        (0, p.initialize_market(key(20)), 1, 3, vec![0], vec![0, 1]),
        (1, p.initialize_user(u), 1, 4, vec![0], vec![0, 1]),
        (
            2,
            p.register_offer(o, terms()).unwrap(),
            25,
            7,
            vec![2, 3],
            vec![5],
        ),
        (
            3,
            p.deposit(u, key(21), 1).unwrap(),
            9,
            7,
            vec![1, 3, 4],
            vec![2],
        ),
        (
            5,
            p.update_offer(o, terms()).unwrap(),
            25,
            4,
            vec![2],
            vec![3],
        ),
        (
            6,
            p.book(m, u, k.booking, key(22), &request(), BookingMode::OneShot)
                .unwrap(),
            108,
            9,
            vec![3, 4, 6],
            vec![5, 6, 7],
        ),
        (
            7,
            p.claim(k, Usage::default(), None).unwrap(),
            13,
            11,
            vec![2, 3, 4, 6, 7],
            vec![5],
        ),
        (8, p.expire(k, None).unwrap(), 1, 5, vec![2, 3, 4], vec![]),
        (
            10,
            p.withdraw(u, key(21), 1).unwrap(),
            9,
            8,
            vec![1, 3, 4],
            vec![2],
        ),
        (
            15,
            p.release(k, None).unwrap(),
            1,
            6,
            vec![2, 3, 4],
            vec![5],
        ),
        (
            17,
            p.close(k.booking, key(22), false),
            1,
            2,
            vec![0, 1],
            vec![],
        ),
        (
            18,
            p.initialize_model(InitializeModel {
                model: m,
                provider: o.provider,
                payer: key(22),
                id: [4; 32],
                limit: 8,
                document: &d,
            })
            .unwrap(),
            69 + d.uri.len(),
            9,
            vec![1, 2, 3, 5, 8],
            vec![2, 3, 4, 5],
        ),
        (
            19,
            p.initialize_provider(o.provider, key(22), &d).unwrap(),
            33 + d.uri.len(),
            5,
            vec![1, 3],
            vec![2, 3],
        ),
        (
            21,
            p.update_provider_metadata(o.provider, &d).unwrap(),
            33 + d.uri.len(),
            3,
            vec![1],
            vec![2],
        ),
        (
            22,
            p.update_model_metadata(m.model, o.provider, &d).unwrap(),
            33 + d.uri.len(),
            4,
            vec![2],
            vec![3],
        ),
        (
            26,
            p.initialize_reusable_booking(u, k.booking, key(22)),
            1,
            5,
            vec![3],
            vec![2, 3, 4],
        ),
        (
            28,
            p.initialize_list(key(30), u.owner, 256).unwrap(),
            5,
            3,
            vec![1],
            vec![1, 2],
        ),
        (
            29,
            p.update_list(key(30), u.owner, o.provider, true),
            34,
            3,
            vec![1],
            vec![2],
        ),
        (
            30,
            p.close_list(key(30), u.owner),
            1,
            2,
            vec![0, 1],
            vec![1],
        ),
        (
            32,
            p.grow_model(m, key(22), 85).unwrap(),
            5,
            6,
            vec![1, 2, 3, 4],
            vec![4],
        ),
    ];
    for (op, ix, len, count, writable, signers) in instructions {
        assert_eq!(ix.program_id, p.program);
        assert_eq!(ix.data[0], op);
        assert_eq!(ix.data.len(), len, "opcode {op}");
        assert_eq!(ix.accounts.len(), count, "opcode {op}");
        for (index, a) in ix.accounts.iter().enumerate() {
            assert_eq!(
                a.is_writable,
                writable.contains(&index),
                "opcode {op} writable {index}"
            );
            assert_eq!(
                a.is_signer,
                signers.contains(&index),
                "opcode {op} signer {index}"
            );
        }
        if op == 6 {
            assert_eq!(ix.accounts[8].pubkey, request().provider_list);
        }
        if op == 29 {
            assert_eq!(&ix.data[1..33], o.provider.as_ref());
            assert_eq!(ix.data[33], 1);
        }
        if op == 32 {
            assert_eq!(&ix.data[1..], &85u32.to_le_bytes());
            assert_eq!(
                ix.accounts.iter().map(|a| a.pubkey).collect::<Vec<_>>(),
                vec![
                    p.market,
                    m.model,
                    m.terms,
                    m.capacity,
                    key(22),
                    address::SYSTEM_PROGRAM
                ]
            );
        }
    }
    for limit in [0, 1, 4097, u32::MAX] {
        assert!(p.grow_model(m, key(22), limit).is_err());
    }
    assert!(p.grow_model(m, key(22), 4096).is_ok());
}

#[test]
fn reusable_variants_and_booking_fields_match_active_layout() {
    let (p, m, u, _, k) = context();
    let q = request();
    let ix = p
        .book(
            m,
            u,
            k.booking,
            key(22),
            &q,
            BookingMode::Reusable {
                generation: 7,
                fresh: false,
            },
        )
        .unwrap();
    assert_eq!(ix.data.len(), 116);
    assert!(!ix.accounts[6].is_signer);
    assert!(!ix.accounts[7].is_signer);
    assert_eq!(&ix.data[1..33], &q.model_id);
    assert_eq!(&ix.data[47..51], &100u32.to_le_bytes());
    assert_eq!(&ix.data[56..64], &10_000u64.to_le_bytes());
    assert_eq!(&ix.data[76..108], q.cache_provider.as_ref());
    assert_eq!(&ix.data[108..116], &7u64.to_le_bytes());
    let receipt = Receipt {
        owner: u.owner,
        user: u.user,
        booking: k.booking,
        generation: 7,
        charge: 89,
        claim_end: 456,
    };
    let ix = p
        .claim(
            k,
            Usage {
                uncached: 10,
                cached: 3,
                output: 4,
            },
            Some(&receipt),
        )
        .unwrap();
    assert_eq!(ix.data.len(), 141);
    assert_eq!(&ix.data[13..21], &7u64.to_le_bytes());
    assert_eq!(&ix.data[21..], &receipt.encode());
    let decoded = PublishedReceipt::from_claim(&p.program, &ix).unwrap();
    assert_eq!(decoded.receipt, receipt);
    let mut wrong = ix;
    wrong.data[13] = 8;
    assert!(PublishedReceipt::from_claim(&p.program, &wrong).is_err());
    assert_eq!(
        p.expire(k, Some(7)).unwrap().data,
        [8, 7, 0, 0, 0, 0, 0, 0, 0]
    );
    assert!(p.release(k, Some(0)).is_err());
    assert!(p.close(k.booking, key(22), true).accounts[1].is_signer);
}

#[test]
fn exact_atomic_math_rounding_boundaries_and_overflow() {
    assert_eq!(parse_usdc("18446744073709.551615").unwrap(), u64::MAX);
    for invalid in [
        "18446744073709.551616",
        "-1",
        "+1",
        "1e3",
        "1.0000001",
        "NaN",
        " 1",
        ".1",
    ] {
        assert!(parse_usdc(invalid).is_err(), "{invalid}");
    }
    assert_eq!(parse_usdc("0.000001").unwrap(), 1);
    assert_eq!(format_usdc(u64::MAX), "18446744073709.551615");
    assert_eq!(fee(1, 1), 1);
    assert_eq!(fee(0, u32::MAX), 0);
    assert_eq!(fee(u32::MAX, u32::MAX), 18_446_744_065_120);
    assert_eq!(
        Rates {
            input: 1,
            cached: 1,
            output: 1
        }
        .charge(Usage {
            uncached: 1,
            cached: 1,
            output: 1
        }),
        3
    );
    assert_eq!(
        Rates {
            input: 1,
            cached: 0,
            output: 1
        }
        .reserve(1, 1),
        3
    );
}

#[test]
fn matching_uses_list_full_caps_cache_identity_capacity_and_stable_ties() {
    let (_, m, _, _, _) = context();
    let t = OfferTerms {
        rates: Rates {
            input: 1_000_000,
            cached: 1,
            output: 1_000_000,
        },
        speed_tps: 10,
        private: true,
        active: true,
        capacity: 1,
    };
    let offer = |provider| Offer {
        provider,
        payout: key(11),
        terms: t,
    };
    let mut terms = TermsBook {
        model: m.model,
        limit: 8,
        entries: vec![offer(key(10)), offer(key(11))],
    };
    let cap = Capacity::default();
    let mut list = ProviderList {
        market: key(2),
        authority: key(8),
        limit: 8,
        providers: vec![key(10), key(11)],
    };
    let mut capacity = CapacityBook {
        model: m.model,
        limit: 8,
        entries: vec![cap, cap],
    };
    let mut q = request();
    assert_eq!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .unwrap()
            .slot,
        0
    );
    q.cache_provider = key(11);
    assert_eq!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .unwrap()
            .slot,
        1
    );
    q.estimated_cached = 0;
    assert_eq!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .unwrap()
            .slot,
        0
    );
    q.rate_caps.cached = 0;
    assert!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .is_none()
    );
    q.rate_caps.cached = u32::MAX;
    capacity.entries[0].active = 1;
    assert_eq!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .unwrap()
            .slot,
        1
    );
    assert_eq!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .unwrap()
            .reserve,
        151
    );
    assert!(q.select(&terms, &capacity, &list, 150).unwrap().is_none());
    list.providers = vec![key(10)];
    assert!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .is_none()
    );
    list.providers = vec![key(11)];
    terms.entries[1].terms.active = false;
    assert!(
        q.select(&terms, &capacity, &list, 10_000)
            .unwrap()
            .is_none()
    );
}

#[test]
fn metadata_commits_exact_bytes_and_rejects_bad_model_binding() {
    let bytes =
        b"{\n  \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\"\n}\n".to_vec();
    let d = Document::parse(Kind::Provider, bytes.clone()).unwrap();
    assert_eq!(d.bytes(), bytes);
    let compact = Document::parse(
        Kind::Provider,
        br#"{"name":"Example","endpoint":"https://example.com/infer"}"#.to_vec(),
    )
    .unwrap();
    assert_ne!(d.sha256(), compact.sha256());
    d.verify(&d.reference("https://example.com/a.json").unwrap())
        .unwrap();
    assert!(
        compact
            .verify(&d.reference("https://example.com/a.json").unwrap())
            .is_err()
    );
    assert!(Document::model("wrong",br#"{"id":"test","name":"Test","modalities":{"input":["text"],"output":["text"]},"contextTokens":100,"outputTokens":10}"#.to_vec()).is_err());
    for uri in [
        "",
        "https://example.com/a b",
        "https://example.com/\n",
        "https://example.com/\u{7f}",
    ] {
        assert!(DocumentRef::new([0; 32], uri).is_err());
    }
}

#[test]
fn account_decoders_validate_owner_layout_pdas_bindings_and_padding() {
    let (p, m, _, _, _) = context();
    let mut model = raw(p.program, Model::MAGIC, MODEL_LEN);
    put(&mut model.data, 8, p.market.as_ref());
    put(&mut model.data, 40, &[4; 32]);
    put(&mut model.data, 72, m.terms.as_ref());
    put(&mut model.data, 104, m.capacity.as_ref());
    put(&mut model.data, 136, &2u32.to_le_bytes());
    model.data[140] = address::model(&p.program, &p.market, &[4; 32]).1;
    let decoded = Model::decode(&m.model, &model, &p.program).unwrap();
    assert_eq!(decoded.limit, 2);
    assert!(Model::decode(&key(99), &model, &p.program).is_err());
    for len in 0..MODEL_LEN {
        let mut bad = model.clone();
        bad.data.truncate(len);
        assert!(Model::decode(&m.model, &bad, &p.program).is_err());
    }
    let mut wrong = model.clone();
    wrong.owner = key(99);
    assert!(Model::decode(&m.model, &wrong, &p.program).is_err());
    wrong = model.clone();
    wrong.data[140] ^= 1;
    assert!(Model::decode(&m.model, &wrong, &p.program).is_err());
    let authority = key(10);
    let (profile, bump) = address::provider(&p.program, &p.market, &authority);
    let mut account = raw(p.program, Provider::MAGIC, PROVIDER_LEN);
    put(&mut account.data, 8, p.market.as_ref());
    put(&mut account.data, 40, authority.as_ref());
    put(&mut account.data, 72, &[42; 32]);
    put(&mut account.data, 104, &1u16.to_le_bytes());
    account.data[106] = 1;
    account.data[107] = bump;
    account.data[108] = b'x';
    assert_eq!(
        Provider::decode(&profile, &account, &p.program)
            .unwrap()
            .authority,
        authority
    );
    account.data[109] = 1;
    assert!(Provider::decode(&profile, &account, &p.program).is_err());
    account.data[109] = 0;
    account.data[106] = 2;
    assert!(Provider::decode(&profile, &account, &p.program).is_ok());
    let mut capacity = raw(p.program, CapacityBook::MAGIC, 48 + 2 * 32);
    put(&mut capacity.data, 8, m.model.as_ref());
    put(&mut capacity.data, 40, &2u32.to_le_bytes());
    put(&mut capacity.data, 44, &1u32.to_le_bytes());
    put(&mut capacity.data, 64, &u32::MAX.to_le_bytes());
    assert_eq!(
        CapacityBook::decode(&m.capacity, &capacity, &p.program)
            .unwrap()
            .entries[0]
            .active,
        u32::MAX
    );
    put(&mut capacity.data, 44, &3u32.to_le_bytes());
    assert!(CapacityBook::decode(&m.capacity, &capacity, &p.program).is_err());
    let mut booking = raw(p.program, Booking::MAGIC, BOOKING_LEN);
    booking.data[148] = 5;
    assert_eq!(
        Booking::decode(&key(13), &booking, &p.program)
            .unwrap()
            .status,
        BookingStatus::Available
    );
    booking.data[148] = 6;
    assert!(Booking::decode(&key(13), &booking, &p.program).is_err());
}

#[test]
fn provider_lists_validate_size_order_duplicates_and_account_owner() {
    let (p, _, _, _, _) = context();
    let mut account = raw(p.program, ProviderList::MAGIC, list_size(3).unwrap());
    put(&mut account.data, 8, p.market.as_ref());
    put(&mut account.data, 40, key(8).as_ref());
    put(&mut account.data, 72, &3u32.to_le_bytes());
    put(&mut account.data, 76, &2u32.to_le_bytes());
    put(&mut account.data, 80, key(10).as_ref());
    put(&mut account.data, 112, key(11).as_ref());
    let tree = LIST_HEADER_LEN + 3 * 32;
    put(&mut account.data, tree, key(11).as_ref());
    put(&mut account.data, tree + 32, key(10).as_ref());
    let list = ProviderList::decode(&key(30), &account, &p.program).unwrap();
    assert!(list.contains(&key(10)));
    assert!(!list.contains(&key(12)));
    assert_eq!(
        serde_json::to_value(&list).unwrap()["providers"][0],
        key(10).to_string()
    );
    for offset in [72, 76] {
        let mut bad = account.clone();
        put(&mut bad.data, offset, &4097u32.to_le_bytes());
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for provider in [key(9), key(10)] {
        let mut bad = account.clone();
        put(&mut bad.data, 112, provider.as_ref());
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for at in [144, tree + 64] {
        let mut bad = account.clone();
        bad.data[at] = 1;
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for slot_len in [32, 40] {
        let mut short = account.clone();
        short.data.truncate(LIST_HEADER_LEN + 3 * slot_len);
        assert!(ProviderList::decode(&key(30), &short, &p.program).is_err());
    }
    account.owner = key(99);
    assert!(ProviderList::decode(&key(30), &account, &p.program).is_err());
    assert!(list_size(0).is_err());
    assert!(list_size(4097).is_err());
    assert_eq!(list_size(256).unwrap(), 16464);
    assert_eq!(list_size(4096).unwrap(), 262224);
}

#[test]
fn invalid_builders_fail_before_rpc_or_signing() {
    let (p, m, u, o, k) = context();
    assert!(p.deposit(u, key(21), 0).is_err());
    assert!(
        p.register_offer(
            o,
            OfferTerms {
                speed_tps: 0,
                ..terms()
            }
        )
        .is_err()
    );
    assert!(
        p.update_offer(
            o,
            OfferTerms {
                rates: Rates {
                    input: 1,
                    cached: 2,
                    output: 1
                },
                ..terms()
            }
        )
        .is_err()
    );
    let mut q = request();
    q.estimated_cached = 21;
    assert!(
        p.book(m, u, k.booking, key(22), &q, BookingMode::OneShot)
            .is_err()
    );
    q = request();
    q.model_id = [0; 32];
    assert!(
        p.book(m, u, k.booking, key(22), &q, BookingMode::OneShot)
            .is_err()
    );
    assert!(book_sizes(0).is_err());
    assert!(book_sizes(4097).is_err());
}

#[test]
fn pda_vectors_match_independent_web3_derivation() {
    assert_eq!(
        address::provider(&key(1), &key(2), &key(3)).0.to_string(),
        "Gt4b1itRCfTy9dLS4Sy8erP7x2jsiUQvKQm8m5xL97WW"
    );
    assert_eq!(
        address::model(&key(1), &key(2), &[4; 32]).0.to_string(),
        "7A8Fqk1FXhwkh93nk1R6CjzhBo9k7dKLqATEpUJY8rSW"
    );
    assert_eq!(
        address::vault(&key(1), &key(3)).0.to_string(),
        "J2FbwMp5fGYX6Zng12uULs2PFoeDjn4EiXyyPwrENBFt"
    );
}

#[test]
fn remaining_accounts_decode_exact_fields_and_reject_malformed_records() {
    let (p, m, u, _, _) = context();
    let mut market = raw(p.program, Market::MAGIC, MARKET_LEN);
    put(&mut market.data, 8, key(10).as_ref());
    put(&mut market.data, 40, p.mint.as_ref());
    assert_eq!(
        Market::decode(&p.market, &market, &p.program).unwrap().mint,
        p.mint
    );
    let mut user = raw(p.program, User::MAGIC, USER_LEN);
    put(&mut user.data, 8, u.owner.as_ref());
    put(&mut user.data, 40, p.market.as_ref());
    put(&mut user.data, 72, &u64::MAX.to_le_bytes());
    put(&mut user.data, 80, u.vault.as_ref());
    user.data[112] = address::vault(&p.program, &u.user).1;
    user.data[113..120].fill(255);
    let decoded = User::decode(&u.user, &user, &p.program).unwrap();
    assert_eq!(decoded.generation, (1 << 56) - 1);
    assert_eq!(decoded.free, u64::MAX);
    user.data[112] ^= 1;
    assert!(User::decode(&u.user, &user, &p.program).is_err());
    let (metadata, bump) = address::model_metadata(&p.program, &m.model);
    let mut account = raw(p.program, ModelMetadata::MAGIC, MODEL_METADATA_LEN);
    put(&mut account.data, 8, p.market.as_ref());
    put(&mut account.data, 40, m.model.as_ref());
    put(&mut account.data, 72, key(10).as_ref());
    put(&mut account.data, 104, &[42; 32]);
    put(&mut account.data, 136, &1u16.to_le_bytes());
    account.data[138] = bump;
    account.data[140] = b'x';
    assert_eq!(
        ModelMetadata::decode(&metadata, &account, &p.program)
            .unwrap()
            .document
            .sha256,
        [42; 32]
    );
    account.data[138] ^= 1;
    assert!(ModelMetadata::decode(&metadata, &account, &p.program).is_err());
    let mut terms_account = raw(p.program, TermsBook::MAGIC, HEADER_LEN + TERMS_LEN);
    put(&mut terms_account.data, 8, m.model.as_ref());
    put(&mut terms_account.data, 40, &1u32.to_le_bytes());
    put(&mut terms_account.data, 44, &1u32.to_le_bytes());
    put(&mut terms_account.data, 48, key(10).as_ref());
    put(&mut terms_account.data, 80, key(11).as_ref());
    put(&mut terms_account.data, 112, key(12).as_ref());
    let bytes = terms().encode().unwrap();
    put(&mut terms_account.data, 144, &bytes[..16]);
    terms_account.data[160] = 1;
    put(&mut terms_account.data, 164, &bytes[16..]);
    assert_eq!(
        TermsBook::decode(&m.terms, &terms_account, &p.program)
            .unwrap()
            .entries[0]
            .terms,
        terms()
    );
    terms_account.data[158] = 2;
    assert!(TermsBook::decode(&m.terms, &terms_account, &p.program).is_err());
    let mut token = RawAccount {
        owner: address::TOKEN_PROGRAM,
        lamports: 1,
        executable: false,
        data: vec![0; 165],
    };
    put(&mut token.data, 0, p.mint.as_ref());
    put(&mut token.data, 32, u.owner.as_ref());
    put(&mut token.data, 64, &u64::MAX.to_le_bytes());
    token.data[108] = 1;
    TokenAccount::decode(&token)
        .unwrap()
        .validate(&p.mint, &u.owner)
        .unwrap();
    token.data[108] = 2;
    assert!(
        TokenAccount::decode(&token)
            .unwrap()
            .validate(&p.mint, &u.owner)
            .is_err()
    );
}

#[test]
fn metadata_uri_matches_indexer_rules_and_preserves_public_queries() {
    let uri = "https://example.com/provider.json?v=2";
    assert_eq!(
        fossa_sdk::metadata::validate_uri(uri).unwrap().as_str(),
        uri
    );
    for uri in [
        "http://example.com/p.json",
        "https://127.0.0.1/p.json",
        "https://[::1]/p.json",
        "https://localhost/p.json",
        "https://provider.local/p.json",
        "https://example.com:8443/p.json",
        "https://user:password@example.com/p.json",
        "https://example.com/p.json#v2",
    ] {
        assert!(fossa_sdk::metadata::validate_uri(uri).is_err(), "{uri}");
    }
    assert_eq!(
        fossa_sdk::hosted::validate_api_origin("https://api.example.com/").unwrap(),
        "https://api.example.com"
    );
    for origin in [
        "https://user:password@api.example.com",
        "https://api.example.com?key=private",
        "https://api.example.com/path",
    ] {
        assert!(fossa_sdk::hosted::validate_api_origin(origin).is_err());
    }
}

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
        group: Pubkey::default(),
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
            fee_destinations: None,
        },
    )
}
fn terms() -> OfferTerms {
    OfferTerms {
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        context_window: 1_000_000,
        max_input: 1_000_000,
        max_output: 1_000_000,
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
        min_context: 0,
        limits_configured: false,
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
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
        (
            "initialize_market",
            p.initialize_market(key(20)),
            8,
            4,
            vec![0, 2],
            vec![0, 2],
        ),
        (
            "initialize_user",
            p.initialize_user(u),
            8,
            6,
            vec![0, 2],
            vec![0, 2],
        ),
        (
            "register_offer",
            p.register_offer(o, terms()).unwrap(),
            78,
            7,
            vec![2, 3],
            vec![5],
        ),
        (
            "deposit",
            p.deposit(u, key(21), 1).unwrap(),
            16,
            8,
            vec![1, 3, 5],
            vec![2],
        ),
        (
            "update_offer",
            p.update_offer(o, terms()).unwrap(),
            78,
            4,
            vec![2],
            vec![3],
        ),
        (
            "reserve_booking",
            p.book(m, u, k.booking, key(22), &request(), BookingMode::OneShot)
                .unwrap(),
            155,
            10,
            vec![3, 4, 6],
            vec![5],
        ),
        (
            "claim_booking",
            p.claim(k, Usage::default(), None).unwrap(),
            21,
            11,
            vec![2, 3, 4, 6, 7],
            vec![5],
        ),
        (
            "expire_booking",
            p.expire(k, None).unwrap(),
            9,
            5,
            vec![2, 3, 4],
            vec![],
        ),
        (
            "withdraw",
            p.withdraw(u, key(21), 1).unwrap(),
            16,
            8,
            vec![1, 3, 4],
            vec![2],
        ),
        (
            "release_booking",
            p.release(k, None).unwrap(),
            9,
            6,
            vec![2, 3, 4],
            vec![5],
        ),
        (
            "close_booking",
            p.close(k.booking, key(22), false),
            8,
            2,
            vec![0, 1],
            vec![],
        ),
        (
            "initialize_model",
            p.initialize_model(InitializeModel {
                model: m,
                provider: o.provider,
                payer: key(22),
                id: [4; 32],
                limit: 8,
                document: &d,
            })
            .unwrap(),
            80 + d.uri.len(),
            9,
            vec![2, 4, 5, 6, 7],
            vec![1, 2, 5, 6],
        ),
        (
            "initialize_provider",
            p.initialize_provider(o.provider, key(22), &d).unwrap(),
            44 + d.uri.len(),
            5,
            vec![2, 3],
            vec![1, 2],
        ),
        (
            "update_profile",
            p.update_provider_metadata(o.provider, &d).unwrap(),
            44 + d.uri.len(),
            3,
            vec![1],
            vec![2],
        ),
        (
            "update_model_metadata",
            p.update_model_metadata(m.model, o.provider, &d).unwrap(),
            44 + d.uri.len(),
            4,
            vec![2],
            vec![3],
        ),
        (
            "initialize_receipt",
            p.initialize_reusable_booking(u, k.booking, key(22)),
            8,
            6,
            vec![3, 4],
            vec![2, 3, 4],
        ),
        (
            "initialize_list",
            p.initialize_list(
                key(30),
                u.owner,
                ListCreation {
                    limit: 256,
                    fee_bps: 10,
                    protocol_share: 5000,
                    maximum_payment: 1,
                    source: key(31),
                    destination: key(32),
                    mint: key(33),
                    token_program: address::TOKEN_PROGRAM,
                },
            )
            .unwrap(),
            24,
            8,
            vec![1, 2, 4, 5],
            vec![1, 2],
        ),
        (
            "update_list",
            p.update_list(key(30), u.owner, o.provider, true),
            41,
            3,
            vec![1],
            vec![2],
        ),
        (
            "close_list",
            p.close_list(key(30), u.owner),
            8,
            3,
            vec![0, 1, 2],
            vec![1],
        ),
        (
            "grow_books",
            p.grow_model(m, key(22), 77).unwrap(),
            12,
            6,
            vec![1, 2, 3, 4],
            vec![4],
        ),
        (
            "initialize_list_metadata",
            p.initialize_list_metadata(key(30), u.owner, key(22), &d)
                .unwrap(),
            44 + d.uri.len(),
            6,
            vec![3, 4],
            vec![2, 3],
        ),
        (
            "update_list_metadata",
            p.update_list_metadata(key(30), u.owner, &d).unwrap(),
            44 + d.uri.len(),
            4,
            vec![2],
            vec![3],
        ),
    ];
    for (name, ix, len, count, writable, signers) in instructions {
        use sha2::{Digest, Sha256};
        let expected = Sha256::digest(format!("global:{name}").as_bytes());
        assert_eq!(ix.program_id, p.program);
        assert_eq!(&ix.data[..8], &expected[..8]);
        assert_eq!(ix.data.len(), len, "{name}");
        assert_eq!(ix.accounts.len(), count, "{name}");
        for (index, account) in ix.accounts.iter().enumerate() {
            assert_eq!(
                account.is_writable,
                writable.contains(&index),
                "{name} writable {index}"
            );
            assert_eq!(
                account.is_signer,
                signers.contains(&index),
                "{name} signer {index}"
            );
        }
        if name == "reserve_booking" {
            assert_eq!(ix.accounts[8].pubkey, request().provider_list);
            assert_eq!(ix.accounts[9].pubkey, p.program);
        }
        if name == "update_list" {
            assert_eq!(&ix.data[8..40], o.provider.as_ref());
            assert_eq!(ix.data[40], 1);
        }
        if matches!(name, "initialize_list_metadata" | "update_list_metadata") {
            assert_eq!(&ix.data[8..40], &d.sha256);
            assert_eq!(&ix.data[40..44], &(d.uri.len() as u32).to_le_bytes());
            assert_eq!(&ix.data[44..], d.uri.as_bytes());
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
    assert_eq!(ix.data.len(), 163);
    assert!(!ix.accounts[6].is_signer);
    assert!(!ix.accounts[7].is_signer);
    assert_eq!(&ix.data[8..40], &q.model_id);
    assert_eq!(&ix.data[54..58], &100u32.to_le_bytes());
    assert_eq!(&ix.data[63..71], &10_000u64.to_le_bytes());
    assert_eq!(&ix.data[83..115], q.cache_provider.as_ref());
    assert_eq!(&ix.data[155..163], &7u64.to_le_bytes());
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
    assert_eq!(ix.data[20], 1);
    assert_eq!(&ix.data[21..], &receipt.encode());
    let decoded = PublishedReceipt::from_claim(&p.program, &ix).unwrap();
    assert_eq!(decoded.receipt, receipt);
    let mut wrong = ix;
    wrong.data[20] = 2;
    assert!(PublishedReceipt::from_claim(&p.program, &wrong).is_err());
    assert_eq!(
        p.expire(k, Some(7)).unwrap().data,
        [
            discriminator("expire_booking").as_slice(),
            &[1],
            &7u64.to_le_bytes()
        ]
        .concat()
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
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        context_window: 1_000_000,
        max_input: 1_000_000,
        max_output: 1_000_000,
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
        fee_bps: 10,
        protocol_share: 10_000,
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
        152
    );
    assert!(q.select(&terms, &capacity, &list, 151).unwrap().is_none());
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
    put(&mut account.data, 76, &10u16.to_le_bytes());
    put(&mut account.data, 78, &10_000u16.to_le_bytes());
    put(&mut account.data, 84, &2u32.to_le_bytes());
    put(&mut account.data, 88, key(10).as_ref());
    put(&mut account.data, 120, key(11).as_ref());
    let list = ProviderList::decode(&key(30), &account, &p.program).unwrap();
    assert!(list.contains(&key(10)));
    assert!(!list.contains(&key(12)));
    assert_eq!(
        serde_json::to_value(&list).unwrap()["providers"][0],
        key(10).to_string()
    );
    for offset in [72, 84] {
        let mut bad = account.clone();
        put(&mut bad.data, offset, &4097u32.to_le_bytes());
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for provider in [key(9), key(10)] {
        let mut bad = account.clone();
        put(&mut bad.data, 120, provider.as_ref());
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for at in [152, 183] {
        let mut bad = account.clone();
        bad.data[at] = 1;
        assert!(ProviderList::decode(&key(30), &bad, &p.program).is_err());
    }
    for slot_len in [16, 24] {
        let mut short = account.clone();
        short.data.truncate(LIST_HEADER_LEN + 3 * slot_len);
        assert!(ProviderList::decode(&key(30), &short, &p.program).is_err());
    }
    account.owner = key(99);
    assert!(ProviderList::decode(&key(30), &account, &p.program).is_err());
    assert!(list_size(0).is_err());
    assert!(list_size(4097).is_err());
    assert_eq!(list_size(256).unwrap(), 8280);
    assert_eq!(list_size(4096).unwrap(), 131160);
}

#[test]
fn list_metadata_decodes_exact_reference_and_rejects_wrong_pda_owner_padding_and_version() {
    let (p, _, _, _, _) = context();
    let list = key(30);
    let (at, bump) = address::list_metadata(&p.program, &list);
    let reference = doc();
    let mut account = raw(p.program, ListMetadata::MAGIC, LIST_METADATA_LEN);
    for (offset, bytes) in [
        (8, p.market.as_ref()),
        (40, list.as_ref()),
        (72, key(8).as_ref()),
        (104, reference.sha256.as_slice()),
        (140, reference.uri.as_bytes()),
    ] {
        put(&mut account.data, offset, bytes);
    }
    put(
        &mut account.data,
        136,
        &(reference.uri.len() as u16).to_le_bytes(),
    );
    account.data[138] = bump;
    let state = ListMetadata::decode(&at, &account, &p.program).unwrap();
    assert_eq!(
        (state.market, state.list, state.authority, state.document),
        (p.market, list, key(8), reference)
    );
    for offset in [0, 40, 138, 139, 395] {
        let mut bad = account.clone();
        bad.data[offset] ^= 1;
        assert!(ListMetadata::decode(&at, &bad, &p.program).is_err());
    }
    assert!(ListMetadata::decode(&key(99), &account, &p.program).is_err());
    account.owner = key(99);
    assert!(ListMetadata::decode(&at, &account, &p.program).is_err());
    let bytes = br##"{ "name":"Example Group", "accentColor":"#A1b2C3", "icon":"https://example.com/icon.svg" }"##;
    let document = Document::parse(Kind::List, bytes.to_vec()).unwrap();
    assert_eq!(document.bytes(), bytes);
    assert!(
        Document::parse(
            Kind::List,
            br##"{"name":"Group","accentColor":"#fff","privilege":true}"##.to_vec()
        )
        .is_err()
    );
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
                checkpoint: [7; 32],
                input_modalities: 1,
                output_modalities: 1,
                context_window: 1_000_000,
                max_input: 1_000_000,
                max_output: 1_000_000,
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
    user.data[120] = address::vault(&p.program, &u.user).1;
    user.data[112..120].fill(255);
    let decoded = User::decode(&u.user, &user, &p.program).unwrap();
    assert_eq!(decoded.generation, u64::MAX);
    assert_eq!(decoded.free, u64::MAX);
    user.data[120] ^= 1;
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
    put(&mut terms_account.data, 112, &terms().checkpoint);
    let bytes = terms().encode().unwrap();
    put(&mut terms_account.data, 144, &bytes[..16]);
    terms_account.data[160] = 1;
    terms_account.data[161] = 1;
    terms_account.data[162] = 1;
    for at in [168, 172, 176] {
        put(&mut terms_account.data, at, &1_000_000u32.to_le_bytes());
    }
    put(&mut terms_account.data, 164, &bytes[16..20]);
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

#[test]
fn list_surcharge_matches_reserve_charge_and_snapshotted_destinations() {
    let (protocol, model, user, _, settlement) = context();
    let mut raw = raw(protocol.program, Booking::MAGIC, BOOKING_LEN);
    for (at, value) in [
        (8, user.owner),
        (40, key(10)),
        (72, protocol.market),
        (192, user.user),
        (224, model.model),
        (288, key(11)),
        (368, key(20)),
        (400, key(21)),
    ] {
        put(&mut raw.data, at, value.as_ref());
    }
    put(&mut raw.data, 104, &153u64.to_le_bytes());
    for at in [112, 116, 120] {
        put(&mut raw.data, at, &1_000_000u32.to_le_bytes());
    }
    put(&mut raw.data, 124, &100u32.to_le_bytes());
    put(&mut raw.data, 128, &50u32.to_le_bytes());
    put(&mut raw.data, 152, &u64::MAX.to_le_bytes());
    put(&mut raw.data, 464, &100u16.to_le_bytes());
    put(&mut raw.data, 466, &10_000u16.to_le_bytes());
    let booking = Booking::decode(&settlement.booking, &raw, &protocol.program).unwrap();
    let usage = Usage {
        uncached: 100,
        cached: 0,
        output: 50,
    };
    let receipt = Receipt::from_booking(settlement.booking, &booking, usage).unwrap();
    assert_eq!(receipt.charge, 152);
    assert_eq!(
        with_list_fee(booking.rates.reserve(100, 50), 100).unwrap(),
        153
    );
    let expected = [
        address::associated_token(&protocol.mint, &key(20)),
        address::associated_token(&protocol.mint, &key(21)),
    ];
    assert_eq!(booking.fee_destinations(&protocol.mint), Some(expected));
    let keys =
        SettlementKeys::from_booking(settlement.booking, &booking, model, user, &protocol.mint)
            .unwrap();
    let claim = protocol.claim(keys, usage, Some(&receipt)).unwrap();
    assert_eq!(claim.accounts.len(), 13);
    assert_eq!(
        claim.accounts[11..]
            .iter()
            .map(|a| a.pubkey)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        PublishedReceipt::from_claim(&protocol.program, &claim)
            .unwrap()
            .receipt,
        receipt
    );
    let mut backing = booking;
    backing.reserve_destination = true;
    assert_eq!(
        backing.fee_destinations(&protocol.mint).unwrap()[1],
        key(21)
    );
    assert!(with_list_fee(u64::MAX, 100).is_err());
}

#[test]
fn historical_receipts_remain_decodable_after_native_upgrade() {
    let (protocol, _, user, _, keys) = context();
    let receipt = Receipt {
        owner: user.owner,
        user: user.user,
        booking: keys.booking,
        generation: 7,
        charge: 11,
        claim_end: 123,
    };
    let usage = Usage {
        uncached: 3,
        cached: 2,
        output: 1,
    };
    let mut old = protocol.claim(keys, usage, Some(&receipt)).unwrap();
    old.data = [
        vec![7],
        3u32.to_le_bytes().to_vec(),
        2u32.to_le_bytes().to_vec(),
        1u32.to_le_bytes().to_vec(),
        7u64.to_le_bytes().to_vec(),
        receipt.encode().to_vec(),
    ]
    .concat();
    let decoded = PublishedReceipt::from_claim(&protocol.program, &old).unwrap();
    assert_eq!(decoded.receipt, receipt);
    assert_eq!(decoded.usage, usage);
    old.data[13] = 8;
    assert!(PublishedReceipt::from_claim(&protocol.program, &old).is_err());
}

#[test]
fn historical_bucket_receipts_keep_generation_and_index_bounds() {
    let (protocol, _, user, _, keys) = context();
    let mut receipt = Receipt {
        owner: user.owner,
        user: user.user,
        booking: keys.booking,
        generation: 7,
        charge: 11,
        claim_end: 123,
    };
    let usage = Usage {
        uncached: 3,
        cached: 2,
        output: 1,
    };
    let mut old = protocol.claim(keys, usage, Some(&receipt)).unwrap();
    old.accounts
        .push(fossa_sdk::AccountMeta::new(key(99), false));
    let legacy = |receipt: &Receipt, index: u64| {
        [
            vec![7],
            3u32.to_le_bytes().to_vec(),
            2u32.to_le_bytes().to_vec(),
            1u32.to_le_bytes().to_vec(),
            receipt.generation.to_le_bytes().to_vec(),
            42u64.to_le_bytes().to_vec(),
            index.to_le_bytes().to_vec(),
            receipt.encode().to_vec(),
        ]
        .concat()
    };
    old.data = legacy(&receipt, 9);
    assert_eq!(
        PublishedReceipt::from_claim(&protocol.program, &old)
            .unwrap()
            .receipt,
        receipt
    );
    old.data = legacy(&receipt, 65535);
    assert!(PublishedReceipt::from_claim(&protocol.program, &old).is_err());
    receipt.generation = 1u64 << 56;
    old.data = legacy(&receipt, 9);
    assert!(PublishedReceipt::from_claim(&protocol.program, &old).is_err());
    old.accounts.pop();
    old.data.drain(21..37);
    assert!(PublishedReceipt::from_claim(&protocol.program, &old).is_err());
}

#[test]
fn matching_applies_exact_checkpoint_modalities_and_independent_token_limits() {
    let (_, model, _, _, _) = context();
    let request = request();
    let offer = Offer {
        provider: key(10),
        payout: key(11),
        terms: terms(),
    };
    let mut book = TermsBook {
        model: model.model,
        limit: 1,
        entries: vec![offer],
    };
    let capacity = CapacityBook {
        model: model.model,
        limit: 1,
        entries: vec![Capacity::default()],
    };
    let list = ProviderList {
        market: key(2),
        authority: key(3),
        limit: 1,
        fee_bps: 100,
        protocol_share: 10_000,
        providers: vec![key(10)],
    };
    assert!(
        request
            .select(&book, &capacity, &list, u64::MAX)
            .unwrap()
            .is_some()
    );
    let original = book.entries[0].terms;
    let changes: &[fn(&mut OfferTerms)] = &[
        |offer| offer.checkpoint = [8; 32],
        |offer| offer.input_modalities = 2,
        |offer| offer.output_modalities = 2,
        |offer| offer.max_input = 99,
        |offer| offer.max_output = 49,
        |offer| offer.context_window = 149,
    ];
    for change in changes {
        book.entries[0].terms = original;
        change(&mut book.entries[0].terms);
        assert!(
            request
                .select(&book, &capacity, &list, u64::MAX)
                .unwrap()
                .is_none()
        );
    }
    book.entries[0].terms = original;
    let mut constrained = request.clone();
    constrained.min_context = original.context_window + 1;
    constrained.limits_configured = true;
    assert!(
        constrained
            .select(&book, &capacity, &list, u64::MAX)
            .unwrap()
            .is_none()
    );
    constrained.limits_configured = false;
    assert!(constrained.validate().is_err());
    constrained = request.clone();
    constrained.input_modalities = 16;
    assert!(constrained.validate().is_err());
    constrained = request;
    constrained.checkpoint = [0; 32];
    assert!(constrained.validate().is_err());
}

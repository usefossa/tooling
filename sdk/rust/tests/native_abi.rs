use base64::{Engine, engine::general_purpose::STANDARD};
use fossa_sdk::{
    Pubkey,
    accounts::{self, Decode, DocumentRef, OfferTerms},
    amount::{Rates, Usage},
    instruction::*,
    receipt::Receipt,
};
use serde_json::{Value, json};

#[test]
fn builders_match_anchor_generated_instruction_data_and_account_metas() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/native-abi.json")).unwrap();
    let key = |name: &str| {
        fixture["keys"][name]
            .as_str()
            .unwrap()
            .parse::<Pubkey>()
            .unwrap()
    };
    let protocol = Protocol {
        program: fixture["program"].as_str().unwrap().parse().unwrap(),
        market: key("market"),
        mint: key("mint"),
    };
    let model = ModelKeys {
        model: key("model"),
        terms: key("terms"),
        capacity: key("capacity"),
        group: Pubkey::default(),
    };
    let user = UserKeys {
        user: key("user"),
        owner: key("owner"),
        vault: key("vault"),
    };
    let offer = OfferKeys {
        model,
        provider: key("provider"),
        payout: key("payout"),
        slot: 0,
    };
    let settlement = SettlementKeys {
        offer,
        user,
        booking: key("booking"),
        fee_destinations: None,
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
    let document = DocumentRef::new([29; 32], "https://fixture.invalid/model.json").unwrap();
    let generation = fixture["generation"].as_str().unwrap().parse().unwrap();
    let request = BookRequest {
        min_context: 0,
        limits_configured: false,
        checkpoint: [7; 32],
        input_modalities: 1,
        output_modalities: 1,
        model_id: key("model_id").to_bytes(),
        provider_list: key("list"),
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
        cache_provider: key("provider"),
    };
    let receipt = Receipt {
        owner: user.owner,
        user: user.user,
        booking: settlement.booking,
        generation,
        charge: 43,
        claim_end: 1_793_420_000,
    };
    let cases = [
        ("initialize_market", protocol.initialize_market(user.owner)),
        ("initialize_user", protocol.initialize_user(user)),
        (
            "initialize_provider",
            protocol
                .initialize_provider(offer.provider, key("payer"), &document)
                .unwrap(),
        ),
        (
            "initialize_model",
            protocol
                .initialize_model(InitializeModel {
                    model,
                    provider: offer.provider,
                    payer: key("payer"),
                    id: request.model_id,
                    limit: 4,
                    document: &document,
                })
                .unwrap(),
        ),
        (
            "register_offer",
            protocol.register_offer(offer, terms).unwrap(),
        ),
        ("update_offer", protocol.update_offer(offer, terms).unwrap()),
        (
            "deposit",
            protocol.deposit(user, key("source"), 123_456).unwrap(),
        ),
        (
            "withdraw",
            protocol
                .withdraw(user, key("destination"), 123_456)
                .unwrap(),
        ),
        (
            "initialize_receipt",
            protocol.initialize_reusable_booking(user, settlement.booking, key("payer")),
        ),
        (
            "reserve_booking",
            protocol
                .book(
                    model,
                    user,
                    settlement.booking,
                    key("payer"),
                    &request,
                    BookingMode::Reusable {
                        generation,
                        fresh: false,
                    },
                )
                .unwrap(),
        ),
        (
            "claim_booking",
            protocol
                .claim(
                    settlement,
                    Usage {
                        uncached: 7,
                        cached: 3,
                        output: 20,
                    },
                    Some(&receipt),
                )
                .unwrap(),
        ),
        (
            "expire_booking",
            protocol.expire(settlement, Some(generation)).unwrap(),
        ),
        (
            "release_booking",
            protocol.release(settlement, Some(generation)).unwrap(),
        ),
        (
            "close_booking",
            protocol.close(settlement.booking, key("payer"), false),
        ),
        (
            "update_profile",
            protocol
                .update_provider_metadata(offer.provider, &document)
                .unwrap(),
        ),
        (
            "update_model_metadata",
            protocol
                .update_model_metadata(model.model, offer.provider, &document)
                .unwrap(),
        ),
        (
            "initialize_list_metadata",
            protocol
                .initialize_list_metadata(key("list"), user.owner, key("payer"), &document)
                .unwrap(),
        ),
        (
            "update_list_metadata",
            protocol
                .update_list_metadata(key("list"), user.owner, &document)
                .unwrap(),
        ),
        (
            "update_list",
            protocol.update_list(key("list"), user.owner, offer.provider, true),
        ),
        ("close_list", protocol.close_list(key("list"), user.owner)),
        (
            "grow_books",
            protocol.grow_model(model, key("payer"), 8).unwrap(),
        ),
    ];
    for (name, instruction) in cases {
        let expected = &fixture["cases"][name];
        assert_eq!(
            STANDARD.encode(&instruction.data),
            expected["dataBase64"],
            "{name} data"
        );
        assert_eq!(json!(instruction.accounts.iter().map(|account| json!({ "address": account.pubkey.to_string(), "writable": account.is_writable, "signer": account.is_signer })).collect::<Vec<_>>()), expected["accounts"], "{name} accounts");
    }
    for (name, discriminator, length) in [
        ("Market", accounts::Market::MAGIC, accounts::MARKET_LEN),
        ("Model", accounts::Model::MAGIC, accounts::MODEL_LEN),
        (
            "Provider",
            accounts::Provider::MAGIC,
            accounts::PROVIDER_LEN,
        ),
        ("User", accounts::User::MAGIC, accounts::USER_LEN),
        ("Booking", accounts::Booking::MAGIC, accounts::BOOKING_LEN),
        (
            "List",
            accounts::ProviderList::MAGIC,
            accounts::LIST_HEADER_LEN - 4,
        ),
        (
            "Terms",
            accounts::TermsBook::MAGIC,
            accounts::HEADER_LEN - 4,
        ),
        (
            "Capacity",
            accounts::CapacityBook::MAGIC,
            accounts::HEADER_LEN - 4,
        ),
        (
            "ModelMetadata",
            accounts::ModelMetadata::MAGIC,
            accounts::MODEL_METADATA_LEN,
        ),
        (
            "ListMetadata",
            accounts::ListMetadata::MAGIC,
            accounts::LIST_METADATA_LEN,
        ),
    ] {
        assert_eq!(
            STANDARD.encode(discriminator),
            fixture["layouts"][name]["discriminatorBase64"],
            "{name}"
        );
        assert_eq!(json!(length), fixture["layouts"][name]["bytes"], "{name}");
    }
}

use crate::{
    Error, Result,
    args::*,
    config::Config,
    runtime::{self, Runtime},
};
use fossa_sdk::{
    Keypair, PendingTransaction, Pubkey, Signer,
    accounts::*,
    address,
    amount::{self, Rates, Usage},
    instruction::{self, ModelKeys, OfferKeys, SettlementKeys, UserKeys},
    metadata::{Document, Kind},
    onboarding,
};
use serde_json::{Value, json};
use std::{io::Read, path::Path, time::Duration};

mod accounts;
mod bookings;
mod lists;
mod metadata;
mod model_markets;
mod models;
mod profiles;
mod registration;
pub mod routing;

use accounts::*;
use bookings::*;
use lists::*;
use metadata::*;
use models::*;
use profiles::*;
use registration::*;

pub async fn run(cli: Cli) -> Result<Value> {
    preflight(&cli.command)?;
    if let Command::Metadata(MetadataCommand::Validate { kind, file }) = &cli.command {
        return Ok(document_view(&document(*kind, file)?));
    }
    let config = Config::load(&cli.options)?;
    if matches!(cli.command, Command::Onboard) {
        validate_onboard(&config)?;
    }
    match &cli.command {
        Command::Config(ConfigCommand::Show) => {
            return Ok(
                json!({"api_url":config.api_url,"rpc":if config.hosted_rpc { "hosted" } else if config.rpc_url.is_some() { "custom (URL redacted)" } else { "disabled" },"wallet":config.wallet.map(|w|w.to_string()),"keypair_configured":config.keypair.is_some(),"separate_payer":config.payer.is_some(),"journal_dir":config.journal,"program_id":config.program_id.to_string(),"market":config.market.to_string(),"provider_list":config.provider_list.to_string(),"usdc_mint":config.mint.to_string(),"mode":if cli.options.execute {"execute"} else {"plan"}}),
            );
        }
        Command::Metadata(MetadataCommand::Upload { kind, file }) => {
            let doc = document(*kind, file)?;
            return if cli.options.execute {
                Ok(
                    json!({"mode":"uploaded","upload":runtime::hosted(&config)?.upload_metadata(&doc).await?}),
                )
            } else {
                Ok(
                    json!({"mode":"plan","operation":"upload metadata","document":document_view(&doc),"reference":reference_view(&hosted_reference(&config,&doc)?)}),
                )
            };
        }
        Command::Image(ImageCommand::Upload { file }) => {
            let (mime, bytes) = image(file)?;
            return if cli.options.execute {
                Ok(
                    json!({"mode":"uploaded","upload":runtime::hosted(&config)?.upload_image(mime,&bytes).await?}),
                )
            } else {
                Ok(
                    json!({"mode":"plan","operation":"upload image","content_type":mime,"bytes":bytes.len(),"sha256":fossa_sdk::metadata::sha256_hex(&bytes)}),
                )
            };
        }
        Command::Transaction(command) => return transaction(&config, &cli.options, command).await,
        _ => (),
    }
    let rt = Runtime::connect(config, cli.options).await?;
    match cli.command {
        Command::Routing(command) => routing::run(&rt, command).await,
        Command::Provider(command) => provider(&rt, command).await,
        Command::Model(command) => model(&rt, command).await,
        Command::Group(command) => model_markets::group(&rt, command).await,
        Command::Council(command) => model_markets::council(&rt, command).await,
        Command::Offer(command) => offer(&rt, command).await,
        Command::List(command) => provider_list(&rt, command).await,
        Command::Account(AccountCommand::Show { address, kind }) => {
            account(&rt, address, kind).await
        }
        Command::Onboard => onboard(&rt).await,
        Command::User(command) => user(&rt, command).await,
        Command::Booking(command) => booking(&rt, command).await,
        _ => unreachable!(),
    }
}
fn preflight(command: &Command) -> Result<()> {
    model_markets::preflight(command)?;
    match command {
        Command::Metadata(
            MetadataCommand::Validate { kind, file } | MetadataCommand::Upload { kind, file },
        ) => {
            document(*kind, file)?;
        }
        Command::Image(ImageCommand::Upload { file }) => {
            image(file)?;
        }
        Command::Provider(ProviderCommand::Register(args)) => {
            document(MetadataKind::Provider, &args.metadata_file)?;
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::Provider(ProviderCommand::Update(args)) => {
            Document::parse(
                Kind::Provider,
                serde_json::to_vec(
                    &json!({"name":args.name.as_deref().unwrap_or("Profile"),"endpoint":"https://example.invalid","image":args.image_url}),
                )?,
            )?;
            if let Some(path) = &args.metadata_file {
                document(MetadataKind::Provider, path)?;
            }
            if let Some(path) = &args.image_file {
                image(path)?;
            }
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::Model(ModelCommand::UpdateMetadata {
            model,
            document: args,
        }) => {
            model_document(&model.model, &args.metadata_file)?;
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::List(ListCommand::Create {
            metadata_file,
            metadata_uri,
            ..
        }) => {
            if let Some(path) = metadata_file {
                document(MetadataKind::List, path)?;
            }
            validate_uri(metadata_uri.as_deref())?;
        }
        Command::List(ListCommand::Update { profile, .. }) => {
            if let Some(path) = &profile.metadata_file {
                document(MetadataKind::List, path)?;
            }
            if let Some(path) = &profile.image_file {
                image(path)?;
            }
            if let Some(path) = &profile.icon_file {
                icon(path)?;
            }
            patch_list_profile(
                &json!({"name":"Group","accentColor":"#000000"}),
                profile,
                None,
                None,
            )?;
            validate_uri(profile.metadata_uri.as_deref())?;
        }
        Command::Offer(OfferCommand::Update { patch, .. }) => {
            if matches!((patch.input_price,patch.cached_price),(Some(input),Some(cached)) if cached>input)
            {
                return Err(Error::Input(
                    "cached price must not exceed input price".into(),
                ));
            }
        }
        Command::Booking(BookingCommand::Create(args)) => {
            book_request(args, crate::config::PROVIDER_LIST).validate()?;
        }
        _ => (),
    }
    Ok(())
}

fn model_address(rt: &Runtime, id: &str) -> Result<Pubkey> {
    if id.is_empty() {
        return Err(Error::Input("model identifier must not be empty".into()));
    }
    Ok(address::model(
        &rt.protocol.program,
        &rt.protocol.market,
        &address::model_id(id),
    )
    .0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_checkpoint_defaults_to_earliest_ordered_identity() {
        let source = include_str!("../config.example.toml")
            .replace("checkpoint = \"example/model@2026-10-17\"\n", "");
        let mut config: crate::config::FileConfig = toml::from_str(&source).unwrap();
        let model = &mut config.provider.models[0];
        assert!(model.checkpoint.is_none());
        let mut value = json!({"id":model.id,"name":"Model","modalities":{"input":["text"],"output":["text"]},"contextTokens":128,"outputTokens":64,"checkpoints":[{"id":"example/model","name":"Latest","ordered":false},{"id":"example/model@0905","name":"0905"},{"id":"example/model@1017","name":"1017"}]});
        let document = Document::model(&model.id, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            model_checkpoint(model, &document).unwrap(),
            address::model_id("example/model@0905")
        );
        model.checkpoint = Some("example/model@1017".into());
        assert_eq!(
            model_checkpoint(model, &document).unwrap(),
            address::model_id("example/model@1017")
        );
        model.checkpoint = None;
        value["checkpoints"].as_array_mut().unwrap().truncate(1);
        let document = Document::model(&model.id, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            model_checkpoint(model, &document).unwrap(),
            address::model_id("example/model")
        );
        value.as_object_mut().unwrap().remove("checkpoints");
        let document = Document::model(&model.id, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(model_checkpoint(model, &document).is_err());
    }

    #[test]
    fn group_profile_patch_preserves_fields_and_explicitly_clears_assets() {
        let current = json!({"schemaVersion":1,"name":"Group","accentColor":"#123456","image":"https://example.com/a.png","website":"https://example.com","icon":"https://example.com/a.svg"});
        let patch = ListProfileArgs {
            name: Some("Next".into()),
            remove_image: true,
            remove_website: true,
            remove_icon: true,
            ..Default::default()
        };
        let next = patch_list_profile(&current, &patch, None, None).unwrap();
        assert_eq!(
            next.value(),
            &json!({"schemaVersion":1,"name":"Next","accentColor":"#123456","image":null,"website":null,"icon":null})
        );
        let invalid = ListProfileArgs {
            accent_color: Some("#fff\n".into()),
            ..Default::default()
        };
        assert!(patch_list_profile(&current, &invalid, None, None).is_err());
        assert!(
            patch_list_profile(
                &json!({"schemaVersion":1}),
                &ListProfileArgs::default(),
                None,
                None
            )
            .is_err()
        );
    }
    #[test]
    fn price_patch_preserves_capacity_privacy_activation_speed_and_other_rates() {
        let old = OfferTerms {
            checkpoint: [7; 32],
            input_modalities: 1,
            output_modalities: 1,
            context_window: 1_000_000,
            max_input: 1_000_000,
            max_output: 1_000_000,
            rates: Rates {
                input: 1_000_000,
                cached: 500_000,
                output: 2_000_000,
            },
            speed_tps: 300,
            private: true,
            active: false,
            capacity: 7,
        };
        let next = patch_terms(
            old,
            &OfferPatch {
                output_price: Some(3_123_456),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            next,
            OfferTerms {
                checkpoint: [7; 32],
                input_modalities: 1,
                output_modalities: 1,
                context_window: 1_000_000,
                max_input: 1_000_000,
                max_output: 1_000_000,
                rates: Rates {
                    output: 3_123_456,
                    ..old.rates
                },
                ..old
            }
        );
        assert!(
            patch_terms(
                old,
                &OfferPatch {
                    input_price: Some(1),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn profile_patch_preserves_extensions_changes_exact_hash_and_new_reference() {
        let bytes=br#"{ "name":"Old", "endpoint":"https://provider.example/infer", "image":"https://provider.example/old.png", "extension":{"a":[1,true,"opaque"]} }"#;
        let current = Document::parse(Kind::Provider, bytes.to_vec()).unwrap();
        let old = current.reference("https://example.com/old.json").unwrap();
        current.verify(&old).unwrap();
        let next =
            patch_profile(&current, Some("New"), Some("https://example.com/new.png")).unwrap();
        assert_eq!(next.value()["extension"], current.value()["extension"]);
        assert_eq!(next.value()["endpoint"], current.value()["endpoint"]);
        assert_eq!(next.value()["name"], "New");
        assert_eq!(next.value()["image"], "https://example.com/new.png");
        assert!(next.verify(&old).is_err());
        assert_eq!(
            next.reference("https://example.com/new.json")
                .unwrap()
                .sha256,
            next.sha256()
        );
    }
    #[test]
    fn invalid_profile_patch_cannot_reach_upload() {
        let current = Document::parse(
            Kind::Provider,
            br#"{"name":"Old","endpoint":"https://example.com/infer"}"#.to_vec(),
        )
        .unwrap();
        assert!(patch_profile(&current, Some(" "), None).is_err());
        assert!(patch_profile(&current, None, Some("http://example.com/logo.png")).is_err());
    }
}

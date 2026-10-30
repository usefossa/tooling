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

mod model_markets;

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
fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Input(format!("file exceeds {limit} bytes")));
    }
    Ok(bytes)
}
fn document(kind: MetadataKind, path: &Path) -> Result<Document> {
    Ok(Document::parse(
        kind.into(),
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
fn model_document(id: &str, path: &Path) -> Result<Document> {
    Ok(Document::model(
        id,
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
fn image(path: &Path) -> Result<(&'static str, Vec<u8>)> {
    let bytes = read_limited(path, 2 * 1024 * 1024)?;
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err(Error::Input("expected PNG, JPEG or WebP image".into()));
    };
    Ok((mime, bytes))
}
fn icon(path: &Path) -> Result<Vec<u8>> {
    let bytes = read_limited(path, fossa_sdk::schema::MAX_ICON_BYTES)?;
    fossa_sdk::schema::validate_icon(&bytes).map_err(fossa_sdk::Error::Metadata)?;
    Ok(bytes)
}
fn validate_uri(uri: Option<&str>) -> Result<()> {
    if let Some(uri) = uri {
        fossa_sdk::metadata::validate_uri(uri)?;
    }
    Ok(())
}

fn document_view(doc: &Document) -> Value {
    json!({"sha256":runtime::hex(&doc.sha256()),"bytes":doc.bytes().len(),"document":doc.value()})
}
fn reference_view(reference: &DocumentRef) -> Value {
    json!({"sha256":runtime::hex(&reference.sha256),"uri":reference.uri})
}
fn hosted_reference(config: &Config, doc: &Document) -> Result<DocumentRef> {
    Ok(doc.reference(format!(
        "{}/v1/market/metadata/{}",
        config.api_url.trim_end_matches('/'),
        runtime::hex(&doc.sha256())
    ))?)
}
async fn publish(rt: &Runtime, doc: &Document, uri: Option<&str>) -> Result<DocumentRef> {
    if let Some(uri) = uri {
        let reference = doc.reference(uri)?;
        if rt.options.execute {
            Document::fetch(doc.kind(), &reference).await?;
        }
        return Ok(reference);
    }
    if rt.options.execute {
        rt.ensure_no_pending()?;
        Ok(runtime::hosted(&rt.config)?
            .upload_metadata(doc)
            .await?
            .reference()?)
    } else {
        hosted_reference(&rt.config, doc)
    }
}
async fn provider(rt: &Runtime, command: ProviderCommand) -> Result<Value> {
    let wallet = rt.wallet()?;
    let address = address::provider(&rt.protocol.program, &rt.protocol.market, &wallet).0;
    if matches!(command, ProviderCommand::Show) {
        return Ok(
            json!({"address":address.to_string(),"provider":rt.rpc.account::<Provider>(address,rt.protocol.program).await?}),
        );
    }
    let state = rt
        .rpc
        .account::<Provider>(address, rt.protocol.program)
        .await?;
    match command {
        ProviderCommand::Register(args) => {
            if state.is_some() {
                return Err(Error::Input(
                    "provider already registered; use provider update".into(),
                ));
            }
            let doc = document(MetadataKind::Provider, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt
                .protocol
                .initialize_provider(wallet, rt.payer()?, &reference)?;
            rt.transact(
                "register provider",
                vec![ix],
                &[],
                json!({"provider":address.to_string(),"metadata":reference_view(&reference)}),
            )
            .await
        }
        ProviderCommand::Update(args) => {
            let state = state
                .ok_or_else(|| Error::Input("provider missing; use provider register".into()))?;
            let doc = if let Some(path) = &args.metadata_file {
                document(MetadataKind::Provider, path)?
            } else {
                let current = Document::fetch(Kind::Provider, &state.document).await?;
                let image_data = args.image_file.as_deref().map(image).transpose()?;
                let image_uri = image_data
                    .as_ref()
                    .map(|(_, bytes)| {
                        format!(
                            "{}/v1/providers/images/{}",
                            rt.config.api_url,
                            fossa_sdk::metadata::sha256_hex(bytes)
                        )
                    })
                    .or_else(|| args.image_url.clone());
                let document = patch_profile(&current, args.name.as_deref(), image_uri.as_deref())?;
                if rt.options.execute
                    && let Some((mime, bytes)) = &image_data
                {
                    rt.ensure_no_pending()?;
                    runtime::hosted(&rt.config)?
                        .upload_image(mime, bytes)
                        .await?;
                }
                document
            };
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt.protocol.update_provider_metadata(wallet, &reference)?;
            rt.transact("update provider metadata",vec![ix],&[],json!({"provider":address.to_string(),"previous":reference_view(&state.document),"metadata":reference_view(&reference),"document":doc.value(),"image_upload":args.image_file.is_some()})).await
        }
        ProviderCommand::Show => unreachable!(),
    }
}
pub fn patch_profile(
    current: &Document,
    name: Option<&str>,
    image: Option<&str>,
) -> Result<Document> {
    let mut value = current.value().clone();
    if let Some(name) = name {
        value["name"] = name.into();
    }
    if let Some(image) = image {
        value["image"] = image.into();
    }
    Ok(Document::parse(
        Kind::Provider,
        serde_json::to_vec(&value)?,
    )?)
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
async fn model(rt: &Runtime, command: ModelCommand) -> Result<Value> {
    match command {
        ModelCommand::AssignGroup { model, group } => {
            model_markets::assign(rt, model, Some(group)).await
        }
        ModelCommand::ClearGroup(model) => model_markets::assign(rt, model, None).await,
        ModelCommand::Show(args) => {
            let address = model_address(rt, &args.model)?;
            let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            let metadata = rt
                .rpc
                .account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            Ok(
                json!({"address":address.to_string(),"model":model,"metadata":metadata,"terms":terms,"capacity":capacity}),
            )
        }
        ModelCommand::Grow { model: args, limit } => {
            let address = model_address(rt, &args.model)?;
            let model = rt
                .rpc
                .require_account::<Model>(address, rt.protocol.program)
                .await?;
            if model.market != rt.protocol.market
                || limit <= model.limit
                || limit - model.limit > 85
            {
                return Err(Error::Input(
                    "new limit must add 1 to 85 slots in this market".into(),
                ));
            }
            let books = rt
                .rpc
                .multiple_accounts(&[model.terms, model.capacity])
                .await?;
            let terms_raw = books.value[0]
                .as_ref()
                .ok_or(fossa_sdk::Error::MissingAccount(model.terms))?;
            let capacity_raw = books.value[1]
                .as_ref()
                .ok_or(fossa_sdk::Error::MissingAccount(model.capacity))?;
            let terms = TermsBook::decode(&model.terms, terms_raw, &rt.protocol.program)?;
            let capacity =
                CapacityBook::decode(&model.capacity, capacity_raw, &rt.protocol.program)?;
            terms.validate_pair(&capacity, &address, &model)?;
            let [terms_size, capacity_size] = instruction::book_sizes(limit)?;
            let (terms_rent, capacity_rent) =
                tokio::try_join!(rt.rpc.rent(terms_size), rt.rpc.rent(capacity_size))?;
            let rent = terms_rent
                .saturating_sub(terms_raw.lamports)
                .checked_add(capacity_rent.saturating_sub(capacity_raw.lamports))
                .ok_or(fossa_sdk::Error::Arithmetic)?;
            rt.transact("grow model books", vec![rt.protocol.grow_model(ModelKeys::from_account(address, &model), rt.payer()?, limit)?], &[], json!({"model":args.model,"previous_limit":model.limit,"limit":limit,"additional_rent_lamports":rent.to_string()})).await
        }
        ModelCommand::UpdateMetadata {
            model,
            document: args,
        } => {
            let address = model_address(rt, &model.model)?;
            let metadata = rt
                .rpc
                .require_account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            if metadata.market != rt.protocol.market
                || metadata.model != address
                || metadata.authority != rt.wallet()?
            {
                return Err(Error::Input(
                    "model metadata authority or binding mismatch".into(),
                ));
            }
            let doc = model_document(&model.model, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            rt.transact(
                "update model metadata",
                vec![
                    rt.protocol
                        .update_model_metadata(address, rt.wallet()?, &reference)?,
                ],
                &[],
                json!({"model":model.model,"metadata":reference_view(&reference)}),
            )
            .await
        }
    }
}
pub fn patch_terms(mut terms: OfferTerms, patch: &OfferPatch) -> Result<OfferTerms> {
    if let Some(n) = patch.input_price {
        terms.rates.input = n;
    }
    if let Some(n) = patch.cached_price {
        terms.rates.cached = n;
    }
    if let Some(n) = patch.output_price {
        terms.rates.output = n;
    }
    if let Some(n) = patch.capacity {
        terms.capacity = n;
    }
    if let Some(n) = patch.speed_tps {
        terms.speed_tps = n;
    }
    if let Some(v) = patch.active {
        terms.active = v;
    }
    if let Some(v) = patch.private {
        terms.private = v;
    }
    terms.validate()?;
    Ok(terms)
}
async fn selected_offer(rt: &Runtime, args: &OfferArg) -> Result<(OfferKeys, Offer, Capacity)> {
    offer_at(rt, model_address(rt, &args.model.model)?, args.slot).await
}
async fn offer_at(
    rt: &Runtime,
    address: Pubkey,
    slot: Option<u32>,
) -> Result<(OfferKeys, Offer, Capacity)> {
    let (model, terms, capacities) = rt.rpc.model_books(&rt.protocol, address).await?;
    let wallet = rt.wallet()?;
    let slot = if let Some(n) = slot {
        n
    } else {
        let matches = terms
            .entries
            .iter()
            .enumerate()
            .filter(|(_, offer)| offer.provider == wallet)
            .map(|(i, _)| i as u32)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error::Input(
                "select --slot: wallet must have exactly one offer to infer slot".into(),
            ));
        }
        matches[0]
    };
    let offer = terms
        .entries
        .get(slot as usize)
        .ok_or_else(|| Error::Input("offer slot out of range".into()))?
        .clone();
    if offer.provider != wallet {
        return Err(Error::Input("offer belongs to another provider".into()));
    }
    let keys = OfferKeys {
        model: ModelKeys::from_account(address, &model),
        provider: wallet,
        payout: offer.payout,
        slot,
    };
    Ok((keys, offer, capacities.entries[slot as usize]))
}
async fn offer(rt: &Runtime, command: OfferCommand) -> Result<Value> {
    match command {
        OfferCommand::List(args) => {
            let address = model_address(rt, &args.model)?;
            let (_, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            Ok(
                json!({"model":args.model,"address":address.to_string(),"rate_unit":"micro-USDC per 1,000,000 tokens","offers":terms.entries,"capacity":capacity.entries}),
            )
        }
        OfferCommand::Update { offer, patch } => {
            let (keys, current, _) = selected_offer(rt, &offer).await?;
            let next = patch_terms(current.terms, &patch)?;
            rt.transact("update offer",vec![rt.protocol.update_offer(keys,next)?],&[],json!({"model":offer.model.model,"slot":keys.slot,"previous":current.terms,"next":next,"rate_unit":"micro-USDC per 1,000,000 tokens"})).await
        }
    }
}
async fn provider_list(rt: &Runtime, command: ListCommand) -> Result<Value> {
    const COMPUTE_UNITS: u32 = 250_000;
    if let ListCommand::Create {
        capacity,
        fee_bps,
        protocol_share_bps,
        maximum_payment,
        metadata_file,
        metadata_uri,
    } = command
    {
        let list = Keypair::new();
        let authority = rt.wallet()?;
        let address = list.pubkey();
        let market = rt
            .rpc
            .require_account::<Market>(rt.protocol.market, rt.protocol.program)
            .await?;
        if !market.creation_enabled
            || market.creation_net_price == 0
            || market.creation_mint == Pubkey::default()
        {
            return Err(Error::Input("List creation is disabled until governance-token payment is configured and enabled".into()));
        }
        let mut instructions = vec![
            fossa_sdk::transaction::compute_unit_limit(COMPUTE_UNITS),
            rt.protocol.initialize_list(
                list.pubkey(),
                authority,
                instruction::ListCreation {
                    limit: capacity,
                    fee_bps,
                    protocol_share: protocol_share_bps,
                    maximum_payment,
                    source: address::associated_token_with_program(
                        &market.creation_mint,
                        &authority,
                        &market.creation_token_program,
                    ),
                    destination: address::associated_token_with_program(
                        &market.creation_mint,
                        &market.council,
                        &market.creation_token_program,
                    ),
                    mint: market.creation_mint,
                    token_program: market.creation_token_program,
                },
            )?,
        ];
        let metadata = if let Some(path) = metadata_file {
            let doc = document(MetadataKind::List, &path)?;
            let reference = publish(rt, &doc, metadata_uri.as_deref()).await?;
            instructions.push(rt.protocol.initialize_list_metadata(
                address,
                authority,
                rt.payer()?,
                &reference,
            )?);
            Some(json!({"reference":reference_view(&reference),"document":doc.value()}))
        } else {
            None
        };
        return rt.transact("create provider list", instructions, &[list], json!({"list":address.to_string(),"authority":authority.to_string(),"capacity":capacity,"metadata":metadata})).await;
    }
    let address = match command {
        ListCommand::Show { list, .. }
        | ListCommand::Update { list, .. }
        | ListCommand::ConfigureFees { list, .. }
        | ListCommand::Add { list, .. }
        | ListCommand::Remove { list, .. }
        | ListCommand::Close { list } => list,
        _ => unreachable!(),
    };
    let state = rt
        .rpc
        .require_account::<ProviderList>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("provider list market mismatch".into()));
    }
    if let ListCommand::Show { fetch_metadata, .. } = &command {
        let metadata = list_metadata(rt, address, &state).await?;
        let document = if *fetch_metadata && let Some(metadata) = &metadata {
            Some(
                Document::fetch(Kind::List, &metadata.document)
                    .await?
                    .value()
                    .clone(),
            )
        } else {
            None
        };
        return Ok(
            json!({"address":address.to_string(),"state":state,"metadata":metadata,"document":document}),
        );
    }
    let authority = rt.wallet()?;
    if state.authority != authority {
        return Err(Error::Input(
            "provider list belongs to another owner".into(),
        ));
    }
    if let ListCommand::Update { profile, .. } = &command {
        return update_list_profile(rt, address, &state, profile).await;
    }
    if let ListCommand::ConfigureFees {
        fee_bps,
        protocol_share_bps,
        ..
    } = command
    {
        let fees = fossa_sdk::reserve::ListFees {
            fee_bps,
            protocol_share: protocol_share_bps,
        };
        return rt.transact("configure list fees", vec![rt.protocol.configure_list_fees(address, authority, fees)?], &[], json!({"list":address.to_string(),"previous":{"fee_bps":state.fee_bps,"protocol_share":state.protocol_share},"next":fees})).await;
    }
    let (label, ix) = match command {
        ListCommand::Add { provider, .. } => {
            if state.contains(&provider) || state.providers.len() == state.limit as usize {
                return Err(Error::Input("provider already listed or list full".into()));
            }
            (
                "add listed provider",
                rt.protocol.update_list(address, authority, provider, true),
            )
        }
        ListCommand::Remove { provider, .. } => {
            if !state.contains(&provider) {
                return Err(Error::Input("provider is not listed".into()));
            }
            (
                "remove listed provider",
                rt.protocol.update_list(address, authority, provider, false),
            )
        }
        ListCommand::Close { .. } => {
            if !state.providers.is_empty() {
                return Err(Error::Input("provider list must be empty".into()));
            }
            (
                "close provider list",
                rt.protocol.close_list(address, authority),
            )
        }
        _ => unreachable!(),
    };
    let instructions = if matches!(command, ListCommand::Close { .. }) {
        vec![ix]
    } else {
        vec![
            fossa_sdk::transaction::compute_unit_limit(COMPUTE_UNITS),
            ix,
        ]
    };
    rt.transact(
        label,
        instructions,
        &[],
        json!({"list":address.to_string()}),
    )
    .await
}

async fn list_metadata(
    rt: &Runtime,
    list: Pubkey,
    state: &ProviderList,
) -> Result<Option<ListMetadata>> {
    let address = address::list_metadata(&rt.protocol.program, &list).0;
    let Some(raw) = rt.rpc.raw_account(address).await? else {
        return Ok(None);
    };
    if raw.owner == address::SYSTEM_PROGRAM && raw.data.is_empty() {
        return Ok(None);
    }
    let metadata = ListMetadata::decode(&address, &raw, &rt.protocol.program)?;
    if metadata.market != state.market
        || metadata.list != list
        || metadata.authority != state.authority
    {
        return Err(Error::Input("list metadata binding mismatch".into()));
    }
    Ok(Some(metadata))
}

fn patch_list_profile(
    current: &Value,
    patch: &ListProfileArgs,
    image: Option<&str>,
    icon: Option<&str>,
) -> Result<Document> {
    let mut value = current.clone();
    for (field, next) in [
        ("name", patch.name.as_deref()),
        ("accentColor", patch.accent_color.as_deref()),
        ("website", patch.website.as_deref()),
        ("image", image.or(patch.image_url.as_deref())),
        ("icon", icon.or(patch.icon_url.as_deref())),
    ] {
        if let Some(next) = next {
            value[field] = next.into();
        }
    }
    for (field, clear) in [
        ("image", patch.remove_image),
        ("website", patch.remove_website),
        ("icon", patch.remove_icon),
    ] {
        if clear {
            value[field] = Value::Null;
        }
    }
    Ok(Document::parse(Kind::List, serde_json::to_vec(&value)?)?)
}

async fn update_list_profile(
    rt: &Runtime,
    list: Pubkey,
    state: &ProviderList,
    args: &ListProfileArgs,
) -> Result<Value> {
    let previous = list_metadata(rt, list, state).await?;
    let doc = if let Some(path) = &args.metadata_file {
        document(MetadataKind::List, path)?
    } else {
        let current = if let Some(metadata) = &previous {
            Document::fetch(Kind::List, &metadata.document)
                .await?
                .value()
                .clone()
        } else {
            json!({"schemaVersion":1})
        };
        let image_data = args.image_file.as_deref().map(image).transpose()?;
        let icon_data = args.icon_file.as_deref().map(icon).transpose()?;
        let asset_uri = |bytes: &[u8]| {
            format!(
                "{}/v1/providers/images/{}",
                rt.config.api_url.trim_end_matches('/'),
                fossa_sdk::metadata::sha256_hex(bytes)
            )
        };
        let image_uri = image_data.as_ref().map(|(_, bytes)| asset_uri(bytes));
        let icon_uri = icon_data.as_deref().map(asset_uri);
        let doc = patch_list_profile(&current, args, image_uri.as_deref(), icon_uri.as_deref())?;
        if rt.options.execute {
            rt.ensure_no_pending()?;
            if let Some((mime, bytes)) = &image_data {
                runtime::hosted(&rt.config)?
                    .upload_image(mime, bytes)
                    .await?;
            }
            if let Some(bytes) = &icon_data {
                runtime::hosted(&rt.config)?
                    .upload_image("image/svg+xml", bytes)
                    .await?;
            }
        }
        doc
    };
    let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
    let instruction = if previous.is_some() {
        rt.protocol
            .update_list_metadata(list, state.authority, &reference)?
    } else {
        rt.protocol
            .initialize_list_metadata(list, state.authority, rt.payer()?, &reference)?
    };
    rt.transact("update list metadata", vec![instruction], &[], json!({"list":list.to_string(),"previous":previous.map(|m|reference_view(&m.document)),"metadata":reference_view(&reference),"document":doc.value(),"image_upload":args.image_file.is_some(),"icon_upload":args.icon_file.is_some()})).await
}

async fn validate_token(rt: &Runtime, account: Pubkey, owner: Pubkey) -> Result<()> {
    let raw = rt
        .rpc
        .raw_account(account)
        .await?
        .ok_or(fossa_sdk::Error::MissingAccount(account))?;
    TokenAccount::decode(&raw)?.validate(&rt.protocol.mint, &owner)?;
    Ok(())
}
async fn account(rt: &Runtime, address: Pubkey, kind: AccountKind) -> Result<Value> {
    macro_rules! typed {
        ($t:ty) => {
            serde_json::to_value(
                rt.rpc
                    .require_account::<$t>(address, rt.protocol.program)
                    .await?,
            )?
        };
    }
    let state = match kind {
        AccountKind::Market => typed!(Market),
        AccountKind::Provider => typed!(Provider),
        AccountKind::Model => typed!(Model),
        AccountKind::ModelMetadata => typed!(ModelMetadata),
        AccountKind::ModelGroup => typed!(ModelGroup),
        AccountKind::User => typed!(User),
        AccountKind::Terms => typed!(TermsBook),
        AccountKind::Capacity => typed!(CapacityBook),
        AccountKind::Booking => typed!(Booking),
        AccountKind::ProviderList => typed!(ProviderList),
        AccountKind::ListMetadata => typed!(ListMetadata),
        AccountKind::Token => serde_json::to_value(TokenAccount::decode(
            &rt.rpc
                .raw_account(address)
                .await?
                .ok_or(fossa_sdk::Error::MissingAccount(address))?,
        )?)?,
    };
    Ok(json!({"address":address.to_string(),"account":state}))
}
async fn transaction(
    config: &Config,
    options: &Options,
    command: &TransactionCommand,
) -> Result<Value> {
    let (path, resume) = match command {
        TransactionCommand::Status { file } => (file, false),
        TransactionCommand::Resume { file } => (file, true),
    };
    let _journal_lock = runtime::JournalLock::acquire(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let pending = PendingTransaction::load(path)?;
    let rpc = runtime::rpc(config)?;
    let signature = pending.validate()?;
    if resume && options.execute {
        rpc.submit(&pending)
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let status = rpc
            .confirm(&pending, Duration::from_secs(options.confirmation_seconds))
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let archive = runtime::archive_pending(path, &pending)?;
        return Ok(
            json!({"mode":"executed","signature":pending.signature,"status":status,"journal":archive}),
        );
    }
    let mut status = rpc.signature_status(&signature).await?;
    let finalized_height = if status.is_none() {
        Some(
            rpc.clone()
                .with_commitment(fossa_sdk::rpc::Commitment::Finalized)
                .block_height()
                .await?,
        )
    } else {
        None
    };
    let expired = finalized_height.is_some_and(|n| n > pending.last_valid_block_height);
    if expired {
        status = rpc.signature_status(&signature).await?;
    }
    Ok(
        json!({"mode":if resume {"plan"} else {"inspection"},"signature":pending.signature,"status":status,"expired_without_observed_status":expired&&status.is_none(),"last_valid_block_height":pending.last_valid_block_height,"journal":path,"next":if resume {"--execute reconciles then resends only original bytes"} else {"use transaction resume --file PATH --execute; preserve unresolved journals"}}),
    )
}
async fn user(rt: &Runtime, command: UserCommand) -> Result<Value> {
    if let UserCommand::Show { user } = command {
        return account(rt, user, AccountKind::User).await;
    }
    let wallet = rt.wallet()?;
    if let UserCommand::Init = command {
        let key = Keypair::new();
        let address = key.pubkey();
        let authority = address::vault(&rt.protocol.program, &address).0;
        let vault = address::associated_token(&rt.protocol.mint, &authority);
        let payer = rt.payer()?;
        let ix = vec![
            instruction::create_associated_token(payer, authority, rt.protocol.mint),
            rt.protocol.initialize_user(UserKeys {
                user: address,
                owner: wallet,
                vault,
            }),
        ];
        return rt
            .transact(
                "initialize user",
                ix,
                &[key],
                json!({"user":address.to_string(),"vault":vault.to_string()}),
            )
            .await;
    }
    let (address, amount, token, deposit) = match command {
        UserCommand::Deposit {
            user,
            amount,
            source,
        } => (user, amount, source, true),
        UserCommand::Withdraw {
            user,
            amount,
            destination,
        } => (user, amount, destination, false),
        _ => unreachable!(),
    };
    let state = rt
        .rpc
        .require_account::<User>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market || state.owner != wallet {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let default_token = token.is_none();
    let token = token.unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &wallet));
    let keys = UserKeys::from_account(address, &state);
    let ix = if deposit {
        validate_token(rt, token, wallet).await?;
        vec![rt.protocol.deposit(keys, token, amount)?]
    } else {
        let mut instructions = Vec::with_capacity(2);
        if default_token {
            instructions.push(instruction::create_associated_token(
                rt.payer()?,
                wallet,
                rt.protocol.mint,
            ));
        }
        instructions.push(rt.protocol.withdraw(keys, token, amount)?);
        instructions
    };
    rt.transact(if deposit {"deposit user funds"} else {"withdraw user funds"},ix,&[],json!({"user":address.to_string(),"amount_atomic":amount.to_string(),"amount_usdc":amount::format_usdc(amount)})).await
}
async fn booking(rt: &Runtime, command: BookingCommand) -> Result<Value> {
    if let BookingCommand::Create(args) = command {
        return book(rt, args).await;
    }
    let address = match command {
        BookingCommand::Show { booking }
        | BookingCommand::Claim { booking, .. }
        | BookingCommand::Release { booking }
        | BookingCommand::Expire { booking } => booking,
        BookingCommand::Create(_) => unreachable!(),
    };
    if matches!(command, BookingCommand::Show { .. }) {
        return account(rt, address, AccountKind::Booking).await;
    }
    let state = rt
        .rpc
        .require_account::<Booking>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("booking market mismatch".into()));
    }
    let model = rt
        .rpc
        .require_account::<Model>(state.model, rt.protocol.program)
        .await?;
    let model = ModelKeys::from_account(state.model, &model);
    let user = rt
        .rpc
        .require_account::<User>(state.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market {
        return Err(Error::Input("booking user market mismatch".into()));
    }
    let keys = SettlementKeys::from_booking(
        address,
        &state,
        model,
        UserKeys::from_account(state.user, &user),
        &rt.protocol.mint,
    )?;
    let generation = (state.generation > 0).then_some(state.generation);
    let (label, ix) = match command {
        BookingCommand::Claim {
            uncached,
            cached,
            output,
            ..
        } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            if u64::from(uncached) + u64::from(cached) > u64::from(state.max_input)
                || output > state.max_output
            {
                return Err(Error::Input("usage exceeds booking token caps".into()));
            }
            let usage = Usage {
                uncached,
                cached,
                output,
            };
            let receipt = generation
                .map(|_| fossa_sdk::receipt::Receipt::from_booking(address, &state, usage))
                .transpose()?;
            (
                "claim booking",
                rt.protocol.claim(keys, usage, receipt.as_ref())?,
            )
        }
        BookingCommand::Release { .. } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            ("release booking", rt.protocol.release(keys, generation)?)
        }
        BookingCommand::Expire { .. } => ("expire booking", rt.protocol.expire(keys, generation)?),
        _ => unreachable!(),
    };
    rt.transact(
        label,
        vec![ix],
        &[],
        json!({"booking":address.to_string(),"generation":state.generation.to_string()}),
    )
    .await
}
async fn onboard(rt: &Runtime) -> Result<Value> {
    let cfg = &rt.config.file.provider;
    let file = cfg
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    let doc = document(MetadataKind::Provider, &rt.config.base.join(file))?;
    validate_uri(cfg.metadata_uri.as_deref())?;
    let reference = match &cfg.metadata_uri {
        Some(uri) => doc.reference(uri)?,
        None => hosted_reference(&rt.config, &doc)?,
    };
    let mut documents = vec![(doc, cfg.metadata_uri.as_deref())];
    let mut models = Vec::new();
    for model in &cfg.models {
        let document = model_document(&model.id, &rt.config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        let reference = match &model.metadata_uri {
            Some(uri) => document.reference(uri)?,
            None => hosted_reference(&rt.config, &document)?,
        };
        let terms = OfferTerms {
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(cfg.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(cfg.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(cfg.private).unwrap_or(false),
            active: model.active.or(cfg.active).unwrap_or(true),
        };
        terms.validate()?;
        models.push(onboarding::ModelConfig {
            id: model.id.clone(),
            document: reference,
            terms,
            slot: model.slot,
        });
        documents.push((document, model.metadata_uri.as_deref()));
    }
    let config = onboarding::OnboardConfig {
        provider: rt.wallet()?,
        payer: rt.payer()?,
        document: reference,
        book_limit: cfg.model_book_limit.unwrap_or(16),
        models,
    };
    let plan = onboarding::plan(&rt.rpc, &rt.protocol, &config).await?;
    let view = json!({"mode":"plan","operation":"onboard","blockers":plan.blockers,"steps":plan.steps.iter().map(|s|json!({"operation":s.label,"instructions":runtime::instruction_view(&s.instructions)})).collect::<Vec<_>>(),"metadata":documents.iter().map(|(d,_)|document_view(d)).collect::<Vec<_>>()});
    if !rt.options.execute {
        return Ok(view);
    }
    if !plan.blockers.is_empty() {
        return Err(Error::Input(format!(
            "onboarding blocked: {}",
            plan.blockers.join("; ")
        )));
    }
    rt.ensure_no_pending()?;
    for (doc, uri) in &documents {
        publish(rt, doc, *uri).await?;
    }
    let mut completed = Vec::new();
    for step in plan.steps {
        completed.push(
            rt.transact(
                &step.label,
                step.instructions,
                &step.signers,
                json!({"onboarding":true}),
            )
            .await?,
        );
    }
    Ok(json!({"mode":"executed","operation":"onboard","steps":completed}))
}

fn book_request(args: &BookArgs, default_list: Pubkey) -> instruction::BookRequest {
    instruction::BookRequest {
        model_id: address::model_id(&args.model.model),
        provider_list: args.list.unwrap_or(default_list),
        rate_caps: Rates {
            input: args.input_price_cap,
            cached: args.cached_price_cap,
            output: args.output_price_cap,
        },
        min_speed: args.min_speed,
        max_input: args.max_input,
        max_output: args.max_output,
        private: args.private,
        max_spend: args.max_spend,
        estimated_input: args.estimated_input.unwrap_or(args.max_input),
        estimated_cached: args.estimated_cached,
        estimated_output: args.estimated_output.unwrap_or(args.max_output),
        cache_provider: args.cache_provider.unwrap_or_default(),
    }
}
async fn book(rt: &Runtime, args: BookArgs) -> Result<Value> {
    let user = rt
        .rpc
        .require_account::<User>(args.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market || user.owner != rt.wallet()? {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let address = model_address(rt, &args.model.model)?;
    let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
    let list = rt
        .rpc
        .require_account::<ProviderList>(
            args.list.unwrap_or(rt.config.provider_list),
            rt.protocol.program,
        )
        .await?;
    if list.market != rt.protocol.market {
        return Err(Error::Input("provider list market mismatch".into()));
    }
    let request = book_request(&args, rt.config.provider_list);
    let selected = request
        .select(&terms, &capacity, &list, user.free)?
        .ok_or_else(|| {
            Error::Input(
                "no eligible offer fits list, price, speed, privacy, capacity and balance limits"
                    .into(),
            )
        })?;
    let key = Keypair::new();
    let booking = key.pubkey();
    let payer = rt.payer()?;
    let ix = vec![
        fossa_sdk::transaction::compute_unit_limit(
            (20_000 + 500 * terms.entries.len().min(4096) as u32).min(1_400_000),
        ),
        rt.protocol.initialize_reusable_booking(
            UserKeys::from_account(args.user, &user),
            booking,
            payer,
        ),
        rt.protocol.book(
            ModelKeys::from_account(address, &model),
            UserKeys::from_account(args.user, &user),
            booking,
            payer,
            &request,
            instruction::BookingMode::OneShot,
        )?,
    ];
    rt.transact("create booking",ix,&[key],json!({"booking":booking.to_string(),"user":args.user.to_string(),"candidate_slot":selected.slot,"reserve_atomic":selected.reserve.to_string(),"selection":"on-chain selection uses execution-time state"})).await
}
fn validate_onboard(config: &Config) -> Result<()> {
    let provider = &config.file.provider;
    let path = provider
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    document(MetadataKind::Provider, &config.base.join(path))?;
    validate_uri(provider.metadata_uri.as_deref())?;
    instruction::book_sizes(provider.model_book_limit.unwrap_or(16))?;
    for model in &provider.models {
        model_document(&model.id, &config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        OfferTerms {
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(provider.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(provider.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(provider.private).unwrap_or(false),
            active: model.active.or(provider.active).unwrap_or(true),
        }
        .validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

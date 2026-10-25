use crate::{
    Error, Instruction, Keypair, Pubkey, Result, RpcClient, Signer,
    accounts::{Decode, DocumentRef, Model, ModelMetadata, OfferTerms, Provider},
    address, ensure,
    instruction::{InitializeModel, ModelKeys, OfferKeys, Protocol, create_associated_token},
};

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub id: String,
    pub document: DocumentRef,
    pub terms: OfferTerms,
    pub slot: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct OnboardConfig {
    pub provider: Pubkey,
    pub payer: Pubkey,
    pub document: DocumentRef,
    pub book_limit: u32,
    pub models: Vec<ModelConfig>,
}
pub struct PlanStep {
    pub label: String,
    pub instructions: Vec<Instruction>,
    pub signers: Vec<Keypair>,
}
impl PlanStep {
    fn new(label: impl Into<String>, instructions: Vec<Instruction>) -> Self {
        Self {
            label: label.into(),
            instructions,
            signers: vec![],
        }
    }
}
pub struct OnboardingPlan {
    pub steps: Vec<PlanStep>,
    pub blockers: Vec<String>,
}

pub async fn plan(
    rpc: &RpcClient,
    protocol: &Protocol,
    config: &OnboardConfig,
) -> Result<OnboardingPlan> {
    let [terms_size, capacity_size] = crate::instruction::book_sizes(config.book_limit)?;
    config.document.encode()?;
    let mut seen = std::collections::HashSet::new();
    for model in &config.models {
        ensure(
            !model.id.is_empty() && seen.insert(&model.id),
            "unique model ids",
        )?;
        model.terms.validate()?;
        model.document.encode()?;
    }
    rpc.validate_market(protocol).await?;
    let mut plan = OnboardingPlan {
        steps: vec![],
        blockers: vec![],
    };
    let profile_key = address::provider(&protocol.program, &protocol.market, &config.provider).0;
    let profile = rpc
        .account::<Provider>(profile_key, protocol.program)
        .await?;
    if let Some(profile) = &profile {
        if profile.document != config.document {
            plan.steps.push(PlanStep::new(
                "update provider metadata",
                vec![protocol.update_provider_metadata(config.provider, &config.document)?],
            ));
        }
    } else {
        plan.steps.push(PlanStep::new(
            "register provider",
            vec![protocol.initialize_provider(config.provider, config.payer, &config.document)?],
        ));
    }
    let mut book_rent = None;
    for desired in &config.models {
        let mut growth = None;
        let id = address::model_id(&desired.id);
        let model_key = address::model(&protocol.program, &protocol.market, &id).0;
        let existing = rpc.account::<Model>(model_key, protocol.program).await?;
        let (model, offer, slot) = if let Some(state) = existing {
            ensure(
                state.market == protocol.market && state.id == id,
                "model binding",
            )?;
            let keys = ModelKeys::from_account(model_key, &state);
            let metadata_key = address::model_metadata(&protocol.program, &model_key).0;
            let result = rpc
                .multiple_accounts(&[metadata_key, state.terms, state.capacity])
                .await?;
            let doc = ModelMetadata::decode(
                &metadata_key,
                result.value[0]
                    .as_ref()
                    .ok_or(Error::MissingAccount(metadata_key))?,
                &protocol.program,
            )?;
            ensure(
                doc.market == protocol.market && doc.model == model_key,
                "model metadata binding",
            )?;
            if doc.document != desired.document {
                if doc.authority == config.provider {
                    plan.steps.push(PlanStep::new(
                        format!("update model metadata {}", desired.id),
                        vec![protocol.update_model_metadata(
                            model_key,
                            config.provider,
                            &desired.document,
                        )?],
                    ));
                } else {
                    plan.blockers.push(format!(
                        "model {} metadata owned by {}",
                        desired.id, doc.authority
                    ));
                }
            }
            let terms = crate::accounts::TermsBook::decode(
                &state.terms,
                result.value[1]
                    .as_ref()
                    .ok_or(Error::MissingAccount(state.terms))?,
                &protocol.program,
            )?;
            let capacities = crate::accounts::CapacityBook::decode(
                &state.capacity,
                result.value[2]
                    .as_ref()
                    .ok_or(Error::MissingAccount(state.capacity))?,
                &protocol.program,
            )?;
            terms.validate_pair(&capacities, &model_key, &state)?;
            let slot = desired.slot.unwrap_or_else(|| {
                terms
                    .entries
                    .iter()
                    .position(|o| o.provider == config.provider)
                    .unwrap_or(terms.entries.len()) as u32
            });
            ensure(
                slot <= terms.entries.len() as u32 && slot < crate::accounts::MAX_ENTRIES,
                "offer slot or maximum book capacity",
            )?;
            let offer = terms.entries.get(slot as usize).cloned();
            if let Some(offer) = &offer {
                ensure(offer.provider == config.provider, "offer authority")?;
            }
            ensure(
                offer.is_some() || !terms.entries.iter().any(|o| o.provider == config.provider),
                "one offer per provider and model",
            )?;
            if slot == state.limit {
                growth = Some(protocol.grow_model(keys, config.payer, state.limit + 1)?);
            }
            (keys, offer, slot)
        } else {
            ensure(desired.slot.is_none_or(|n| n == 0), "new model offer slot")?;
            let terms = Keypair::new();
            let capacity = Keypair::new();
            let keys = ModelKeys {
                model: model_key,
                terms: terms.pubkey(),
                capacity: capacity.pubkey(),
            };
            let (terms_rent, capacity_rent) = match book_rent {
                Some(rent) => rent,
                None => {
                    let (terms, capacity) =
                        tokio::join!(rpc.rent(terms_size), rpc.rent(capacity_size));
                    let rent = (terms?, capacity?);
                    book_rent = Some(rent);
                    rent
                }
            };
            let mut instructions = vec![
                protocol.create_account(config.payer, keys.terms, terms_rent, terms_size)?,
                protocol.create_account(
                    config.payer,
                    keys.capacity,
                    capacity_rent,
                    capacity_size,
                )?,
            ];
            instructions.push(protocol.initialize_model(InitializeModel {
                model: keys,
                provider: config.provider,
                payer: config.payer,
                id,
                limit: config.book_limit,
                document: &desired.document,
            })?);
            plan.steps.push(PlanStep {
                label: format!("initialize model {}", desired.id),
                instructions,
                signers: vec![terms, capacity],
            });
            (keys, None, 0)
        };
        let source = address::associated_token(&protocol.mint, &config.provider);
        let payout = offer.as_ref().map(|o| o.payout).unwrap_or(source);
        let keys = OfferKeys {
            model,
            provider: config.provider,
            payout,
            slot,
        };
        if let Some(offer) = &offer {
            if offer.terms != desired.terms {
                plan.steps.push(PlanStep::new(
                    format!("update offer {}", desired.id),
                    vec![protocol.update_offer(keys, desired.terms)?],
                ));
            }
        } else {
            let mut instructions: Vec<_> = growth.into_iter().collect();
            instructions.extend([
                create_associated_token(config.payer, config.provider, protocol.mint),
                protocol.register_offer(keys, desired.terms)?,
            ]);
            plan.steps.push(PlanStep::new(
                format!("register offer {}", desired.id),
                instructions,
            ));
        }
    }
    Ok(plan)
}

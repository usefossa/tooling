use super::*;
use clap::{Args, Subcommand};
use fossa_sdk::{
    Instruction,
    scoped::{
        ListCapacity, ListRouting, ListTerms, ProviderModelCapacity, ProviderPolicy, ProviderWallet,
    },
};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct Limits {
    #[arg(long, default_value_t = 60)]
    window_seconds: u32,
    #[arg(long, default_value_t = 0)]
    bookings_per_window: u32,
    #[arg(long, default_value_t = 0)]
    outstanding_bookings: u32,
}
impl Limits {
    fn value(self) -> BookingLimits {
        BookingLimits {
            wallet_window_seconds: self.window_seconds,
            wallet_bookings_per_window: self.bookings_per_window,
            wallet_booking_capacity: self.outstanding_bookings,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Council permanently enables list-isolated routing and stops legacy bookings
    #[command(after_help=DOCS)]
    Activate,
    /// Inspect selected list routing, provider policy and optional model allocation
    #[command(after_help=DOCS)]
    Show {
        #[arg(long)]
        model: Option<String>,
    },
    /// List owner initializes its routing registry
    #[command(after_help=DOCS)]
    Initialize {
        #[arg(long, default_value_t = 16)]
        limit: u32,
    },
    /// List owner grows its routing registry
    #[command(after_help=DOCS)]
    Grow {
        #[arg(long)]
        limit: u32,
    },
    /// List owner closes empty routing state after all books are closed
    #[command(after_help=DOCS)]
    Close,
    /// List owner admits or removes a provider; removal disables routing immediately
    #[command(after_help=DOCS)]
    Member {
        #[arg(long)]
        provider: Pubkey,
        #[arg(long,action=clap::ArgAction::Set)]
        enabled: bool,
    },
    /// Create or update provider limits; multiple enrolled lists require staged updates
    #[command(after_help=DOCS)]
    Policy {
        #[command(flatten)]
        limits: Limits,
    },
    /// Begin staged policy update; freeze every enrolled list before committing
    #[command(after_help=DOCS)]
    PolicyBegin,
    /// Commit new limits after every active mirror has been frozen
    #[command(after_help=DOCS)]
    PolicyCommit {
        #[command(flatten)]
        limits: Limits,
    },
    /// Provider opts into selected list using its current policy
    #[command(after_help=DOCS)]
    Enable,
    /// Provider or list owner freezes provider routing on selected list
    #[command(after_help=DOCS)]
    Freeze {
        #[arg(long)]
        provider: Option<Pubkey>,
    },
    /// Initialize provider-model total capacity, configure it, or reclaim drained legacy usage
    #[command(after_help=DOCS)]
    Capacity {
        #[arg(long)]
        model: String,
        #[arg(long)]
        total: Option<u32>,
        #[arg(long, conflicts_with = "total")]
        sync: bool,
    },
    /// List owner initializes, grows or closes model books
    #[command(after_help=DOCS)]
    Books {
        #[arg(long)]
        model: String,
        #[arg(long, conflicts_with = "close")]
        limit: Option<u32>,
        #[arg(long)]
        close: bool,
    },
    /// Set selected-list offer from OfferTerms JSON; capacity zero recovers allocation
    #[command(after_help=DOCS)]
    Offer {
        #[arg(long)]
        model: String,
        #[arg(long)]
        terms: PathBuf,
        #[arg(long)]
        payout: Option<Pubkey>,
    },
    /// Remove a departed provider's offer after its local bookings have settled
    #[command(after_help=DOCS)]
    PruneOffer {
        #[arg(long)]
        model: String,
        #[arg(long)]
        provider: Pubkey,
    },
    /// Initialize/grow wallet quota storage or prune/close expired, settled history
    #[command(after_help=DOCS)]
    Wallet {
        #[arg(long,conflicts_with_all=["prune","close"])]
        limit: Option<u32>,
        #[arg(long, conflicts_with = "close")]
        prune: bool,
        #[arg(long)]
        close: bool,
    },
}

async fn model_keys(rt: &Runtime, id: &str) -> Result<ModelKeys> {
    let at = super::model_address(rt, id)?;
    let model = rt
        .rpc
        .require_account::<Model>(at, rt.protocol.program)
        .await?;
    if model.market != rt.protocol.market {
        return Err(Error::Input("Model market mismatch".into()));
    }
    Ok(ModelKeys::from_account(at, &model))
}
async fn submit(rt: &Runtime, label: &str, ix: Vec<Instruction>) -> Result<Value> {
    rt.transact(
        label,
        ix,
        &[],
        json!({"provider_list":rt.config.provider_list.to_string()}),
    )
    .await
}
pub async fn configure_policy(rt: &Runtime, limits: BookingLimits) -> Result<Value> {
    limits.encode()?;
    let p = rt.protocol;
    let provider = rt.wallet()?;
    let list = rt.config.provider_list;
    let policy = rt
        .rpc
        .account::<ProviderPolicy>(
            address::provider_policy(&p.program, &p.market, &provider).0,
            p.program,
        )
        .await?;
    let Some(policy) = policy else {
        return submit(
            rt,
            "initialize provider policy",
            vec![p.initialize_provider_policy(provider, rt.payer()?, limits)?],
        )
        .await;
    };
    let routing = rt
        .rpc
        .account::<ListRouting>(address::list_routing(&p.program, &list).0, p.program)
        .await?;
    let enrolled = routing.as_ref().is_some_and(|state| {
        state
            .entries
            .iter()
            .any(|entry| entry.provider == provider && entry.active)
    });
    if policy.active_mirrors > u32::from(enrolled) {
        return Err(Error::Input("Multiple active lists: run routing policy-begin, routing freeze for every enrolled list, then routing policy-commit and routing enable on each list.".into()));
    }
    let mut ix = vec![];
    if !policy.updating {
        ix.push(p.begin_provider_policy_update(provider));
    }
    if enrolled {
        ix.push(p.freeze_policy_mirror(list, provider, provider));
    }
    ix.push(p.commit_provider_policy_update(provider, limits)?);
    if enrolled {
        ix.push(p.enable_policy_mirror(list, provider));
    }
    submit(rt, "configure provider policy", ix).await
}
pub async fn run(rt: &Runtime, command: Command) -> Result<Value> {
    let p = rt.protocol;
    let wallet = rt.wallet()?;
    let payer = rt.payer()?;
    let list = rt.config.provider_list;
    let ix = match command {
        Command::Activate => vec![p.activate_list_routing(wallet)],
        Command::Show { model } => {
            let policy = rt
                .rpc
                .account::<ProviderPolicy>(
                    address::provider_policy(&p.program, &p.market, &wallet).0,
                    p.program,
                )
                .await?;
            let routing = rt
                .rpc
                .account::<ListRouting>(address::list_routing(&p.program, &list).0, p.program)
                .await?;
            let usage = rt
                .rpc
                .account::<ProviderWallet>(
                    address::provider_wallet(&p.program, &p.market, &wallet).0,
                    p.program,
                )
                .await?;
            let mut value =
                json!({"provider_policy":policy,"routing":routing,"wallet_usage":usage});
            if let Some(id) = model {
                let m = model_keys(rt, &id).await?;
                value["capacity_budget"] = serde_json::to_value(
                    rt.rpc
                        .account::<ProviderModelCapacity>(
                            address::provider_capacity(&p.program, &m.model, &wallet).0,
                            p.program,
                        )
                        .await?,
                )?;
                value["terms"] = serde_json::to_value(
                    rt.rpc
                        .account::<ListTerms>(
                            address::list_terms(&p.program, &list, &m.model).0,
                            p.program,
                        )
                        .await?,
                )?;
            }
            return Ok(value);
        }
        Command::Initialize { limit } => {
            vec![p.initialize_list_routing(list, wallet, payer, limit)?]
        }
        Command::Grow { limit } => vec![p.grow_list_routing(list, wallet, payer, limit)],
        Command::Close => vec![p.close_list_routing(list, wallet)],
        Command::Member { provider, enabled } => {
            let policy = rt
                .rpc
                .account::<ProviderPolicy>(
                    address::provider_policy(&p.program, &p.market, &provider).0,
                    p.program,
                )
                .await?;
            vec![p.update_routing_list(list, wallet, provider, enabled, policy.is_some())]
        }
        Command::Policy { limits } => return configure_policy(rt, limits.value()).await,
        Command::PolicyBegin => vec![p.begin_provider_policy_update(wallet)],
        Command::PolicyCommit { limits } => {
            vec![p.commit_provider_policy_update(wallet, limits.value())?]
        }
        Command::Enable => vec![p.enable_policy_mirror(list, wallet)],
        Command::Freeze { provider } => {
            vec![p.freeze_policy_mirror(list, provider.unwrap_or(wallet), wallet)]
        }
        Command::Capacity { model, total, sync } => {
            let m = model_keys(rt, &model).await?;
            if sync {
                vec![p.sync_legacy_capacity(m, wallet)]
            } else {
                let total = total.ok_or_else(|| Error::Input("Set --total or --sync".into()))?;
                let existing = rt
                    .rpc
                    .account::<ProviderModelCapacity>(
                        address::provider_capacity(&p.program, &m.model, &wallet).0,
                        p.program,
                    )
                    .await?;
                vec![if existing.is_some() {
                    p.configure_model_capacity(m, wallet, total)
                } else {
                    p.initialize_model_capacity(m, wallet, payer, total)
                }]
            }
        }
        Command::Books {
            model,
            limit,
            close,
        } => {
            let m = model_keys(rt, &model).await?;
            if close {
                vec![p.close_list_books(list, m.model, wallet)]
            } else {
                let limit = limit.ok_or_else(|| Error::Input("Set --limit or --close".into()))?;
                let existing = rt
                    .rpc
                    .account::<ListTerms>(
                        address::list_terms(&p.program, &list, &m.model).0,
                        p.program,
                    )
                    .await?;
                vec![if existing.is_some() {
                    p.grow_list_books(list, m.model, wallet, payer, limit)
                } else {
                    p.initialize_list_books(list, m.model, wallet, payer, limit)?
                }]
            }
        }
        Command::Offer {
            model,
            terms,
            payout,
        } => {
            let m = model_keys(rt, &model).await?;
            let terms: OfferTerms = serde_json::from_slice(&std::fs::read(terms)?)?;
            vec![p.set_list_offer(
                list,
                m.model,
                wallet,
                payout.unwrap_or_else(|| address::associated_token(&p.mint, &wallet)),
                terms,
            )?]
        }
        Command::PruneOffer { model, provider } => {
            vec![p.prune_list_offer(list, model_keys(rt, &model).await?.model, provider)]
        }
        Command::Wallet {
            limit,
            prune,
            close,
        } => {
            let existing = rt
                .rpc
                .account::<ProviderWallet>(
                    address::provider_wallet(&p.program, &p.market, &wallet).0,
                    p.program,
                )
                .await?;
            if close {
                vec![
                    p.close_provider_wallet(
                        wallet,
                        existing
                            .ok_or_else(|| Error::Input("Wallet quota account missing".into()))?
                            .rent_payer,
                    ),
                ]
            } else if prune {
                vec![p.prune_provider_wallet(wallet)]
            } else {
                let limit =
                    limit.ok_or_else(|| Error::Input("Set --limit, --prune or --close".into()))?;
                vec![if existing.is_some() {
                    p.grow_provider_wallet(wallet, payer, limit)
                } else {
                    p.initialize_provider_wallet(wallet, payer, limit)?
                }]
            }
        }
    };
    submit(rt, "manage list routing", ix).await
}

pub async fn book(rt: &Runtime, args: BookArgs) -> Result<Value> {
    let p = rt.protocol;
    let owner = rt.wallet()?;
    let payer = rt.payer()?;
    let user = rt.rpc.require_account::<User>(args.user, p.program).await?;
    if user.owner != owner || user.market != p.market {
        return Err(Error::Input("User authority or market mismatch".into()));
    }
    let m = model_keys(rt, &args.model.model).await?;
    let request = super::book_request(&args, rt.config.provider_list);
    let list = request.provider_list;
    let list_state = rt
        .rpc
        .require_account::<ProviderList>(list, p.program)
        .await?;
    let terms = rt
        .rpc
        .require_account::<ListTerms>(
            address::list_terms(&p.program, &list, &m.model).0,
            p.program,
        )
        .await?;
    let capacity = rt
        .rpc
        .require_account::<ListCapacity>(
            address::list_capacity(&p.program, &list, &m.model).0,
            p.program,
        )
        .await?;
    let routing = rt
        .rpc
        .require_account::<ListRouting>(address::list_routing(&p.program, &list).0, p.program)
        .await?;
    let ledger = rt
        .rpc
        .account::<ProviderWallet>(
            address::provider_wallet(&p.program, &p.market, &owner).0,
            p.program,
        )
        .await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Input("Clock before epoch".into()))?
        .as_secs() as i64;
    let selected = request
        .select_scoped(
            &terms,
            &capacity,
            &list_state,
            &routing,
            ledger.as_ref(),
            user.free,
            now,
        )?
        .ok_or_else(|| {
            Error::Input("No eligible provider fits wallet, capacity and request limits".into())
        })?;
    let booking = Keypair::new();
    let u = UserKeys::from_account(args.user, &user);
    let mut ix = vec![fossa_sdk::transaction::compute_unit_limit(
        (20_000 + 500 * terms.entries.len().min(4096) as u32).min(1_400_000),
    )];
    if let Some(ledger) = ledger {
        if ledger.entries.len() == ledger.capacity as usize {
            ix.push(p.grow_provider_wallet(owner, payer, ledger.capacity + 8));
        }
    } else {
        ix.push(p.initialize_provider_wallet(owner, payer, 16)?);
    }
    ix.push(p.initialize_reusable_booking(u, booking.pubkey(), payer));
    ix.push(p.book_scoped(
        m,
        u,
        booking.pubkey(),
        payer,
        &request,
        instruction::BookingMode::OneShot,
    )?);
    rt.transact(
        "create list booking",
        ix,
        &[booking],
        json!({"selected":selected,"provider_list":list.to_string()}),
    )
    .await
}

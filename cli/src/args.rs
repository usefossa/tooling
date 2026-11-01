use clap::{Args, Parser, Subcommand, ValueEnum};
use fossa_sdk::{Pubkey, amount};
use std::path::PathBuf;

pub const DOCS: &str = "docs: https://fossa.ag/docs/providers/cli.md\nsdk: https://fossa.ag/docs/providers/sdk.md\nstart: https://fossa.ag/docs/providers/quick-start.md";

#[derive(Parser, Debug)]
#[command(name = "fossa", version, styles = help_styles(), about = "Inspect and manage Fossa accounts, providers and bookings; mutations plan until --execute", after_help = DOCS)]
pub struct Cli {
    #[command(flatten)]
    pub options: Options,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Args, Debug, Default)]
pub struct Options {
    /// Force compact JSON output and structured errors, including in terminals
    #[arg(long, global = true)]
    pub json: bool,
    /// Explicit TOML config; file paths inside resolve relative to its directory
    #[arg(long, global = true, env = "FOSSA_CONFIG", hide_env_values = true)]
    pub config: Option<PathBuf>,
    /// Own Solana RPC URL; never included in output
    #[arg(long, global = true, env = "FOSSA_RPC_URL", hide_env_values = true)]
    pub rpc_url: Option<String>,
    /// Fossa RPC defaults on without a custom RPC; reads sign authentication messages
    #[arg(
        long,
        global = true,
        env = "FOSSA_HOSTED_RPC",
        hide_env_values = true,
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    pub hosted_rpc: Option<bool>,
    /// Fossa API origin (default https://api.fossa.ag)
    #[arg(long, global = true, env = "FOSSA_API_URL", hide_env_values = true)]
    pub api_url: Option<String>,
    /// Standard Solana JSON keypair outside git
    #[arg(long, global = true, env = "FOSSA_KEYPAIR", hide_env_values = true)]
    pub keypair: Option<PathBuf>,
    /// Authority for planning/inspection without loading a signing key
    #[arg(long, global = true, env = "FOSSA_WALLET", hide_env_values = true)]
    pub wallet: Option<Pubkey>,
    /// Optional separate fee payer keypair
    #[arg(long, global = true, env = "FOSSA_PAYER", hide_env_values = true)]
    pub payer: Option<PathBuf>,
    /// Override the default Fossa program address
    #[arg(long, global = true, env = "FOSSA_PROGRAM_ID", hide_env_values = true)]
    pub program_id: Option<Pubkey>,
    /// Override the default market address
    #[arg(long, global = true, env = "FOSSA_MARKET", hide_env_values = true)]
    pub market: Option<Pubkey>,
    /// Default provider list for bookings; a booking --list overrides this value
    #[arg(
        long,
        global = true,
        env = "FOSSA_PROVIDER_LIST",
        hide_env_values = true
    )]
    pub provider_list: Option<Pubkey>,
    /// Override the default USDC mint
    #[arg(long, global = true, env = "FOSSA_USDC_MINT", hide_env_values = true)]
    pub usdc_mint: Option<Pubkey>,
    /// Authorize this command's uploads, transaction signing and broadcast
    #[arg(long, global = true)]
    pub execute: bool,
    /// Durable transaction directory; defaults to transactions beside config (or cwd)
    #[arg(long, global = true, env = "FOSSA_JOURNAL_DIR", hide_env_values = true)]
    pub journal_dir: Option<PathBuf>,
    /// Confirmation wait; timeout preserves pending transaction for resume
    #[arg(long, global = true, default_value_t = 30)]
    pub confirmation_seconds: u64,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Inspect effective configuration without exposing RPC credentials
    #[command(subcommand, after_help = DOCS)]
    Config(ConfigCommand),
    /// Reconcile configured provider, model metadata and offers
    #[command(after_help = DOCS)]
    Onboard,
    /// Inspect and manage owner-controlled provider lists
    #[command(subcommand, after_help = DOCS)]
    List(ListCommand),
    /// Inspect and edit provider profile
    #[command(subcommand, after_help = DOCS)]
    Provider(ProviderCommand),
    /// Inspect model accounts or update metadata owned by your wallet
    #[command(subcommand, after_help = DOCS)]
    Model(ModelCommand),
    /// Manage shared model markets and redeem their backing
    #[command(subcommand, after_help = DOCS)]
    Group(GroupCommand),
    /// Inspect and configure router governance policy
    #[command(subcommand, after_help = DOCS)]
    Council(CouncilCommand),
    /// Inspect offers; edit selected fields while preserving others
    #[command(subcommand, after_help = DOCS)]
    Offer(OfferCommand),
    /// Validate exact document bytes offline or upload immutable metadata
    #[command(subcommand, after_help = DOCS)]
    Metadata(MetadataCommand),
    /// Upload an immutable PNG, JPEG or WebP image
    #[command(subcommand, after_help = DOCS)]
    Image(ImageCommand),
    /// Read and decode an account with SDK ownership/layout validation
    #[command(subcommand, after_help = DOCS)]
    Account(AccountCommand),
    /// Inspect or safely resume a durable signed transaction
    #[command(subcommand, after_help = DOCS)]
    Transaction(TransactionCommand),
    /// User account operations for integration workflows; no inference requests
    #[command(subcommand, after_help = DOCS)]
    User(UserCommand),
    /// Inspect and settle bookings; all mutation commands plan by default
    #[command(subcommand, after_help = DOCS)]
    Booking(BookingCommand),
}
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Show selected configuration and defaults (RPC URL redacted)
    #[command(after_help = DOCS)]
    Show,
}
#[derive(Subcommand, Debug)]
pub enum ProviderCommand {
    /// Inspect provider PDA and committed metadata reference
    #[command(after_help = DOCS)]
    Show,
    /// Register a provider profile
    #[command(after_help = DOCS)]
    Register(DocumentArgs),
    /// Edit a verified existing profile, preserving unspecified fields and extensions
    #[command(after_help = DOCS)]
    Update(ProfileArgs),
}
#[derive(Args, Debug)]
pub struct DocumentArgs {
    /// Valid metadata JSON; exact bytes determine SHA-256
    #[arg(long)]
    pub metadata_file: PathBuf,
    /// Custom immutable HTTPS location; omit for Fossa hosting on --execute
    #[arg(long)]
    pub metadata_uri: Option<String>,
}
#[derive(Args, Debug)]
#[group(required = true, multiple = true)]
pub struct ProfileArgs {
    /// Replace display name
    #[arg(long)]
    pub name: Option<String>,
    /// Replace image with an existing HTTPS image URL
    #[arg(long, conflicts_with = "image_file")]
    pub image_url: Option<String>,
    /// Upload local PNG/JPEG/WebP before publishing updated profile
    #[arg(long, conflicts_with = "image_url")]
    pub image_file: Option<PathBuf>,
    /// Replace document with this validated file instead of fetching current metadata
    #[arg(long, conflicts_with_all = ["name", "image_url", "image_file"])]
    pub metadata_file: Option<PathBuf>,
    /// Custom immutable URI for the resulting exact document bytes
    #[arg(long, requires = "metadata_file")]
    pub metadata_uri: Option<String>,
}
#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    /// Inspect model, books and metadata reference
    #[command(after_help = DOCS)]
    Show(ModelArg),
    /// Council assigns an existing shared model market
    #[command(after_help = DOCS)]
    AssignGroup {
        #[command(flatten)]
        model: ModelArg,
        #[arg(long)]
        group: Pubkey,
    },
    /// Council sends future bookings' protocol fees to treasury
    #[command(after_help = DOCS)]
    ClearGroup(ModelArg),
    /// Increase permanent offer slots; any payer may fund rent for up to 77 new slots
    #[command(after_help = DOCS)]
    Grow {
        #[command(flatten)]
        model: ModelArg,
        /// New total slot limit, at most 4096
        #[arg(long, value_parser = clap::value_parser!(u32).range(2..=4096))]
        limit: u32,
    },
    /// Change model metadata; signer must own model metadata authority
    #[command(after_help = DOCS)]
    UpdateMetadata {
        #[command(flatten)]
        model: ModelArg,
        #[command(flatten)]
        document: DocumentArgs,
    },
}
#[derive(Subcommand, Debug)]
pub enum GroupCommand {
    /// Inspect shared market, token metadata and USDC backing
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        group: Pubkey,
    },
    /// Launch using prepared InitializeModelGroup JSON accounts and GroupInit JSON parameters
    #[command(after_help = DOCS)]
    Create {
        #[arg(long)]
        accounts_file: PathBuf,
        #[arg(long)]
        parameters_file: PathBuf,
        /// Fresh backing signer; required only with --execute
        #[arg(long)]
        backing_keypair: Option<PathBuf>,
        /// Fresh curve LP signer; required only with --execute
        #[arg(long)]
        curve_lp_keypair: Option<PathBuf>,
    },
    /// Council replaces token name, symbol and URI
    #[command(after_help = DOCS)]
    UpdateMetadata {
        #[arg(long)]
        group: Pubkey,
        #[arg(long)]
        name: String,
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        uri: String,
    },
    /// Council configures future fee routing and token transfer tax
    #[command(after_help = DOCS)]
    ConfigureReserve {
        #[arg(long)]
        group: Pubkey,
        #[arg(long, action = clap::ArgAction::Set)]
        route_to_reserve: bool,
        #[arg(long, value_parser = clap::value_parser!(u16).range(0..=10000))]
        tax_bps: u16,
        /// Maximum transfer tax in model-token atomic units
        #[arg(long)]
        maximum_tax: u64,
    },
    /// Burn model-token atoms for proportional USDC backing
    #[command(after_help = DOCS)]
    Redeem {
        #[arg(long)]
        group: Pubkey,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        amount: u64,
        /// Minimum USDC received, in atomic units
        #[arg(long)]
        minimum_output: u64,
        #[arg(long)]
        source: Option<Pubkey>,
        #[arg(long)]
        destination: Option<Pubkey>,
    },
    /// Permissionlessly collect harvested model-token tax into bank's token account
    #[command(after_help = DOCS)]
    CollectTax {
        #[arg(long)]
        group: Pubkey,
        #[arg(long)]
        destination: Pubkey,
    },
    /// Sweep residual backing to treasury only after token supply reaches zero
    #[command(after_help = DOCS)]
    Sweep {
        #[arg(long)]
        group: Pubkey,
        #[arg(long)]
        destination: Option<Pubkey>,
    },
}
#[derive(Subcommand, Debug)]
pub enum CouncilCommand {
    /// Inspect council, bank, list-creation policy and configured DEX
    #[command(after_help = DOCS)]
    Show,
    /// Patch list-creation policy; omitted fields retain current settings
    #[command(after_help = DOCS)]
    ConfigureFees {
        #[arg(long)]
        bank: Option<Pubkey>,
        #[arg(long)]
        creation_mint: Option<Pubkey>,
        #[arg(long)]
        creation_token_program: Option<Pubkey>,
        /// Required net payment in governance-token atomic units
        #[arg(long)]
        creation_net_price: Option<u64>,
        #[arg(long, action = clap::ArgAction::Set)]
        creation_enabled: Option<bool>,
    },
    /// Bind the router to an initialized, council-authorized DEX
    #[command(after_help = DOCS)]
    ConfigureDex {
        #[arg(long)]
        program: Pubkey,
    },
    /// Rotate council and treasury together; both wallets must authorize
    #[command(after_help = DOCS)]
    Rotate {
        #[arg(long)]
        next: Pubkey,
        /// Next council signer; required only with --execute
        #[arg(long)]
        next_keypair: Option<PathBuf>,
    },
}
#[derive(Args, Debug)]
pub struct ModelArg {
    /// Exact model identifier (hashed by SDK)
    #[arg(long)]
    pub model: String,
}
#[derive(Args, Debug)]
pub struct OfferArg {
    #[command(flatten)]
    pub model: ModelArg,
    /// Offer slot; omit only when exactly one belongs to the configured wallet
    #[arg(long)]
    pub slot: Option<u32>,
}
#[derive(Subcommand, Debug)]
pub enum OfferCommand {
    /// Show every offer and capacity entry for a model
    #[command(after_help = DOCS)]
    List(ModelArg),
    /// Patch only named fields; prices are USDC per 1,000,000 tokens
    #[command(after_help = DOCS)]
    Update {
        #[command(flatten)]
        offer: OfferArg,
        #[command(flatten)]
        patch: OfferPatch,
    },
}
#[derive(Args, Debug, Default)]
#[group(required = true, multiple = true)]
pub struct OfferPatch {
    /// immutable checkpoint identity; SHA-256 of exact UTF-8 bytes is stored on chain
    #[arg(long, value_parser = parse_checkpoint)]
    pub checkpoint: Option<[u8; 32]>,
    /// comma-separated input types: text,image,audio,video
    #[arg(long, value_parser = parse_modalities)]
    pub input_modalities: Option<u8>,
    /// comma-separated output types: text,image,audio,video
    #[arg(long, value_parser = parse_modalities)]
    pub output_modalities: Option<u8>,
    /// offered context window shared by input and output tokens
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub context_tokens: Option<u32>,
    /// offered maximum conservative input reservation
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub input_bound_tokens: Option<u32>,
    /// offered maximum output reservation
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub output_tokens: Option<u32>,
    /// USDC per million uncached input tokens, at most 6 decimals
    #[arg(long, value_parser = parse_positive_rate)]
    pub input_price: Option<u32>,
    /// USDC per million cached input tokens, at most 6 decimals
    #[arg(long, value_parser = parse_rate)]
    pub cached_price: Option<u32>,
    /// USDC per million output tokens, at most 6 decimals
    #[arg(long, value_parser = parse_positive_rate)]
    pub output_price: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub capacity: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub speed_tps: Option<u16>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub active: Option<bool>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub private: Option<bool>,
}
#[derive(Subcommand, Debug)]
pub enum ListCommand {
    /// Read list authority and provider public keys
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        list: Pubkey,
        /// Fetch and verify the profile JSON against its on-chain hash
        #[arg(long)]
        fetch_metadata: bool,
    },
    /// Create an empty provider list owned by the configured wallet
    #[command(after_help = DOCS)]
    Create {
        #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(1..=4096))]
        capacity: u32,
        /// Inference surcharge in basis points (10-300)
        #[arg(long, value_parser = clap::value_parser!(u16).range(10..=300))]
        fee_bps: u16,
        /// Protocol share of the surcharge in basis points (5000-10000)
        #[arg(long, value_parser = clap::value_parser!(u16).range(5000..=10000))]
        protocol_share_bps: u16,
        /// Maximum creation-token debit in atomic units
        #[arg(long)]
        maximum_payment: u64,
        /// Optional profile JSON; exact bytes determine SHA-256
        #[arg(long)]
        metadata_file: Option<PathBuf>,
        /// Custom immutable HTTPS location; omit for Fossa hosting on --execute
        #[arg(long, requires = "metadata_file")]
        metadata_uri: Option<String>,
    },
    /// Create or update your provider list metadata
    #[command(after_help = DOCS)]
    Update {
        #[arg(long)]
        list: Pubkey,
        #[command(flatten)]
        profile: ListProfileArgs,
    },
    /// Update list surcharge and protocol share for future bookings
    #[command(after_help = DOCS)]
    ConfigureFees {
        #[arg(long)]
        list: Pubkey,
        #[arg(long, value_parser = clap::value_parser!(u16).range(10..=300))]
        fee_bps: u16,
        #[arg(long, value_parser = clap::value_parser!(u16).range(5000..=10000))]
        protocol_share_bps: u16,
    },
    /// Add a provider public key to your list
    #[command(after_help = DOCS)]
    Add {
        #[arg(long)]
        list: Pubkey,
        #[arg(long)]
        provider: Pubkey,
    },
    /// Remove a provider public key from your list
    #[command(after_help = DOCS)]
    Remove {
        #[arg(long)]
        list: Pubkey,
        #[arg(long)]
        provider: Pubkey,
    },
    /// Close your empty list and recover account rent
    #[command(after_help = DOCS)]
    Close {
        #[arg(long)]
        list: Pubkey,
    },
}
#[derive(Args, Debug, Default)]
#[group(required = true, multiple = true)]
pub struct ListProfileArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long, conflicts_with_all = ["image_file", "remove_image"])]
    pub image_url: Option<String>,
    /// Upload local PNG/JPEG/WebP profile picture
    #[arg(long, conflicts_with = "remove_image")]
    pub image_file: Option<PathBuf>,
    #[arg(long)]
    pub remove_image: bool,
    #[arg(long, conflicts_with = "remove_website")]
    pub website: Option<String>,
    #[arg(long)]
    pub remove_website: bool,
    /// Six-digit hexadecimal color, for example #8B7ACF
    #[arg(long)]
    pub accent_color: Option<String>,
    #[arg(long, conflicts_with_all = ["icon_file", "remove_icon"])]
    pub icon_url: Option<String>,
    /// Upload a simple white SVG icon, at most 64 KiB
    #[arg(long, conflicts_with = "remove_icon")]
    pub icon_file: Option<PathBuf>,
    #[arg(long)]
    pub remove_icon: bool,
    /// Replace profile with exact validated JSON bytes
    #[arg(long, conflicts_with_all = ["name", "image_url", "image_file", "remove_image", "website", "remove_website", "accent_color", "icon_url", "icon_file", "remove_icon"])]
    pub metadata_file: Option<PathBuf>,
    #[arg(long, requires = "metadata_file")]
    pub metadata_uri: Option<String>,
}
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum MetadataKind {
    Provider,
    Model,
    List,
}
impl From<MetadataKind> for fossa_sdk::metadata::Kind {
    fn from(value: MetadataKind) -> Self {
        match value {
            MetadataKind::Provider => Self::Provider,
            MetadataKind::Model => Self::Model,
            MetadataKind::List => Self::List,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum MetadataCommand {
    /// Validate local bytes offline; print hash, size and parsed document
    #[command(after_help = DOCS)]
    Validate { kind: MetadataKind, file: PathBuf },
    /// Plan or upload exact local bytes; upload alone never updates on-chain metadata
    #[command(after_help = DOCS)]
    Upload { kind: MetadataKind, file: PathBuf },
}
#[derive(Subcommand, Debug)]
pub enum ImageCommand {
    /// Plan or upload immutable image bytes (maximum 2 MiB)
    #[command(after_help = DOCS)]
    Upload { file: PathBuf },
}
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum AccountKind {
    Market,
    Provider,
    Model,
    ModelMetadata,
    ModelGroup,
    User,
    Terms,
    Capacity,
    Booking,
    ProviderList,
    ListMetadata,
    Token,
}
#[derive(Subcommand, Debug)]
pub enum AccountCommand {
    /// Fetch typed account; rejects incorrect owner, size, magic or PDA binding
    #[command(after_help = DOCS)]
    Show {
        address: Pubkey,
        #[arg(long)]
        kind: AccountKind,
    },
}
#[derive(Subcommand, Debug)]
pub enum TransactionCommand {
    /// Reconcile saved signature with chain; never deletes or broadcasts
    #[command(after_help = DOCS)]
    Status {
        #[arg(long)]
        file: PathBuf,
    },
    /// Plan or resume exact signed bytes after status/expiry reconciliation
    #[command(after_help = DOCS)]
    Resume {
        #[arg(long)]
        file: PathBuf,
    },
}
#[derive(Subcommand, Debug)]
pub enum UserCommand {
    /// Inspect an existing user account
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        user: Pubkey,
    },
    /// Create user account and its USDC vault; returns generated account address
    #[command(after_help = DOCS)]
    Init,
    /// Deposit USDC into a user account owned by signer
    #[command(after_help = DOCS)]
    Deposit {
        #[arg(long)]
        user: Pubkey,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
        #[arg(long)]
        source: Option<Pubkey>,
    },
    /// Withdraw free USDC from a user account owned by signer
    #[command(after_help = DOCS)]
    Withdraw {
        #[arg(long)]
        user: Pubkey,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
        #[arg(long)]
        destination: Option<Pubkey>,
    },
}
#[derive(Subcommand, Debug)]
pub enum BookingCommand {
    /// Create one-shot booking from capped prices; never sends an inference request
    #[command(after_help = DOCS)]
    Create(BookArgs),
    /// Inspect booking state
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        booking: Pubkey,
    },
    /// Claim measured token usage and publish reusable booking receipt; no inference is performed
    #[command(after_help = DOCS)]
    Claim {
        #[arg(long)]
        booking: Pubkey,
        #[arg(long)]
        uncached: u32,
        #[arg(long, default_value_t = 0)]
        cached: u32,
        #[arg(long)]
        output: u32,
    },
    /// Provider releases booking without charging; generation protects reusable bookings
    #[command(after_help = DOCS)]
    Release {
        #[arg(long)]
        booking: Pubkey,
    },
    /// Expire booking after its claim deadline
    #[command(after_help = DOCS)]
    Expire {
        #[arg(long)]
        booking: Pubkey,
    },
}
pub fn parse_rate(value: &str) -> Result<u32, String> {
    amount::parse_usdc(value)
        .map_err(|_| "expected nonnegative USDC price with at most 6 decimals".into())
        .and_then(|n| {
            u32::try_from(n).map_err(|_| "price exceeds 4294.967295 USDC per million tokens".into())
        })
}
pub fn parse_positive_usdc(value: &str) -> Result<u64, String> {
    let n = amount::parse_usdc(value)
        .map_err(|_| "expected positive USDC amount with at most 6 decimals")?;
    if n == 0 {
        Err("amount must be positive".into())
    } else {
        Ok(n)
    }
}

#[derive(Args, Debug)]
pub struct BookArgs {
    /// minimum provider context window in tokens
    #[arg(long, default_value_t = 0)]
    pub min_context: u32,
    /// record that manual token limits are configured
    #[arg(long)]
    pub limits_configured: bool,
    /// exact immutable checkpoint identity advertised by the model metadata
    #[arg(long, value_parser = parse_checkpoint)]
    pub checkpoint: [u8; 32],
    #[arg(long, default_value = "text", value_parser = parse_modalities)]
    pub input_modalities: u8,
    #[arg(long, default_value = "text", value_parser = parse_modalities)]
    pub output_modalities: u8,
    /// Provider list constraining cheapest eligible matching
    #[arg(long)]
    pub list: Option<Pubkey>,
    #[arg(long)]
    pub user: Pubkey,
    #[command(flatten)]
    pub model: ModelArg,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_input: u32,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_output: u32,
    /// Maximum reservation in USDC, at most 6 decimals
    #[arg(long, value_parser = parse_positive_usdc)]
    pub max_spend: u64,
    #[arg(long, value_parser = parse_positive_rate)]
    pub input_price_cap: u32,
    #[arg(long, value_parser = parse_rate)]
    pub cached_price_cap: u32,
    #[arg(long, value_parser = parse_positive_rate)]
    pub output_price_cap: u32,
    #[arg(long, default_value_t = 1)]
    pub min_speed: u16,
    #[arg(long)]
    pub private: bool,
    #[arg(long)]
    pub estimated_input: Option<u32>,
    #[arg(long)]
    pub estimated_output: Option<u32>,
    #[arg(long, default_value_t = 0)]
    pub estimated_cached: u32,
    #[arg(long)]
    pub cache_provider: Option<Pubkey>,
}
pub fn parse_positive_rate(value: &str) -> Result<u32, String> {
    let n = parse_rate(value)?;
    if n == 0 {
        Err("price must be positive".into())
    } else {
        Ok(n)
    }
}

pub fn help_styles() -> clap::builder::Styles {
    use clap::builder::styling::AnsiColor;
    clap::builder::Styles::styled()
        .header(AnsiColor::Cyan.on_default().bold())
        .usage(AnsiColor::Cyan.on_default().bold())
        .literal(AnsiColor::Green.on_default().bold())
        .placeholder(AnsiColor::Yellow.on_default())
        .error(AnsiColor::Red.on_default().bold())
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default())
}

impl Cli {
    pub fn parse_options() -> Result<Self, clap::Error> {
        use clap::{CommandFactory, FromArgMatches, parser::ValueSource};
        let matches = Self::command()
            .color(crate::output::help_color())
            .try_get_matches()?;
        let mut cli = Self::from_arg_matches(&matches)?;
        if cli.options.hosted_rpc == Some(true) && cli.options.rpc_url.is_some() {
            match (
                matches.value_source("rpc_url"),
                matches.value_source("hosted_rpc"),
            ) {
                (Some(ValueSource::CommandLine), Some(ValueSource::EnvVariable)) => {
                    cli.options.hosted_rpc = Some(false)
                }
                (Some(ValueSource::EnvVariable), Some(ValueSource::CommandLine)) => {
                    cli.options.rpc_url = None
                }
                _ => {
                    let mut error = clap::Error::raw(
                        clap::error::ErrorKind::ArgumentConflict,
                        "select one RPC transport",
                    );
                    error.insert(
                        clap::error::ContextKind::InvalidArg,
                        clap::error::ContextValue::String("--rpc-url".into()),
                    );
                    error.insert(
                        clap::error::ContextKind::PriorArg,
                        clap::error::ContextValue::String("--hosted-rpc".into()),
                    );
                    return Err(error);
                }
            }
        }
        Ok(cli)
    }
}

pub fn parse_checkpoint(value: &str) -> Result<[u8; 32], String> {
    if value.is_empty()
        || value.chars().count() > 256
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(
            "expected immutable checkpoint identity without whitespace, at most 256 characters"
                .into(),
        );
    }
    Ok(fossa_sdk::address::model_id(value))
}
pub fn parse_modalities(value: &str) -> Result<u8, String> {
    value.split(',').try_fold(0, |mask, name| {
        let bit = match name {
            "text" => 1,
            "image" => 2,
            "audio" => 4,
            "video" => 8,
            _ => return Err("expected comma-separated text,image,audio,video".into()),
        };
        if mask & bit != 0 {
            return Err("duplicate modality".into());
        }
        Ok(mask | bit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn command_tree_is_consistent_and_every_help_links_docs() {
        let mut command = Cli::command();
        command.clone().debug_assert();
        fn visit(command: &mut clap::Command) {
            if command.get_name() == "help" {
                return;
            }
            let help = command.render_long_help().to_string();
            for url in [
                "https://fossa.ag/docs/providers/cli.md",
                "https://fossa.ag/docs/providers/sdk.md",
                "https://fossa.ag/docs/providers/quick-start.md",
            ] {
                assert!(help.contains(url), "{} missing docs", command.get_name());
            }
            for sub in command.get_subcommands_mut() {
                visit(sub);
            }
        }
        visit(&mut command);
    }
    #[test]
    fn prices_and_amounts_never_use_float_rounding() {
        assert_eq!(parse_rate("1.234567").unwrap(), 1_234_567);
        assert_eq!(parse_rate("4294.967295").unwrap(), u32::MAX);
        for bad in ["1e2", "-1", "0.0000001", "NaN", "4294.967296"] {
            assert!(parse_rate(bad).is_err());
        }
        assert!(parse_positive_usdc("0").is_err());
        assert!(
            Cli::try_parse_from(["fossa", "offer", "update", "--model", "example/model"]).is_err()
        );
        assert!(
            Cli::try_parse_from([
                "fossa",
                "offer",
                "update",
                "--model",
                "example/model",
                "--input-price",
                "0"
            ])
            .is_err()
        );
    }
}

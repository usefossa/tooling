// the sandbox runs the same packaged tokenizer and chat-template code as the clients.
// only JSON data enters it from model listings. no files, network or module loader.
use anyhow::{Context as _, Result, ensure};
use rquickjs::{Context, Function, Runtime};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const CACHE_SIZE: usize = 2;
type Cached = (String, Arc<OnceCell<Arc<Mutex<TextCounter>>>>);

pub struct Counters {
    entries: Mutex<VecDeque<Cached>>,
    active: Arc<Semaphore>,
}
impl Default for Counters {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            active: Arc::new(Semaphore::new(CACHE_SIZE)),
        }
    }
}
impl Counters {
    pub async fn count<F, E>(
        &self,
        reference: &Value,
        body: &Value,
        fetch: impl FnOnce(String, usize) -> F,
    ) -> Result<u32>
    where
        F: std::future::Future<Output = std::result::Result<Vec<u8>, E>>,
        E: std::fmt::Display,
    {
        ensure!(
            reference["format"] == "huggingface-chat-v1",
            "Unsupported text counter"
        );
        let hash = reference["sha256"]
            .as_str()
            .context("Missing tokenizer digest")?;
        let url = reference["url"].as_str().context("Missing tokenizer URL")?;
        let permit = self.active.clone().acquire_owned().await?;
        let cell = {
            let mut entries = self.entries.lock().await;
            let cell = entries
                .iter()
                .position(|(digest, _)| digest == hash)
                .and_then(|index| entries.remove(index))
                .map(|(_, cell)| cell)
                .unwrap_or_default();
            entries.push_front((hash.to_owned(), Arc::clone(&cell)));
            entries.truncate(CACHE_SIZE);
            cell
        };
        let engine = cell
            .get_or_try_init(|| async {
                let bytes = fetch(url.to_owned(), MAX_BYTES)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    bytes.len() <= MAX_BYTES && hex::encode(Sha256::digest(&bytes)) == hash,
                    "Tokenizer digest mismatch"
                );
                Ok::<_, anyhow::Error>(Arc::new(Mutex::new(
                    tokio::task::spawn_blocking(move || TextCounter::load(&bytes)).await??,
                )))
            })
            .await?;
        let body = serde_json::to_vec(body)?;
        // keep concurrent calls to one checkpoint in order, including its execution deadline.
        let guard = engine.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            guard.count(&body)
        })
        .await?
    }
}

pub struct TextCounter {
    context: Context,
    runtime: Runtime,
}

fn limit(runtime: &Runtime) {
    let deadline = Instant::now() + Duration::from_secs(5);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
}

impl TextCounter {
    pub fn load(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "Tokenizer too large");
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(256 << 20);
        runtime.set_max_stack_size(1024 * 1024);
        limit(&runtime);
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> rquickjs::Result<()> {
            ctx.eval::<(), _>(r#"
                globalThis.console = {warn() { throw Error("Unsupported tokenizer"); }};
                globalThis.TextEncoder = class { encode(text) { return Uint8Array.from(unescape(encodeURIComponent(text)), c => c.charCodeAt(0)); } };
                globalThis.TextDecoder = class { decode(bytes) { return decodeURIComponent(Array.from(bytes, b => "%" + b.toString(16).padStart(2, "0")).join("")); } };
            "#)?;
            for (name, source) in [
                ("Tokenizer", include_str!("../tokenizers-0.2.0.cjs")),
                ("Template", include_str!("../jinja-0.5.13.cjs")),
            ] {
                ctx.eval::<(), _>(format!("globalThis.{name}=(()=>{{let module={{exports:{{}}}};{source}\nreturn module.exports.{name};}})()"))?;
            }
            let core = include_str!("../core.js").replacen("export function", "function", 1);
            ctx.eval::<(), _>(core)?;
            ctx.globals().set("bundle", ctx.json_parse(bytes)?)?;
            ctx.eval::<(), _>("globalThis.countPrompt = makeCounter(Tokenizer, Template, bundle); delete globalThis.bundle;")
        })?;
        Ok(Self { context, runtime })
    }

    pub fn count(&self, body: &[u8]) -> Result<u32> {
        limit(&self.runtime);
        let tokens = self.context.with(|ctx| {
            let count: Function = ctx.globals().get("countPrompt")?;
            count.call::<_, u32>((ctx.json_parse(body)?,))
        })?;
        ensure!(tokens > 0, "Invalid token count");
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_chat_and_tools_with_same_reference_as_clients() {
        let bytes = include_bytes!("../fixture.json");
        let bundle: Value = serde_json::from_slice(bytes).unwrap();
        let engine = TextCounter::load(bytes).unwrap();
        for case in bundle["checks"].as_array().unwrap() {
            assert_eq!(
                u64::from(engine.count(&serde_json::to_vec(case).unwrap()).unwrap()),
                case["tokens"].as_u64().unwrap()
            );
        }
        let mut broken = bundle;
        broken["checks"][0]["tokens"] = 1.into();
        assert!(TextCounter::load(&serde_json::to_vec(&broken).unwrap()).is_err());
    }
}

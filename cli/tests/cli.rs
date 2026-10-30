use base64::{Engine, engine::general_purpose::STANDARD};
use fossa_sdk::{
    Hash, Keypair, PendingTransaction, Protocol, Pubkey, Signer, transaction::PreparedTransaction,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
struct Mock {
    url: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(j) = self.join.take() {
            j.join().unwrap();
        }
    }
}
fn mock(handler: impl Fn(&str, &Value) -> Value + Send + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let exit = stop.clone();
    let join = thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if exit.load(Ordering::SeqCst) {
                break;
            }
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend(&buf[..n]);
                if let Some(n) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break n + 4;
                }
            };
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let len = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|s| s.parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < end + len {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend(&buf[..n]);
            }
            let request: Value = serde_json::from_slice(&bytes[end..end + len]).unwrap();
            let path = head.split_whitespace().nth(1).unwrap();
            let body = handler(path, &request).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    Mock {
        url,
        stop,
        join: Some(join),
    }
}
fn cmd(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fossa"));
    c.env_clear().current_dir(dir);
    c
}
fn result(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
}
fn protocol_args(c: &mut Command, rpc: &str, wallet: Pubkey) {
    c.args([
        "--rpc-url",
        rpc,
        "--program-id",
        &key(1).to_string(),
        "--market",
        &key(2).to_string(),
        "--usdc-mint",
        &key(3).to_string(),
        "--wallet",
        &wallet.to_string(),
    ]);
}
fn rpc(req: &Value, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":req["id"],"result":value})
}
fn market_account(admin: Pubkey) -> Value {
    let mut data = <fossa_sdk::accounts::Market as fossa_sdk::accounts::Decode>::MAGIC.to_vec();
    data.extend(admin.as_ref());
    data.extend(key(3).as_ref());
    data.resize(fossa_sdk::accounts::MARKET_LEN, 0);
    data[104..136].copy_from_slice(key(4).as_ref());
    data[136..168].copy_from_slice(fossa_sdk::address::TOKEN_PROGRAM.as_ref());
    data[168..176].copy_from_slice(&1u64.to_le_bytes());
    data[176] = 1;
    json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(data),"base64"]})
}
fn signed() -> PendingTransaction {
    let signer = Keypair::new();
    let protocol = Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    };
    PreparedTransaction::new(
        &[protocol.update_list(key(5), signer.pubkey(), key(6), false)],
        signer.pubkey(),
        Hash::new_from_array([8; 32]),
        100,
        9,
    )
    .unwrap()
    .sign(&[&signer])
    .unwrap()
}
#[test]
fn version_and_offline_validation_need_no_config_network_or_keys() {
    let dir = tempfile::tempdir().unwrap();
    let version = cmd(dir.path()).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("fossa {}\n", env!("CARGO_PKG_VERSION"))
    );
    let raw = b"{ \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\" }\n";
    std::fs::write(dir.path().join("provider.json"), raw).unwrap();
    let value = result(
        cmd(dir.path())
            .args([
                "--config",
                "missing.toml",
                "metadata",
                "validate",
                "provider",
                "provider.json",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["sha256"], fossa_sdk::metadata::sha256_hex(raw));
    assert_eq!(value["bytes"], raw.len());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn env_config_overrides_and_urls_remain_redacted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"),format!("[solana]\nrpc_url='https://example.com/?api-key=never-print-this'\n[provider]\nwallet='{}'\n",key(5))).unwrap();
    let value = result(
        cmd(dir.path())
            .env("FOSSA_CONFIG", "config.toml")
            .env("FOSSA_WALLET", key(6).to_string())
            .args(["config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["wallet"], key(6).to_string());
    assert!(!value.to_string().contains("never-print-this"));
    assert!(!dir.path().join("transactions").exists());
}
#[test]
fn invalid_metadata_and_prices_fail_before_rpc_or_key_loading() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.json"), "{}").unwrap();
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            "http://127.0.0.1:1",
            "--keypair",
            "missing.json",
            "--execute",
            "provider",
            "register",
            "--metadata-file",
            "bad.json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("metadata")
    );
    assert!(!dir.path().join("transactions").exists());
    let output = cmd(dir.path())
        .args(["offer", "update", "--model", "m", "--input-price", "1.1e3"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stderr).is_ok());
}
#[test]
fn provider_registration_default_plan_never_uploads_signs_or_writes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("provider.json"),
        r#"{"name":"Example","endpoint":"https://example.com/infer"}"#,
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.to_owned());
        assert_eq!(method, "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(4))}else{Value::Null}}),
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let value = result(
        command
            .args([
                "--keypair",
                "missing.json",
                "provider",
                "register",
                "--metadata-file",
                "provider.json",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["mode"], "plan");
    assert_eq!(
        value["instructions"][0]["discriminator"],
        instruction_discriminator("initialize_provider")
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(!dir.path().join("transactions").exists());
}
#[test]
fn fresh_execution_simulates_saves_before_broadcast_and_archives_confirmed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    let wallet = signer.pubkey();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let pending_path = dir.path().join("transactions/pending.json");
    let journal = pending_path.clone();
    let wire = Arc::new(Mutex::new(None::<String>));
    let seen_wire = wire.clone();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        let value = match method {
            "getAccountInfo" => json!({"context":{"slot":9},"value":market_account(key(4))}),
            "getMinimumBalanceForRentExemption" => json!(1000),
            "getLatestBlockhash" => {
                json!({"context":{"slot":9},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}})
            }
            "simulateTransaction" => {
                assert!(req["params"][1]["sigVerify"].is_boolean());
                assert!(!journal.exists());
                *seen_wire.lock().unwrap() = Some(req["params"][0].as_str().unwrap().into());
                json!({"context":{"slot":9},"value":{"err":null,"logs":[],"unitsConsumed":100}})
            }
            "sendTransaction" => {
                let pending = PendingTransaction::load(&journal).unwrap();
                assert_eq!(pending.wire_base64, req["params"][0]);
                assert_eq!(
                    Some(pending.wire_base64.clone()),
                    *seen_wire.lock().unwrap()
                );
                json!(pending.signature)
            }
            "getSignatureStatuses" => {
                json!({"context":{"slot":10},"value":[{"slot":10,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]})
            }
            _ => panic!("unexpected {method}"),
        };
        rpc(req, value)
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let output = command
        .args(["--keypair", "key.json", "--execute", "user", "init"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(wire.lock().unwrap().as_ref().unwrap())
    );
    let value = result(output);
    let archive = value["journal"].as_str().unwrap();
    assert_eq!(value["mode"], "executed");
    assert!(!pending_path.exists());
    assert!(Path::new(archive).exists());
    assert_eq!(
        PendingTransaction::load(archive).unwrap().wire_base64,
        wire.lock().unwrap().as_ref().unwrap().to_owned()
    );
    assert!(!calls.lock().unwrap().iter().any(|s| s == "getBlockHeight"));
}
#[test]
fn resume_reconciles_before_send_preserves_uncertain_signature_and_never_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    let path = dir.path().join("pending.json");
    pending.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let expected = pending.wire_base64.clone();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        match method {
            "getSignatureStatuses" => rpc(req, json!({"context":{"slot":9},"value":[null]})),
            "getBlockHeight" => rpc(req, json!(99)),
            "sendTransaction" => {
                assert_eq!(req["params"][0], expected);
                json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":"unknown delivery"}})
            }
            _ => panic!("unexpected {method}"),
        }
    });
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            &mock.url,
            "transaction",
            "resume",
            "--file",
            "pending.json",
            "--execute",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(String::from_utf8_lossy(&output.stderr).contains(&pending.signature));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&pending.wire_base64));
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["getSignatureStatuses", "getBlockHeight", "sendTransaction"]
    );
}
#[test]
fn expired_resume_checks_status_twice_and_keeps_journal_without_broadcast() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    let path = dir.path().join("pending.json");
    pending.save(&path).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        rpc(
            req,
            match method {
                "getSignatureStatuses" => json!({"context":{"slot":9},"value":[null]}),
                "getBlockHeight" => json!(101),
                _ => panic!("unexpected {method}"),
            },
        )
    });
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            &mock.url,
            "transaction",
            "resume",
            "--file",
            "pending.json",
            "--execute",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(path.exists());
    assert_eq!(
        PendingTransaction::load(path).unwrap().signature,
        pending.signature
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "getSignatureStatuses",
            "getBlockHeight",
            "getSignatureStatuses"
        ]
    );
}
#[test]
fn confirmed_resume_archives_without_rebroadcast() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    pending.save(dir.path().join("pending.json")).unwrap();
    let mock = mock(|_, req| {
        assert_eq!(req["method"], "getSignatureStatuses");
        rpc(
            req,
            json!({"context":{"slot":10},"value":[{"slot":10,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]}),
        )
    });
    let value = result(
        cmd(dir.path())
            .args([
                "--rpc-url",
                &mock.url,
                "transaction",
                "resume",
                "--file",
                "pending.json",
                "--execute",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["signature"], pending.signature);
    assert!(!dir.path().join("pending.json").exists());
}

#[test]
fn help_never_displays_environment_values_and_parse_errors_name_safe_flags() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["provider", "update", "--help"],
        vec!["transaction", "resume", "--help"],
    ] {
        let output = cmd(dir.path())
            .env(
                "FOSSA_RPC_URL",
                "https://example.com/?api-key=rpc-help-secret",
            )
            .env("FOSSA_API_URL", "https://api-help-secret@example.com")
            .env("FOSSA_KEYPAIR", "/private/path-help-secret.json")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("help-secret"));
        assert!(text.contains("https://fossa.ag/docs/providers/cli.md"));
    }
    let output = cmd(dir.path())
        .args([
            "offer",
            "update",
            "--model",
            "m",
            "--input-price",
            "not-a-price",
        ])
        .output()
        .unwrap();
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        error["error"]["argument"]
            .as_str()
            .unwrap()
            .contains("--input-price")
    );
}
#[test]
fn offer_update_plans_keep_all_omitted_chain_terms_and_parse_real_rpc_strings() {
    let dir = tempfile::tempdir().unwrap();
    let wallet = key(5);
    let (model, bump) = fossa_sdk::address::model(
        &key(1),
        &key(2),
        &fossa_sdk::address::model_id("example/model"),
    );
    let terms_key = key(6);
    let capacity_key = key(7);
    let raw = |bytes: Vec<u8>| json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(bytes),"base64"]});
    let mut m = vec![0; 176];
    m[..8].copy_from_slice(<fossa_sdk::accounts::Model as fossa_sdk::accounts::Decode>::MAGIC);
    m[8..40].copy_from_slice(key(2).as_ref());
    m[40..72].copy_from_slice(&fossa_sdk::address::model_id("example/model"));
    m[72..104].copy_from_slice(terms_key.as_ref());
    m[104..136].copy_from_slice(capacity_key.as_ref());
    m[136..140].copy_from_slice(&1u32.to_le_bytes());
    m[140] = bump;
    let mut t = vec![0; 168];
    t[..8].copy_from_slice(<fossa_sdk::accounts::TermsBook as fossa_sdk::accounts::Decode>::MAGIC);
    t[8..40].copy_from_slice(model.as_ref());
    t[40..44].copy_from_slice(&1u32.to_le_bytes());
    t[44..48].copy_from_slice(&1u32.to_le_bytes());
    t[48..80].copy_from_slice(wallet.as_ref());
    t[80..112].copy_from_slice(key(8).as_ref());
    t[112..144].copy_from_slice(key(9).as_ref());
    t[144..148].copy_from_slice(&1_000_000u32.to_le_bytes());
    t[148..152].copy_from_slice(&500_000u32.to_le_bytes());
    t[152..156].copy_from_slice(&2_000_000u32.to_le_bytes());
    t[156..158].copy_from_slice(&300u16.to_le_bytes());
    t[158] = 1;
    t[159] = 0;
    t[160] = 1;
    t[164..168].copy_from_slice(&7u32.to_le_bytes());
    let mut c = vec![0; 80];
    c[..8]
        .copy_from_slice(<fossa_sdk::accounts::CapacityBook as fossa_sdk::accounts::Decode>::MAGIC);
    c[8..40].copy_from_slice(model.as_ref());
    c[40..44].copy_from_slice(&1u32.to_le_bytes());
    c[44..48].copy_from_slice(&1u32.to_le_bytes());
    let (m, t, c) = (raw(m), raw(t), raw(c));
    let mock = mock(move |_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getAccountInfo" => {
                    json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(4))}else{m.clone()}})
                }
                "getMultipleAccounts" => json!({"context":{"slot":9},"value":[t,c]}),
                method => panic!("unexpected {method}"),
            },
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let value = result(
        command
            .args([
                "offer",
                "update",
                "--model",
                "example/model",
                "--output-price",
                "3.123456",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        value["details"]["next"],
        json!({"rates":{"input":1_000_000,"cached":500_000,"output":3_123_456},"speed_tps":300,"private":true,"active":false,"capacity":7})
    );
    assert_eq!(
        value["instructions"][0]["discriminator"],
        instruction_discriminator("update_offer")
    );
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn booking_budget_counts_every_permanent_slot_and_preserves_packet_limit() {
    for count in [1u32, 16, 500, 2760, 2761, 4096] {
        let dir = tempfile::tempdir().unwrap();
        let wallet = key(5);
        let user = key(10);
        let id = fossa_sdk::address::model_id("example/model");
        let (model, bump) = fossa_sdk::address::model(&key(1), &key(2), &id);
        let raw = |bytes: Vec<u8>| json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(bytes),"base64"]});
        let mut m = vec![0; 176];
        m[..8].copy_from_slice(<fossa_sdk::accounts::Model as fossa_sdk::accounts::Decode>::MAGIC);
        m[8..40].copy_from_slice(key(2).as_ref());
        m[40..72].copy_from_slice(&id);
        m[72..104].copy_from_slice(key(6).as_ref());
        m[104..136].copy_from_slice(key(7).as_ref());
        m[136..140].copy_from_slice(&count.to_le_bytes());
        m[140] = bump;
        let mut u = vec![0; 128];
        u[..8].copy_from_slice(<fossa_sdk::accounts::User as fossa_sdk::accounts::Decode>::MAGIC);
        u[8..40].copy_from_slice(wallet.as_ref());
        u[40..72].copy_from_slice(key(2).as_ref());
        u[72..80].copy_from_slice(&1_000_000u64.to_le_bytes());
        u[80..112].copy_from_slice(key(11).as_ref());
        u[120] = fossa_sdk::address::vault(&key(1), &user).1;
        let mut t = vec![0; 48 + count as usize * 120];
        let mut c = vec![0; 48 + count as usize * 32];
        for (book, magic) in [
            (
                &mut t,
                <fossa_sdk::accounts::TermsBook as fossa_sdk::accounts::Decode>::MAGIC,
            ),
            (
                &mut c,
                <fossa_sdk::accounts::CapacityBook as fossa_sdk::accounts::Decode>::MAGIC,
            ),
        ] {
            book[..8].copy_from_slice(magic);
            book[8..40].copy_from_slice(model.as_ref());
            book[40..44].copy_from_slice(&count.to_le_bytes());
            book[44..48].copy_from_slice(&count.to_le_bytes());
        }
        for slot in 0..count as usize {
            let term = &mut t[48 + slot * 120..48 + (slot + 1) * 120];
            term[..32].copy_from_slice(key(12).as_ref());
            term[32..64].copy_from_slice(key(13).as_ref());
            term[64..96].copy_from_slice(key(14).as_ref());
            for offset in [96, 100, 104] {
                term[offset..offset + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
            }
            term[108..110].copy_from_slice(&1u16.to_le_bytes());
            term[111] = u8::from(slot + 1 == count as usize);
            term[112] = 1;
            term[116..120].copy_from_slice(&1u32.to_le_bytes());
            c[48 + slot * 32..56 + slot * 32].copy_from_slice(&1_000_000u64.to_le_bytes());
        }
        let mut list = vec![0; fossa_sdk::instruction::list_size(1).unwrap()];
        list[..8].copy_from_slice(
            <fossa_sdk::accounts::ProviderList as fossa_sdk::accounts::Decode>::MAGIC,
        );
        list[8..40].copy_from_slice(key(2).as_ref());
        list[40..72].copy_from_slice(wallet.as_ref());
        list[72..76].copy_from_slice(&1u32.to_le_bytes());
        list[76..78].copy_from_slice(&100u16.to_le_bytes());
        list[78..80].copy_from_slice(&10_000u16.to_le_bytes());
        list[84..88].copy_from_slice(&1u32.to_le_bytes());
        list[88..120].copy_from_slice(key(12).as_ref());
        let list = raw(list);
        let (m, u, t, c) = (raw(m), raw(u), raw(t), raw(c));
        let mock = mock(move |_, req| {
            rpc(
                req,
                match req["method"].as_str().unwrap() {
                    "getAccountInfo" => {
                        json!({"context":{"slot":9},"value":if req["params"][0] == key(2).to_string() { market_account(key(4)) } else if req["params"][0] == user.to_string() {u.clone()} else if req["params"][0] == key(30).to_string() {list.clone()} else {m.clone()}})
                    }
                    "getMultipleAccounts" => json!({"context":{"slot":9},"value":[t,c]}),
                    "getMinimumBalanceForRentExemption" => json!(1000),
                    method => panic!("unexpected {method}"),
                },
            )
        });
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        let plan = result(
            command
                .args([
                    "booking",
                    "create",
                    "--user",
                    &user.to_string(),
                    "--model",
                    "example/model",
                    "--list",
                    &key(30).to_string(),
                    "--max-input",
                    "1",
                    "--max-output",
                    "1",
                    "--max-spend",
                    "1",
                    "--input-price-cap",
                    "1",
                    "--cached-price-cap",
                    "1",
                    "--output-price-cap",
                    "1",
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(plan["details"]["candidate_slot"], count - 1);
        let mut bytes = vec![2];
        bytes.extend((20_000 + 500 * count).min(1_400_000).to_le_bytes());
        assert_eq!(
            plan["instructions"][0]["data_hex"],
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let instructions: Vec<_> = plan["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|ix| {
                let hex = ix["data_hex"].as_str().unwrap();
                fossa_sdk::Instruction {
                    program_id: ix["program"].as_str().unwrap().parse().unwrap(),
                    data: (0..hex.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                        .collect(),
                    accounts: ix["accounts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| fossa_sdk::AccountMeta {
                            pubkey: a["address"].as_str().unwrap().parse().unwrap(),
                            is_signer: a["signer"].as_bool().unwrap(),
                            is_writable: a["writable"].as_bool().unwrap(),
                        })
                        .collect(),
                }
            })
            .collect();
        PreparedTransaction::new(&instructions, wallet, Hash::default(), 100, 0).unwrap();
        assert!(!dir.path().join("transactions").exists());
    }
}

#[test]
fn explicit_rpc_choice_overrides_other_transport_environment() {
    let dir = tempfile::tempdir().unwrap();
    let value = result(
        cmd(dir.path())
            .env("FOSSA_RPC_URL", "https://rpc-secret.example")
            .args(["--hosted-rpc", "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["rpc"], "hosted");
    let value = result(
        cmd(dir.path())
            .env("FOSSA_HOSTED_RPC", "true")
            .args(["--rpc-url", "https://own.example", "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["rpc"], "custom (URL redacted)");
}
#[test]
fn invalid_api_origin_never_leaks_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let output = cmd(dir.path())
        .args([
            "--api-url",
            "https://origin-secret@example.com/?token=origin-secret",
            "config",
            "show",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("origin-secret"));
    assert!(output.stdout.is_empty());
}

#[test]
fn simulation_failure_never_creates_journal_or_broadcasts() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    let wallet = signer.pubkey();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let mock = mock(|_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getAccountInfo" => json!({"context":{"slot":9},"value":market_account(key(4))}),
                "getMinimumBalanceForRentExemption" => json!(1000),
                "getLatestBlockhash" => {
                    json!({"context":{"slot":9},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}})
                }
                "simulateTransaction" => {
                    json!({"context":{"slot":9},"value":{"err":{"InstructionError":[0,"Custom"]},"logs":[],"unitsConsumed":100}})
                }
                method => panic!("unexpected {method}"),
            },
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let output = command
        .args(["--keypair", "key.json", "--execute", "user", "init"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.path().join("transactions/pending.json").exists());
}
#[test]
fn onboarding_plans_permissionless_registration_without_uploading() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("provider.json"),
        r#"{"name":"Example","endpoint":"https://example.com/infer"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[provider]\nmetadata_file='provider.json'\n",
    )
    .unwrap();
    let mock = mock(|_, req| {
        assert_eq!(req["method"], "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(5))}else{Value::Null}}),
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let value = result(
        command
            .args(["--config", "config.toml", "onboard"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["blockers"], json!([]));
    assert_eq!(value["steps"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["steps"][0]["instructions"][0]["discriminator"],
        instruction_discriminator("initialize_provider")
    );
    assert!(!dir.path().join("transactions/pending.json").exists());
}
#[test]
fn metadata_upload_is_exact_immutable_and_independent_of_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let bytes = b"{ \"name\":\"Example\",\"endpoint\":\"https://example.com/infer\" }\n";
    std::fs::write(dir.path().join("provider.json"), bytes).unwrap();
    let sha = fossa_sdk::metadata::sha256_hex(bytes);
    let base = Arc::new(Mutex::new(String::new()));
    let url = base.clone();
    let hash = sha.clone();
    let mock = mock(move |path, req| {
        assert_eq!(path, "/v1/providers/metadata/provider");
        assert_eq!(req["name"], "Example");
        json!({"sha256":hash,"uri":format!("{}/v1/market/metadata/{hash}",url.lock().unwrap()),"bytes":bytes.len()})
    });
    *base.lock().unwrap() = mock.url.clone();
    let value = result(
        cmd(dir.path())
            .args([
                "--api-url",
                &mock.url,
                "--keypair",
                "key.json",
                "metadata",
                "upload",
                "provider",
                "provider.json",
                "--execute",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["upload"]["sha256"], sha);
    assert_eq!(value["upload"]["bytes"], bytes.len());
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn no_arguments_show_help_successfully_and_json_flag_keeps_pipes_compact() {
    let dir = tempfile::tempdir().unwrap();
    let help = cmd(dir.path())
        .env("FOSSA_CONFIG", "missing-help-secret.toml")
        .env("FOSSA_KEYPAIR", "missing-help-secret.json")
        .env("FOSSA_WALLET", "invalid-help-secret")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Usage: fossa"));
    assert!(help.contains("Fossa accounts, providers and bookings"));
    assert!(help.contains("--json"));
    assert!(!help.contains("fossa-provider"));
    assert!(!help.contains("help-secret"));
    assert!(!help.contains('\u{1b}'));
    for flags in [vec!["config", "show"], vec!["--json", "config", "show"]] {
        let output = cmd(dir.path()).args(flags).output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains('\u{1b}'));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["result"]["rpc"],
            "hosted"
        );
    }
    let error = cmd(dir.path())
        .args(["--json", "provider", "register"])
        .output()
        .unwrap();
    assert!(!error.status.success());
    assert!(error.stdout.is_empty());
    let text = String::from_utf8(error.stderr).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("--metadata-file"));
    assert!(!text.contains('\u{1b}'));
    assert!(serde_json::from_str::<Value>(&text).unwrap()["error"]["hint"].is_string());
}
#[test]
fn no_color_overrides_forced_help_colors_and_never_alters_json_data() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["--help"], vec!["provider", "update", "--help"]] {
        let output = cmd(dir.path())
            .env("NO_COLOR", "")
            .env("CLICOLOR_FORCE", "1")
            .env("TERM", "xterm-256color")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8(output.stdout).unwrap().contains('\u{1b}'));
    }
    let value = result(
        cmd(dir.path())
            .env("NO_COLOR", "1")
            .args(["--json", "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["api_url"], "https://api.fossa.ag");
    assert_eq!(
        value["program_id"],
        "F8q85nw6UScZCBKBJqVAu7ZdYhmT9XRsm6TqZzJqXSNT"
    );
    assert_eq!(
        value["market"],
        "8wceBTFDAiC9WiT3xZGRf9Nf4xrz8eBCxExBtDWz1Som"
    );
    assert_eq!(
        value["usdc_mint"],
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    );
}
#[test]
fn default_transport_respects_file_env_flags_and_explicit_disable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[solana]\nrpc_url='https://own.example/?token=secret'\n",
    )
    .unwrap();
    assert_eq!(
        result(
            cmd(dir.path())
                .args(["--config", "config.toml", "config", "show"])
                .output()
                .unwrap()
        )["rpc"],
        "custom (URL redacted)"
    );
    assert_eq!(
        result(
            cmd(dir.path())
                .args(["--config", "config.toml", "--hosted-rpc", "config", "show"])
                .output()
                .unwrap()
        )["rpc"],
        "hosted"
    );
    assert_eq!(
        result(
            cmd(dir.path())
                .args(["--hosted-rpc=false", "config", "show"])
                .output()
                .unwrap()
        )["rpc"],
        "disabled"
    );
    std::fs::write(dir.path().join("config.toml"), "[hosted]\nrpc=true\n").unwrap();
    assert_eq!(
        result(
            cmd(dir.path())
                .env("FOSSA_HOSTED_RPC", "false")
                .args(["--config", "config.toml", "config", "show"])
                .output()
                .unwrap()
        )["rpc"],
        "disabled"
    );
    std::fs::write(dir.path().join("config.toml"), "[hosted]\nrpc=false\n").unwrap();
    assert_eq!(
        result(
            cmd(dir.path())
                .args(["--config", "config.toml", "config", "show"])
                .output()
                .unwrap()
        )["rpc"],
        "disabled"
    );
    let changed = result(
        cmd(dir.path())
            .args(["--market", &key(9).to_string(), "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(changed["market"], key(9).to_string());
    assert_eq!(
        changed["program_id"],
        "F8q85nw6UScZCBKBJqVAu7ZdYhmT9XRsm6TqZzJqXSNT"
    );
}
#[test]
fn pinned_protocol_needs_no_public_config_lookup_and_hosted_rpc_is_default() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    let wallet = signer.pubkey();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let market = "8wceBTFDAiC9WiT3xZGRf9Nf4xrz8eBCxExBtDWz1Som";
    let mut data = <fossa_sdk::accounts::Market as fossa_sdk::accounts::Decode>::MAGIC.to_vec();
    data.extend(key(4).as_ref());
    data.extend(
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            .parse::<Pubkey>()
            .unwrap()
            .as_ref(),
    );
    data.resize(fossa_sdk::accounts::MARKET_LEN, 0);
    let account = json!({"owner":"F8q85nw6UScZCBKBJqVAu7ZdYhmT9XRsm6TqZzJqXSNT","lamports":1,"executable":false,"data":[STANDARD.encode(data),"base64"]});
    let paths = Arc::new(Mutex::new(Vec::new()));
    let seen = paths.clone();
    let mock = mock(move |path, req| {
        seen.lock().unwrap().push(path.to_owned());
        assert_eq!(req["method"], "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==market {account.clone()}else{Value::Null}}),
        )
    });
    let value = result(
        cmd(dir.path())
            .args([
                "--api-url",
                &mock.url,
                "--keypair",
                "key.json",
                "provider",
                "show",
            ])
            .output()
            .unwrap(),
    );
    assert!(value["provider"].is_null());
    assert_eq!(
        *paths.lock().unwrap(),
        vec!["/v1/providers/rpc", "/v1/providers/rpc"]
    );
    paths.lock().unwrap().clear();
    let value = result(
        cmd(dir.path())
            .args([
                "--api-url",
                "http://127.0.0.1:1",
                "--rpc-url",
                &mock.url,
                "--wallet",
                &wallet.to_string(),
                "provider",
                "show",
            ])
            .output()
            .unwrap(),
    );
    assert!(value["provider"].is_null());
    assert_eq!(*paths.lock().unwrap(), vec!["/", "/"]);
}

#[test]
fn argument_diagnostics_identify_missing_values_and_conflicts_without_echoing_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let output = cmd(dir.path()).args(["--rpc-url"]).output().unwrap();
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["error"]["message"], "--rpc-url requires a value");
    let output = cmd(dir.path())
        .args([
            "--hosted-rpc",
            "--rpc-url",
            "https://rpc.example/?credential=secret-value",
            "config",
            "show",
        ])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        value["error"]["message"],
        "--rpc-url and --hosted-rpc cannot be used together"
    );
    assert!(!value.to_string().contains("secret-value"));
}

#[test]
fn provider_list_commands_plan_exact_membership_and_enforce_owner_capacity_and_empty_close() {
    let dir = tempfile::tempdir().unwrap();
    let list = key(30);
    let owner = key(5);
    let data = Arc::new(Mutex::new(vec![
        0;
        fossa_sdk::instruction::list_size(2)
            .unwrap()
    ]));
    {
        let mut d = data.lock().unwrap();
        d[..8].copy_from_slice(
            <fossa_sdk::accounts::ProviderList as fossa_sdk::accounts::Decode>::MAGIC,
        );
        d[8..40].copy_from_slice(key(2).as_ref());
        d[40..72].copy_from_slice(owner.as_ref());
        d[72..76].copy_from_slice(&2u32.to_le_bytes());
        d[76..78].copy_from_slice(&100u16.to_le_bytes());
        d[78..80].copy_from_slice(&10_000u16.to_le_bytes());
        d[84..88].copy_from_slice(&1u32.to_le_bytes());
        d[88..120].copy_from_slice(key(6).as_ref());
    }
    let state = data.clone();
    let mock = mock(move |_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getMinimumBalanceForRentExemption" => json!(1000),
                "getAccountInfo" => {
                    json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string() {market_account(key(99))} else if req["params"][0]==list.to_string() {json!({"owner":key(1).to_string(),"lamports":1000,"executable":false,"data":[STANDARD.encode(&*state.lock().unwrap()),"base64"]})} else {Value::Null}})
                }
                method => panic!("unexpected mutation or signing request {method}"),
            },
        )
    });
    let invoke = |wallet, args: &[&str]| {
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        command.args(args).output().unwrap()
    };
    let address = list.to_string();
    let create = result(invoke(
        owner,
        &[
            "list",
            "create",
            "--fee-bps",
            "100",
            "--protocol-share-bps",
            "10000",
            "--maximum-payment",
            "1",
            "--capacity",
            "2",
        ],
    ));
    assert_eq!(
        create["instructions"][1]["discriminator"],
        instruction_discriminator("initialize_list")
    );
    assert!(
        create["details"]["list"]
            .as_str()
            .unwrap()
            .parse::<Pubkey>()
            .is_ok()
    );
    let show = result(invoke(key(9), &["list", "show", "--list", &address]));
    assert_eq!(show["state"]["providers"], json!([key(6).to_string()]));
    let add = result(invoke(
        owner,
        &[
            "list",
            "add",
            "--list",
            &address,
            "--provider",
            &key(7).to_string(),
        ],
    ));
    assert_eq!(
        add["instructions"][1]["data_hex"],
        format!(
            "{}{}01",
            instruction_discriminator("update_list"),
            "07".repeat(32)
        )
    );
    let remove = result(invoke(
        owner,
        &[
            "list",
            "remove",
            "--list",
            &address,
            "--provider",
            &key(6).to_string(),
        ],
    ));
    assert_eq!(
        remove["instructions"][1]["data_hex"],
        format!(
            "{}{}00",
            instruction_discriminator("update_list"),
            "06".repeat(32)
        )
    );
    for plan in [&create, &add, &remove] {
        assert_eq!(plan["instructions"][0]["data_hex"], "0290d00300");
    }
    for (wallet, verb, provider) in [
        (key(9), "add", key(7)),
        (owner, "add", key(6)),
        (owner, "remove", key(7)),
    ] {
        assert!(
            !invoke(
                wallet,
                &[
                    "list",
                    verb,
                    "--list",
                    &address,
                    "--provider",
                    &provider.to_string()
                ]
            )
            .status
            .success()
        );
    }
    assert!(
        !invoke(owner, &["list", "close", "--list", &address])
            .status
            .success()
    );
    {
        let mut d = data.lock().unwrap();
        d[84..88].copy_from_slice(&2u32.to_le_bytes());
        d[120..152].copy_from_slice(key(7).as_ref());
    }
    assert!(
        !invoke(
            owner,
            &[
                "list",
                "add",
                "--list",
                &address,
                "--provider",
                &key(8).to_string()
            ]
        )
        .status
        .success()
    );
    {
        let mut d = data.lock().unwrap();
        d[84..88].copy_from_slice(&0u32.to_le_bytes());
        d[88..].fill(0);
    }
    let close = result(invoke(owner, &["list", "close", "--list", &address]));
    assert_eq!(
        close["instructions"][0]["discriminator"],
        instruction_discriminator("close_list")
    );
    assert_eq!(
        close["instructions"][0]["accounts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn group_profile_commands_create_inspect_update_and_bind_authority_without_signing() {
    use fossa_sdk::{accounts::LIST_METADATA_LEN, address};
    let dir = tempfile::tempdir().unwrap();
    let list = key(30);
    let owner = key(5);
    let (metadata_address, bump) = address::list_metadata(&key(1), &list);
    let mut data = vec![0; fossa_sdk::instruction::list_size(2).unwrap()];
    data[..8]
        .copy_from_slice(<fossa_sdk::accounts::ProviderList as fossa_sdk::accounts::Decode>::MAGIC);
    data[8..40].copy_from_slice(key(2).as_ref());
    data[40..72].copy_from_slice(owner.as_ref());
    data[72..76].copy_from_slice(&2u32.to_le_bytes());
    let raw = |owner: Pubkey, data: &[u8]| json!({"owner":owner.to_string(),"lamports":1000,"executable":false,"data":[STANDARD.encode(data),"base64"]});
    let list_account = raw(key(1), &data);
    let metadata = Arc::new(Mutex::new(Value::Null));
    let state = metadata.clone();
    let mock = mock(move |_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getMinimumBalanceForRentExemption" => json!(1000),
                "getAccountInfo" => {
                    json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(99))}else if req["params"][0]==list.to_string(){list_account.clone()}else if req["params"][0]==metadata_address.to_string(){state.lock().unwrap().clone()}else{panic!("unexpected account")}})
                }
                method => panic!("unexpected signing or mutation {method}"),
            },
        )
    });
    let invoke = |wallet, args: &[&str]| {
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        command.args(args).output().unwrap()
    };
    let file = dir.path().join("group.json");
    let bytes = br##"{ "name":"Example Group", "accentColor":"#8B7ACF", "icon":"https://example.com/icon.svg" }"##;
    fs::write(&file, bytes).unwrap();
    let file = file.to_str().unwrap();
    let address = list.to_string();
    let create = result(invoke(
        owner,
        &[
            "list",
            "create",
            "--fee-bps",
            "100",
            "--protocol-share-bps",
            "10000",
            "--maximum-payment",
            "1",
            "--capacity",
            "2",
            "--metadata-file",
            file,
        ],
    ));
    assert_eq!(
        create["instructions"][2]["discriminator"],
        instruction_discriminator("initialize_list_metadata")
    );
    assert_eq!(
        create["details"]["metadata"]["document"]["name"],
        "Example Group"
    );
    let update = [
        "list",
        "update",
        "--list",
        &address,
        "--metadata-file",
        file,
    ];
    let first = result(invoke(owner, &update));
    assert_eq!(
        first["instructions"][0]["discriminator"],
        instruction_discriminator("initialize_list_metadata")
    );
    assert_eq!(
        first["details"]["metadata"]["sha256"],
        fossa_sdk::metadata::sha256_hex(bytes)
    );
    *metadata.lock().unwrap() = raw(address::SYSTEM_PROGRAM, &[]);
    assert_eq!(
        result(invoke(owner, &update))["instructions"][0]["discriminator"],
        instruction_discriminator("initialize_list_metadata")
    );
    let mut account = vec![0; LIST_METADATA_LEN];
    account[..8]
        .copy_from_slice(<fossa_sdk::accounts::ListMetadata as fossa_sdk::accounts::Decode>::MAGIC);
    account[8..40].copy_from_slice(key(2).as_ref());
    account[40..72].copy_from_slice(list.as_ref());
    account[72..104].copy_from_slice(owner.as_ref());
    account[104..136].fill(42);
    let uri = b"https://example.com/group.json";
    account[136..138].copy_from_slice(&(uri.len() as u16).to_le_bytes());
    account[138] = bump;
    account[140..140 + uri.len()].copy_from_slice(uri);
    *metadata.lock().unwrap() = raw(key(1), &account);
    let show = result(invoke(key(9), &["list", "show", "--list", &address]));
    assert_eq!(show["metadata"]["list"], address);
    assert_eq!(
        show["metadata"]["document"]["uri"],
        std::str::from_utf8(uri).unwrap()
    );
    assert_eq!(
        result(invoke(owner, &update))["instructions"][0]["discriminator"],
        instruction_discriminator("update_list_metadata")
    );
    assert!(!invoke(key(9), &update).status.success());
    account[72..104].copy_from_slice(key(9).as_ref());
    *metadata.lock().unwrap() = raw(key(1), &account);
    assert!(!invoke(owner, &update).status.success());
    *metadata.lock().unwrap() = Value::Null;
    let icon = dir.path().join("icon.svg");
    fs::write(
        &icon,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><path fill="white" d="M0 0h24v24z"/></svg>"#,
    )
    .unwrap();
    let patch = result(invoke(
        owner,
        &[
            "list",
            "update",
            "--list",
            &address,
            "--name",
            "Group",
            "--accent-color",
            "#abcdef",
            "--icon-file",
            icon.to_str().unwrap(),
        ],
    ));
    assert_eq!(patch["details"]["icon_upload"], true);
    assert!(
        patch["details"]["document"]["icon"]
            .as_str()
            .unwrap()
            .starts_with("https://api.fossa.ag/v1/providers/images/")
    );
    fs::write(&icon, b"<svg><script/></svg>").unwrap();
    assert!(
        !invoke(
            owner,
            &[
                "list",
                "update",
                "--list",
                &address,
                "--icon-file",
                icon.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn reusable_claim_publishes_bound_receipt_without_auxiliary_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let provider = key(5);
    let user = key(10);
    let owner = key(11);
    let booking = key(12);
    let id = fossa_sdk::address::model_id("example/model");
    let (model, bump) = fossa_sdk::address::model(&key(1), &key(2), &id);
    let raw = |data: Vec<u8>| json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(data),"base64"]});
    let mut b = vec![0; 472];
    b[..8].copy_from_slice(<fossa_sdk::accounts::Booking as fossa_sdk::accounts::Decode>::MAGIC);
    b[8..40].copy_from_slice(owner.as_ref());
    b[40..72].copy_from_slice(provider.as_ref());
    b[72..104].copy_from_slice(key(2).as_ref());
    b[104..112].copy_from_slice(&100u64.to_le_bytes());
    for offset in [112, 116, 120, 124, 128] {
        b[offset..offset + 4].copy_from_slice(&100u32.to_le_bytes());
    }
    b[132..140].copy_from_slice(&123456789i64.to_le_bytes());
    b[152..160].copy_from_slice(&7u64.to_le_bytes());
    b[192..224].copy_from_slice(user.as_ref());
    b[224..256].copy_from_slice(model.as_ref());
    b[288..320].copy_from_slice(key(13).as_ref());
    b[358] = 1;
    let mut u = vec![0; 128];
    u[..8].copy_from_slice(<fossa_sdk::accounts::User as fossa_sdk::accounts::Decode>::MAGIC);
    u[8..40].copy_from_slice(owner.as_ref());
    u[40..72].copy_from_slice(key(2).as_ref());
    u[80..112].copy_from_slice(key(14).as_ref());
    u[120] = fossa_sdk::address::vault(&key(1), &user).1;
    let mut m = vec![0; 176];
    m[..8].copy_from_slice(<fossa_sdk::accounts::Model as fossa_sdk::accounts::Decode>::MAGIC);
    m[8..40].copy_from_slice(key(2).as_ref());
    m[40..72].copy_from_slice(&id);
    m[72..104].copy_from_slice(key(15).as_ref());
    m[104..136].copy_from_slice(key(16).as_ref());
    m[136..140].copy_from_slice(&16u32.to_le_bytes());
    m[140] = bump;
    let (b, u, m) = (raw(b), raw(u), raw(m));
    let mock = mock(move |_, req| {
        assert_eq!(req["method"], "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string() {market_account(key(99))} else if req["params"][0]==booking.to_string(){b.clone()} else if req["params"][0]==user.to_string(){u.clone()} else {m.clone()}}),
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, provider);
    let plan = result(
        command
            .args([
                "booking",
                "claim",
                "--booking",
                &booking.to_string(),
                "--uncached",
                "3",
                "--cached",
                "2",
                "--output",
                "1",
            ])
            .output()
            .unwrap(),
    );
    let ix = &plan["instructions"][0];
    assert_eq!(ix["accounts"].as_array().unwrap().len(), 11);
    let expected = [
        fossa_sdk::instruction::discriminator("claim_booking").to_vec(),
        3u32.to_le_bytes().to_vec(),
        2u32.to_le_bytes().to_vec(),
        1u32.to_le_bytes().to_vec(),
        vec![1],
        fossa_sdk::receipt::Receipt {
            owner,
            user,
            booking,
            generation: 7,
            charge: 3,
            claim_end: 123456789,
        }
        .encode()
        .to_vec(),
    ]
    .concat();
    assert_eq!(
        ix["data_hex"],
        expected
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn model_growth_plans_permissionless_rent_and_rejects_invalid_increments() {
    for (limit, donated, succeeds) in [
        (5u32, false, true),
        (89, true, true),
        (4, false, false),
        (3, false, false),
        (90, false, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let wallet = key(5);
        let id = fossa_sdk::address::model_id("example/model");
        let (model, bump) = fossa_sdk::address::model(&key(1), &key(2), &id);
        let mut m = vec![0; 176];
        m[..8].copy_from_slice(<fossa_sdk::accounts::Model as fossa_sdk::accounts::Decode>::MAGIC);
        for (at, value) in [(8, key(2)), (72, key(6)), (104, key(7))] {
            m[at..at + 32].copy_from_slice(value.as_ref());
        }
        m[40..72].copy_from_slice(&id);
        m[136..140].copy_from_slice(&4u32.to_le_bytes());
        m[140] = bump;
        let mut t = vec![0; 48 + 4 * 120];
        let mut c = vec![0; 48 + 4 * 32];
        for (bytes, magic) in [
            (
                &mut t,
                <fossa_sdk::accounts::TermsBook as fossa_sdk::accounts::Decode>::MAGIC,
            ),
            (
                &mut c,
                <fossa_sdk::accounts::CapacityBook as fossa_sdk::accounts::Decode>::MAGIC,
            ),
        ] {
            bytes[..8].copy_from_slice(magic);
            bytes[8..40].copy_from_slice(model.as_ref());
            bytes[40..44].copy_from_slice(&4u32.to_le_bytes());
        }
        let raw = |bytes: Vec<u8>, lamports: u64| json!({"owner":key(1).to_string(),"lamports":lamports,"executable":false,"data":[STANDARD.encode(bytes),"base64"]});
        let (m, t, c) = (
            raw(m, 1),
            raw(t, if donated { 200_000 } else { 5280 }),
            raw(c, 1760),
        );
        let mock = mock(move |_, q| {
            rpc(
                q,
                match q["method"].as_str().unwrap() {
                    "getAccountInfo" => {
                        json!({"context":{"slot":9},"value":if q["params"][0]==key(2).to_string(){market_account(key(4))}else{m.clone()}})
                    }
                    "getMultipleAccounts" => json!({"context":{"slot":9},"value":[t,c]}),
                    "getMinimumBalanceForRentExemption" => {
                        json!(q["params"][0].as_u64().unwrap() * 10)
                    }
                    other => panic!("unexpected {other}"),
                },
            )
        });
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        let output = command
            .args([
                "model",
                "grow",
                "--model",
                "example/model",
                "--limit",
                &limit.to_string(),
            ])
            .output()
            .unwrap();
        if !succeeds {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("1 to 85"));
            continue;
        }
        let plan = result(output);
        assert_eq!(plan["details"]["previous_limit"], 4);
        assert_eq!(plan["details"]["limit"], limit);
        let rent = if donated {
            (limit - 4) * 32 * 10
        } else {
            (limit - 4) * 152 * 10
        };
        assert_eq!(
            plan["details"]["additional_rent_lamports"],
            rent.to_string()
        );
        let instructions = plan["instructions"].as_array().unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(
            instructions[0]["discriminator"],
            instruction_discriminator("grow_books")
        );
        let accounts = instructions[0]["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 5);
        assert_eq!(accounts[4]["address"], wallet.to_string());
        assert_eq!(accounts[4]["signer"], true);
        assert!(!dir.path().join("transactions").exists());
    }
}

fn instruction_discriminator(name: &str) -> String {
    fossa_sdk::instruction::discriminator(name)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn council_policy_patch_preserves_gate_and_rotation_plans_both_signers() {
    let dir = tempfile::tempdir().unwrap();
    let council = key(5);
    let mock = mock(move |_, req| {
        assert_eq!(req["method"], "getAccountInfo");
        assert_eq!(req["params"][0], key(2).to_string());
        let mut state = market_account(council);
        let mut bytes = STANDARD.decode(state["data"][0].as_str().unwrap()).unwrap();
        bytes[176] = 0;
        state["data"][0] = STANDARD.encode(bytes).into();
        rpc(req, json!({"context":{"slot":9},"value":state}))
    });
    let invoke = |wallet, args: &[&str]| {
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        command.args(args).output().unwrap()
    };
    let bank = key(6).to_string();
    let plan = result(invoke(
        council,
        &["council", "configure-fees", "--bank", &bank],
    ));
    assert_eq!(plan["details"]["next"]["creation_enabled"], false);
    assert_eq!(plan["details"]["next"]["creation_net_price"], 1);
    assert_eq!(plan["details"]["next"]["creation_mint"], key(4).to_string());
    assert_eq!(
        plan["instructions"][0]["accounts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        !invoke(key(9), &["council", "configure-fees", "--bank", &bank])
            .status
            .success()
    );
    let next = key(7).to_string();
    let rotate = result(invoke(council, &["council", "rotate", "--next", &next]));
    let accounts = rotate["instructions"][0]["accounts"].as_array().unwrap();
    assert_eq!(accounts[1]["address"], council.to_string());
    assert_eq!(accounts[2]["address"], next);
    assert_eq!(accounts[1]["signer"], true);
    assert_eq!(accounts[2]["signer"], true);
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn list_fee_planning_enforces_authority_and_permitted_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let authority = key(5);
    let list = key(30);
    let mut bytes = vec![0; fossa_sdk::instruction::list_size(1).unwrap()];
    bytes[..8]
        .copy_from_slice(<fossa_sdk::accounts::ProviderList as fossa_sdk::accounts::Decode>::MAGIC);
    bytes[8..40].copy_from_slice(key(2).as_ref());
    bytes[40..72].copy_from_slice(authority.as_ref());
    bytes[72..76].copy_from_slice(&1u32.to_le_bytes());
    bytes[76..78].copy_from_slice(&100u16.to_le_bytes());
    bytes[78..80].copy_from_slice(&10_000u16.to_le_bytes());
    let mock = mock(move |_, req| {
        assert_eq!(req["method"], "getAccountInfo");
        let state = if req["params"][0] == key(2).to_string() {
            market_account(key(99))
        } else {
            assert_eq!(req["params"][0], list.to_string());
            json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(&bytes),"base64"]})
        };
        rpc(req, json!({"context":{"slot":9},"value":state}))
    });
    let invoke = |wallet, fee: &str| {
        let mut command = cmd(dir.path());
        protocol_args(&mut command, &mock.url, wallet);
        command
            .args([
                "list",
                "configure-fees",
                "--list",
                &list.to_string(),
                "--fee-bps",
                fee,
                "--protocol-share-bps",
                "5000",
            ])
            .output()
            .unwrap()
    };
    let plan = result(invoke(authority, "10"));
    assert_eq!(plan["details"]["previous"]["fee_bps"], 100);
    assert_eq!(plan["details"]["next"]["fee_bps"], 10);
    assert_eq!(plan["details"]["next"]["protocol_share"], 5000);
    assert_eq!(
        plan["instructions"][0]["discriminator"],
        instruction_discriminator("configure_list_fees")
    );
    for (wallet, fee) in [(authority, "9"), (authority, "301"), (key(9), "10")] {
        assert!(!invoke(wallet, fee).status.success());
    }
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn model_assignment_validates_group_market_and_clear_uses_anchor_none() {
    use fossa_sdk::{
        accounts::{Decode, Model, ModelGroup},
        address,
    };
    for foreign in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let council = key(5);
        let id = address::model_id("example/model");
        let (model, bump) = address::model(&key(1), &key(2), &id);
        let group_market = if foreign { key(9) } else { key(2) };
        let (group, group_bump) = address::model_group(&key(1), &group_market, &[4; 32]);
        let mut model_bytes = vec![0; fossa_sdk::accounts::MODEL_LEN];
        model_bytes[..8].copy_from_slice(Model::MAGIC);
        model_bytes[8..40].copy_from_slice(key(2).as_ref());
        model_bytes[40..72].copy_from_slice(&id);
        model_bytes[72..104].copy_from_slice(key(6).as_ref());
        model_bytes[104..136].copy_from_slice(key(7).as_ref());
        model_bytes[136..140].copy_from_slice(&4u32.to_le_bytes());
        model_bytes[140] = bump;
        let mut group_bytes = vec![0; fossa_sdk::accounts::MODEL_GROUP_LEN];
        group_bytes[..8].copy_from_slice(ModelGroup::MAGIC);
        group_bytes[8..40].copy_from_slice(group_market.as_ref());
        group_bytes[40..72].copy_from_slice(&[4; 32]);
        group_bytes[72..104].copy_from_slice(key(8).as_ref());
        group_bytes[104..136].copy_from_slice(address::reserve_mint(&key(1), &group).0.as_ref());
        group_bytes[136] = 1;
        group_bytes[137] = group_bump;
        let mock = mock(move |_, req| {
            assert_eq!(req["method"], "getAccountInfo");
            let at = req["params"][0].as_str().unwrap();
            let raw = |bytes: &[u8]| json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(bytes),"base64"]});
            let state = if at == key(2).to_string() {
                market_account(council)
            } else if at == model.to_string() {
                raw(&model_bytes)
            } else {
                assert_eq!(at, group.to_string());
                raw(&group_bytes)
            };
            rpc(req, json!({"context":{"slot":9},"value":state}))
        });
        let invoke = |args: &[&str]| {
            let mut command = cmd(dir.path());
            protocol_args(&mut command, &mock.url, council);
            command.args(args).output().unwrap()
        };
        let output = invoke(&[
            "model",
            "assign-group",
            "--model",
            "example/model",
            "--group",
            &group.to_string(),
        ]);
        if foreign {
            assert!(!output.status.success());
        } else {
            assert_eq!(result(output)["details"]["group"], group.to_string());
        }
        let clear = result(invoke(&[
            "model",
            "clear-group",
            "--model",
            "example/model",
        ]));
        assert!(clear["details"]["group"].is_null());
        assert_eq!(
            clear["instructions"][0]["accounts"][3]["address"],
            key(1).to_string()
        );
    }
}

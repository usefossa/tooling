use fossa_text_counter::TextCounter;
use serde_json::{Map, Value, json};

fn bundle(vocab: Value, merges: Value, text: &str, expected: usize) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "tokenizer": {
            "model": {"type": "BPE", "vocab": vocab, "merges": merges},
            "added_tokens": [], "normalizer": null, "pre_tokenizer": null,
            "post_processor": null, "decoder": null
        },
        "chatTemplate": "{{ messages[0].content }}",
        "checks": [{"messages": [{"role": "user", "content": text}], "tokens": expected}]
    }))
    .unwrap()
}

#[test]
fn both_merge_formats_preserve_the_last_rank_of_a_repeated_pair() {
    let vocab = json!({"a":0, "b":1, "c":2, "ab":3, "bc":4, "abc":5});
    for merges in [
        json!(["a b", "b c", "a b", "a bc"]),
        json!([["a", "b"], ["b", "c"], ["a", "b"], ["a", "bc"]]),
    ] {
        TextCounter::load(&bundle(vocab.clone(), merges, "abc", 1)).unwrap();
    }
}

#[test]
fn large_bpe_loads_under_the_production_heap_limit() {
    let alphabet: Vec<char> = (0x4e00..0x4e00 + 552)
        .map(|code| char::from_u32(code).unwrap())
        .collect();
    let mut vocab = Map::new();
    let mut merges = Vec::with_capacity(alphabet.len().pow(2));
    for &symbol in &alphabet {
        vocab.insert(symbol.to_string(), vocab.len().into());
    }
    for &left in &alphabet {
        for &right in &alphabet {
            vocab.insert(format!("{left}{right}"), vocab.len().into());
            merges.push(json!([left, right]));
        }
    }
    let bytes = bundle(vocab.into(), merges.into(), "一丁一", 2);
    TextCounter::load(&bytes).unwrap();
}

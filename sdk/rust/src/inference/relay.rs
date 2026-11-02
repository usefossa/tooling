use crate::{Error, Pubkey, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::header::HeaderMap;
use std::net::IpAddr;

const FIELDS: [&str; 4] = [
    "x-fossa-relay-key",
    "x-fossa-relay-at",
    "x-fossa-relay-ip",
    "x-fossa-relay-signature",
];
pub const MAX_AGE_SECS: u64 = 30;

fn field<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    ensure(
        headers.get_all(name).iter().count() == 1,
        "relay header count",
    )?;
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Invalid("relay header"))
}

/// verify a relay assertion before using its address for admission. pin trusted keys locally.
/// the assertion binds the entire wallet authorization; booking/body verification is still required.
pub fn client_ip(
    headers: &HeaderMap,
    provider: Pubkey,
    path: &str,
    trusted: &[ed25519_dalek::VerifyingKey],
    now: u64,
) -> Result<Option<IpAddr>> {
    if FIELDS.iter().all(|name| !headers.contains_key(*name)) {
        return Ok(None);
    }
    let key: Pubkey = field(headers, FIELDS[0])?
        .parse()
        .map_err(|_| Error::Invalid("relay key"))?;
    let key = trusted
        .iter()
        .find(|value| value.as_bytes() == &key.to_bytes())
        .ok_or(Error::Invalid("untrusted relay"))?;
    let at = field(headers, FIELDS[1])?;
    let timestamp: u64 = at.parse().map_err(|_| Error::Invalid("relay timestamp"))?;
    ensure(
        at == timestamp.to_string() && now.abs_diff(timestamp) <= MAX_AGE_SECS,
        "relay expiry",
    )?;
    let ip = field(headers, FIELDS[2])?;
    let address: IpAddr = ip.parse().map_err(|_| Error::Invalid("relay address"))?;
    ensure(
        ip == address.to_canonical().to_string(),
        "canonical relay address",
    )?;
    let signature = field(headers, FIELDS[3])?;
    ensure(signature.len() == 88, "relay signature length")?;
    let signature = STANDARD
        .decode(signature)
        .map_err(|_| Error::Invalid("relay signature"))?;
    let signature = ed25519_dalek::Signature::from_slice(&signature)
        .map_err(|_| Error::Invalid("relay signature"))?;
    let message = statement(headers, provider, path, at, ip)?;
    key.verify_strict(message.as_bytes(), &signature)
        .map_err(|_| Error::Invalid("relay signature"))?;
    Ok(Some(address))
}

pub fn statement(
    headers: &HeaderMap,
    provider: Pubkey,
    path: &str,
    at: &str,
    ip: &str,
) -> Result<String> {
    ensure(
        path.starts_with('/')
            && path.len() <= 2048
            && !path.bytes().any(|byte| byte.is_ascii_control()),
        "relay path",
    )?;
    let mut value = format!("fossa-relay-v1\n{at}\n{ip}\n{provider}\nPOST\n{path}");
    for name in [
        "x-fossa-booking",
        "x-fossa-generation",
        "x-fossa-user",
        "x-fossa-body-sha256",
        "x-fossa-signature",
    ] {
        value.push('\n');
        value.push_str(field(headers, name)?);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn fixture() -> (HeaderMap, ed25519_dalek::VerifyingKey, Pubkey) {
        let signer = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
        let provider = Pubkey::new_from_array([3; 32]);
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("x-fossa-booking", "booking"),
            ("x-fossa-generation", "7"),
            ("x-fossa-user", "wallet"),
            ("x-fossa-body-sha256", "hash"),
            ("x-fossa-signature", "authorization"),
            ("x-fossa-relay-at", "100"),
            ("x-fossa-relay-ip", "198.51.100.8"),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        headers.insert(
            "x-fossa-relay-key",
            Pubkey::new_from_array(signer.verifying_key().to_bytes())
                .to_string()
                .parse()
                .unwrap(),
        );
        let message = statement(
            &headers,
            provider,
            "/v1/chat/completions",
            "100",
            "198.51.100.8",
        )
        .unwrap();
        headers.insert(
            "x-fossa-relay-signature",
            STANDARD
                .encode(signer.sign(message.as_bytes()).to_bytes())
                .parse()
                .unwrap(),
        );
        (headers, signer.verifying_key(), provider)
    }
    #[test]
    fn relay_binds_request_and_rejects_spoofing_expiry_duplicates_and_unknown_keys() {
        let (headers, key, provider) = fixture();
        assert_eq!(
            headers["x-fossa-relay-signature"],
            "h5icz9HA+s0DEem6Qm22g1tjLGeERBTHfh0/rz8adSosZOxpL7aO+jpNcflEDvhcNwXnFUd/nGILU+3fdKYOAw=="
        );
        let verify =
            |headers: &HeaderMap| client_ip(headers, provider, "/v1/chat/completions", &[key], 110);
        assert_eq!(
            verify(&headers).unwrap(),
            Some("198.51.100.8".parse().unwrap())
        );
        assert_eq!(verify(&HeaderMap::new()).unwrap(), None);
        assert!(client_ip(&headers, provider, "/v1/chat/completions", &[], 110).is_err());
        assert!(client_ip(&headers, provider, "/v1/chat/completions", &[key], 131).is_err());
        assert!(
            client_ip(
                &headers,
                provider,
                "/v1/chat/completions/cancel",
                &[key],
                110
            )
            .is_err()
        );
        assert!(
            client_ip(
                &headers,
                Pubkey::new_from_array([4; 32]),
                "/v1/chat/completions",
                &[key],
                110
            )
            .is_err()
        );
        for name in FIELDS.into_iter().chain([
            "x-fossa-booking",
            "x-fossa-generation",
            "x-fossa-user",
            "x-fossa-body-sha256",
            "x-fossa-signature",
        ]) {
            let mut changed = headers.clone();
            changed.insert(name, "tampered".parse().unwrap());
            assert!(verify(&changed).is_err(), "{name}");
            let mut duplicate = headers.clone();
            duplicate.append(name, headers[name].clone());
            assert!(verify(&duplicate).is_err(), "duplicate {name}");
            let mut missing = headers.clone();
            missing.remove(name);
            assert!(verify(&missing).is_err(), "missing {name}");
        }
    }
}

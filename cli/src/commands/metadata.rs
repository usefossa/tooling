use super::*;

pub(super) fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Input(format!("file exceeds {limit} bytes")));
    }
    Ok(bytes)
}
pub(super) fn document(kind: MetadataKind, path: &Path) -> Result<Document> {
    Ok(Document::parse(
        kind.into(),
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
pub(super) fn model_document(id: &str, path: &Path) -> Result<Document> {
    Ok(Document::model(
        id,
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
pub(super) fn image(path: &Path) -> Result<(&'static str, Vec<u8>)> {
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
pub(super) fn icon(path: &Path) -> Result<Vec<u8>> {
    let bytes = read_limited(path, fossa_sdk::schema::MAX_ICON_BYTES)?;
    fossa_sdk::schema::validate_icon(&bytes).map_err(fossa_sdk::Error::Metadata)?;
    Ok(bytes)
}
pub(super) fn validate_uri(uri: Option<&str>) -> Result<()> {
    if let Some(uri) = uri {
        fossa_sdk::metadata::validate_uri(uri)?;
    }
    Ok(())
}

pub(super) fn document_view(doc: &Document) -> Value {
    json!({"sha256":runtime::hex(&doc.sha256()),"bytes":doc.bytes().len(),"document":doc.value()})
}
pub(super) fn reference_view(reference: &DocumentRef) -> Value {
    json!({"sha256":runtime::hex(&reference.sha256),"uri":reference.uri})
}
pub(super) fn hosted_reference(config: &Config, doc: &Document) -> Result<DocumentRef> {
    Ok(doc.reference(format!(
        "{}/v1/market/metadata/{}",
        config.api_url.trim_end_matches('/'),
        runtime::hex(&doc.sha256())
    ))?)
}
pub(super) async fn publish(
    rt: &Runtime,
    doc: &Document,
    uri: Option<&str>,
) -> Result<DocumentRef> {
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

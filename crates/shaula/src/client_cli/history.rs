use super::*;
use shaula_client::Query;
use std::{collections::HashSet, io::Write};
fn filters(filters: &Filters) -> Query {
    let mut query = Vec::new();
    for (key, value) in [
        ("fleet_key", &filters.fleet),
        ("status", &filters.status),
        ("repository", &filters.repository),
        ("job_name", &filters.job_name),
        ("association", &filters.association),
        ("cursor", &filters.cursor),
    ] {
        if let Some(value) = value {
            query.push((key.into(), value.clone()));
        }
    }
    for (key, value) in [
        ("since", filters.since),
        ("until", filters.until),
        ("limit", filters.limit.map(i64::from)),
    ] {
        if let Some(value) = value {
            query.push((key.into(), value.to_string()));
        }
    }
    query
}
pub(super) async fn run(
    client: &Client,
    kind: &str,
    action: HistoryAction,
) -> Result<Outcome, Error> {
    match action {
        HistoryAction::Get { id } => Ok(Outcome::data(match kind {
            "jobs" => client.job(&id).await?.data,
            "generations" => client.generation(&id).await?.data,
            _ => return Err(Error::Invalid("invocations supports list <generation-id>")),
        })),
        HistoryAction::Finalize {
            id,
            reason,
            yes,
            confirmed_absent,
            idempotency_key,
        } => {
            if kind != "generations" || !confirmed_absent {
                return Err(Error::Invalid("finalize requires generations and --confirmed-absent after out-of-band verification"));
            }
            confirm(
                yes,
                &format!("finalize generation {id}; this only updates the ledger"),
            )?;
            let attempt = client.finalize(&id, &reason, idempotency_key).await?;
            let key = attempt.idempotency_key().to_owned();
            match client.execute_finalize(&attempt).await {
                Ok(result) => {
                    let mut out = Outcome::data(result.data);
                    out.receipt = json!({"outcome":"completed","resource_kind":"generation","resource_key":id,"idempotency_key":key});
                    Ok(out)
                }
                Err(e) => {
                    let uncertain = matches!(
                        e,
                        Error::Transport
                            | Error::Protocol
                            | Error::Http {
                                status: 500..=599,
                                ..
                            }
                    );
                    let mut out = Outcome::error(e);
                    if uncertain {
                        out.receipt = json!({"outcome":"uncertain","idempotency_key":key});
                    }
                    Ok(out)
                }
            }
        }
        HistoryAction::List {
            id,
            filters: options,
        } => {
            if kind == "jobs" && options.association.is_some()
                || kind == "generations"
                    && (options.repository.is_some() || options.job_name.is_some())
            {
                return Err(Error::Invalid("unsupported history filter"));
            }
            if kind == "invocations"
                && (options.fleet.is_some()
                    || options.status.is_some()
                    || options.association.is_some()
                    || options.repository.is_some()
                    || options.job_name.is_some()
                    || options.since.is_some()
                    || options.until.is_some())
            {
                return Err(Error::Invalid(
                    "invocation list accepts only cursor and limit",
                ));
            }
            let mut query = filters(&options);
            let mut seen = HashSet::new();
            let mut items = Vec::new();
            let mut metadata = serde_json::Map::new();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
            loop {
                let result = match kind {
                    "jobs" => client.jobs(&query).await,
                    "generations" => client.generations(&query).await,
                    _ => {
                        client
                            .invocations(
                                id.as_deref().ok_or(Error::Invalid(
                                    "invocations list requires generation ID",
                                ))?,
                                &query,
                            )
                            .await
                    }
                };
                let page = match result {
                    Ok(result) => result.data.decode::<Value>().map_err(|_| Error::Protocol)?,
                    Err(e) => {
                        let mut out = Outcome::error(e);
                        metadata.insert("items".into(), json!(items));
                        out.data = Outcome::data(metadata).data;
                        out.partial = !items.is_empty();
                        return Ok(out);
                    }
                };
                if !options.all {
                    return Ok(Outcome::data(page));
                }
                for key in ["latest_create", "latest_destroy"] {
                    if let Some(value) = page.get(key) {
                        metadata.insert(key.into(), value.clone());
                    }
                }
                let next = page
                    .get("next_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let entries = page
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or(Error::Protocol)?;
                let available = options.max_items.saturating_sub(items.len());
                items.extend(entries.iter().take(available).cloned());
                if items.len() >= options.max_items || tokio::time::Instant::now() >= deadline {
                    metadata.insert("items".into(), json!(items));
                    metadata.insert("next_cursor".into(), json!(next));
                    let mut out = Outcome::data(metadata);
                    out.partial = next.is_some() || entries.len() > available;
                    return Ok(out);
                }
                let Some(next) = next else {
                    metadata.insert("items".into(), json!(items));
                    metadata.insert("next_cursor".into(), Value::Null);
                    return Ok(Outcome::data(metadata));
                };
                if !seen.insert(next.clone()) {
                    let mut out = Outcome::error(Error::Protocol);
                    metadata.insert("items".into(), json!(items));
                    out.data = Outcome::data(metadata).data;
                    out.partial = true;
                    return Ok(out);
                }
                query.retain(|(key, _)| key != "cursor");
                query.push(("cursor".into(), next));
            }
        }
    }
}

pub(super) async fn logs(client: &Client, action: LogAction) -> Result<Outcome, Error> {
    let (args, mut file) = match action {
        LogAction::Read(args) => (args, None),
        LogAction::Download { args, file } => (args, Some(super::secure_file::create(&file)?)),
    };
    let mut query = Vec::new();
    for (key, value) in [
        ("cursor", &args.cursor),
        ("phase", &args.phase),
        ("stream", &args.stream),
    ] {
        if let Some(value) = value {
            query.push((key.into(), value.clone()));
        }
    }
    if let Some(limit) = args.limit_bytes {
        query.push(("limit_bytes".into(), limit.to_string()));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(args.timeout);
    let mut seen = HashSet::new();
    let mut content_version = None;
    let mut cursors = HashSet::new();
    let mut emitted = false;
    loop {
        let result = client
            .logs(&args.id, &query)
            .await
            .and_then(|r| r.data.decode::<Value>().map_err(|_| Error::Protocol));
        let mut page = match result {
            Ok(page) => page,
            Err(error) => {
                let mut out = Outcome::error(error);
                out.partial = emitted;
                return Ok(out);
            }
        };
        let version = page.get("content_version").cloned();
        if content_version.is_some() && content_version != version {
            let mut out = Outcome::error(Error::Tracking(
                "log content version changed; refresh the invocation".into(),
            ));
            out.partial = emitted;
            return Ok(out);
        }
        content_version = version;
        let next = page
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let idle = page
            .get("entries")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
        if let Some(entries) = page.get_mut("entries").and_then(Value::as_array_mut) {
            entries.retain(|entry| {
                seen.insert(
                    json!([
                        entry.get("command_ordinal"),
                        entry.get("sequence"),
                        entry.get("phase"),
                        entry.get("stream")
                    ])
                    .to_string(),
                )
            });
            if seen.len() > 100_000 {
                return Err(Error::Tracking("log follow memory budget reached".into()));
            }
        }
        if let Some(file) = &mut file {
            writeln!(file,"{}",json!({"schema_version":1,"event":"log_page","data":page,"metadata":{"partial":false}})).map_err(|_|Error::Invalid("log download write failed"))?;
        } else if args.follow {
            event("log_page", page.clone(), false);
        } else {
            return Ok(Outcome::data(page));
        }
        emitted = true;
        if let Some(next) = next {
            let same_cursor = query
                .iter()
                .any(|(key, value)| key == "cursor" && value == &next);
            // An unsealed invocation deliberately returns the current cursor
            // while idle. Follow may poll it; a finite download stops partial.
            if idle && same_cursor && !args.follow {
                return Ok(Outcome {
                    partial: true,
                    ..Outcome::data(json!({"downloaded":true,"capture":page}))
                });
            }
            if !(cursors.insert(next.clone()) || idle && same_cursor) {
                let mut out = Outcome::error(Error::Protocol);
                out.partial = true;
                return Ok(out);
            }
            query.retain(|(key, _)| key != "cursor");
            query.push(("cursor".into(), next));
        } else if !args.follow {
            return Ok(Outcome::data(json!({"downloaded":true,"capture":page})));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Outcome {
                partial: true,
                ..Outcome::data(Value::Null)
            });
        }
        tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(1))=>{},_=tokio::signal::ctrl_c()=>{return Ok(Outcome{code:130,partial:true,..Outcome::data(Value::Null)});}}
    }
}

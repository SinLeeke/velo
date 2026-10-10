//! G3 live probe: where do "Tus canciones más escuchadas" / "Recomendado para ti" come from?
//! cargo run -p velo-player --example top_probe [-- <spclient path with {user}> | "pf|<op>|<sha256>|<json vars>"]
//! Env: TOP_PROBE_OUT=<dir> saves bodies locally; RECS=1 also tries radio-for-track.
//! Saved audio login; prints status/shape only (never tokens or raw bodies).
use anyhow::Result;
use http::Method;
use serde_json::Value;
use velo_player::{Engine, EngineConfig};

fn shape(v: &Value, depth: usize) -> String {
    match v {
        Value::Object(m) if depth > 0 => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{k}:{}", shape(v, depth - 1)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(m) => format!("obj({})", m.len()),
        Value::Array(a) => format!("arr({})", a.len()),
        Value::String(s) => format!("str({})", s.len()),
        other => other.to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let session = engine.session();
    let user = session.username();
    // {user} in an argument is replaced by the session user name.
    let mut paths: Vec<String> = Vec::new();
    paths.extend(std::env::args().skip(1).map(|a| a.replace("{user}", &user)));
    for p in paths {
        // "pf|<operationName>|<sha256>|<variables json>": pathfinder persisted query.
        if let Some(rest) = p.strip_prefix("pf|") {
            let parts: Vec<&str> = rest.splitn(3, '|').collect();
            let vars: Value = serde_json::from_str(parts[2])?;
            let shown = parts[0];
            match velo_player::session_api::pathfinder::query(session, parts[0], &vars, parts[1])
                .await
            {
                Ok(v) => {
                    println!(
                        "OK  pf {shown}
    {}",
                        shape(&v, 4)
                    );
                    if let Ok(dir) = std::env::var("TOP_PROBE_OUT") {
                        std::fs::write(format!("{dir}/{shown}.json"), v.to_string())?;
                    }
                }
                Err(e) => println!("ERR pf {shown}: {e}"),
            }
            continue;
        }
        let shown = p.replace(&user, "<user>");
        match session
            .spclient()
            .request_as_json(&Method::GET, &p, None, None)
            .await
        {
            Ok(b) => match serde_json::from_slice::<Value>(&b) {
                Ok(v) => {
                    if let Ok(dir) = std::env::var("TOP_PROBE_OUT") {
                        let name = shown
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                            .collect::<String>();
                        let name = &name[..name.len().min(60)];
                        std::fs::write(format!("{dir}/{name}.json"), b.as_ref())?;
                    }
                    println!("OK  {shown}\n    {}", shape(&v, 3));
                    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
                    let text = v.to_string();
                    for part in text.split("\"spotify:").skip(1) {
                        let kind = part.split(':').next().unwrap_or("").to_string();
                        *kinds.entry(kind).or_default() += 1;
                    }
                    println!("    uri kinds: {kinds:?}");
                }
                Err(_) => println!("OK  {shown} ({} bytes, not json)", b.len()),
            },
            Err(e) => println!("ERR {shown}: {e}"),
        }
    }
    // RECS=1: radio of the first recently played track through the session (1 request).
    engine.shutdown().await;
    Ok(())
}

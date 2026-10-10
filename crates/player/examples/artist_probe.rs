//! F6: ejecutar con red; imprime solo metadatos públicos y forma de respuestas, sin tokens.
use anyhow::{Context, Result};
use velo_player::{Engine, EngineConfig, artist};

// Solo claves y tipos, sin valores desconocidos ni cuerpos/encabezados sensibles.
fn inspect_shape(value: &serde_json::Value, path: &str, depth: usize) {
    if depth > 4 {
        return;
    }
    match value {
        serde_json::Value::Object(fields) => {
            println!(
                "artistview {path}: claves={:?}",
                fields.keys().take(40).collect::<Vec<_>>()
            );
            for (key, child) in fields.iter().take(40) {
                inspect_shape(child, &format!("{path}.{key}"), depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            println!("artistview {path}: array {} elementos", items.len());
            if let Some(first) = items.first() {
                inspect_shape(first, &format!("{path}[0]"), depth + 1);
            }
        }
        _ => {}
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let uri = std::env::args()
        .nth(1)
        .context("Indica spotify:artist:<id>")?;
    artist::validate_artist_uri(&uri)?;
    let engine = Engine::from_keyring(EngineConfig::velo("Velo")).await?;
    let result = async {
        if let Ok(profile) = artist::profile(&engine, &uri).await {
        println!("ARTIST_V4: biografía={} caracteres; fotos={}; populares={}; relacionados={}; discos={}; aparece_en={}; parcial={}",
            profile.biography.chars().count(), profile.photos.len(), profile.popular.len(), profile.related.len(),
            profile.discography.len(), profile.appears_on.len(), profile.incomplete);
        for track in &profile.popular { println!("Popular: {} · {} · {}", track.uri, track.title, track.artist_line()); }
        for kind in ["album", "single", "ep", "compilation"] {
            println!("Discografía {kind}: {}", profile.discography.iter().filter(|a| a.album_type == kind).count());
        }
        } else { println!("ARTIST_V4 no disponible para esta sesión"); }
        println!("ARTIST_V4 no incluye oyentes mensuales, ciudades ni verificación; no son cero ni false.");
        // Ruta candidata histórica, sin depender de ella en producción. Un 404/403 no prueba
        // que Spotify no tenga el dato; solo descarta esta ruta para esta sesión.
        let id = uri.strip_prefix("spotify:artist:").context("URI de artista inválida")?;
        let endpoint = format!("/artistview/v1/artist/{id}?platform=desktop");
        match tokio::time::timeout(std::time::Duration::from_secs(12), engine.session().spclient().get_next_page(&endpoint)).await {
            Ok(Ok(bytes)) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                inspect_shape(&value, "root", 0);
                let fields: Vec<_> = value.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                println!("artistview candidato: claves raíz={fields:?}");
                for field in ["stats", "statistics", "monthlyListeners", "topCities", "verified"] {
                    if let Some(v) = value.get(field) {
                        println!("{field}: tipo={}, claves={:?}", if v.is_object() { "object" } else if v.is_array() { "array" } else { "scalar" },
                            v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()));
                    }
                }
            }
            Ok(Err(_)) => println!("artistview candidato: rechazado/no disponible (sin cuerpo ni error sensible)"),
            Err(_) => println!("artistview candidato: timeout"),
        }
        Ok(())
    }.await;
    engine.shutdown().await;
    result
}

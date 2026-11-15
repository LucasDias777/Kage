//! Versões publicadas do Kage: a última release do repositório público no GitHub. A janela usa
//! para a tela de Atualizações; o motor, para o item de atualização do menu da bandeja.

use serde::Deserialize;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;
use windows_core::HSTRING;

/// Repositório público de onde vêm as versões.
pub const REPO: &str = "LucasDias777/Kage";

#[derive(Clone, Debug)]
pub struct Release {
    /// Sem o "v" da tag: "0.2.0".
    pub version: String,
    /// Texto da release no GitHub (Markdown simples).
    pub notes: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// SHA-256 em hexadecimal, quando o GitHub publica (`digest: "sha256:..."`).
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// Versão desta cópia. Em compilação de teste, `KAGE_PRETEND_VERSION` finge uma mais velha.
pub fn current_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("KAGE_PRETEND_VERSION") {
        return v;
    }
    env!("CARGO_PKG_VERSION").to_string()
}

/// "v1.10.2-beta" vira [1, 10, 2]; o que vem depois do "-" não conta.
fn parts(version: &str) -> Vec<u64> {
    let core = version.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("");
    core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parts(candidate), parts(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// Cliente HTTP com os cabeçalhos que a API do GitHub pede.
pub fn http_client() -> windows_core::Result<HttpClient> {
    let client = HttpClient::new()?;
    let headers = client.DefaultRequestHeaders()?;
    headers.TryAppendWithoutValidation(&HSTRING::from("User-Agent"), &HSTRING::from(format!("Kage/{}", env!("CARGO_PKG_VERSION"))))?;
    headers.TryAppendWithoutValidation(&HSTRING::from("Accept"), &HSTRING::from("application/vnd.github+json"))?;
    Ok(client)
}

/// A última release publicada. `Ok(None)` quando o repositório ainda não tem nenhuma.
pub fn latest() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = http_client()
        .and_then(|c| c.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.join())
        .map_err(|e| format!("sem conexão com o GitHub ({})", e.message()))?;
    let status = response.StatusCode().map(|s| s.0).unwrap_or(0);
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(match status {
            403 | 429 => "o GitHub pediu para esperar um pouco antes de procurar de novo".into(),
            _ => format!("o GitHub respondeu {status}"),
        });
    }
    let text = response
        .Content()
        .and_then(|c| c.ReadAsStringAsync()?.join())
        .map_err(|e| format!("resposta do GitHub incompleta ({})", e.message()))?;
    let api: ApiRelease = serde_json::from_str(&text.to_string()).map_err(|e| format!("resposta do GitHub estranha ({e})"))?;
    Ok(Some(Release {
        version: api.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: api.body.unwrap_or_default(),
        assets: api
            .assets
            .into_iter()
            .map(|a| Asset {
                sha256: a.digest.and_then(|d| d.strip_prefix("sha256:").map(str::to_ascii_lowercase)),
                name: a.name,
                url: a.browser_download_url,
                size: a.size,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }
}

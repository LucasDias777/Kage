//! Atualização pelo GitHub: compara a versão instalada com a última release pública do
//! repositório, baixa o `kage.exe` e o `kage-engine.exe` novos, confere tamanho e SHA-256 e troca
//! os da pasta do Kage. Não precisa de login: o repositório é público.
//!
//! A troca usa o fato de o Windows deixar renomear um .exe aberto: o atual vira `.old`, o novo
//! entra no lugar e o motor é reaberto. Os `.old` saem na próxima abertura da janela.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::Foundation::Uri;
use windows::Security::Cryptography::Core::{HashAlgorithmNames, HashAlgorithmProvider};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::IBuffer;
use windows::Web::Http::{HttpClient, HttpCompletionOption};
use windows_core::{Array, HSTRING};
use windows_future::{AsyncOperationProgressHandler, IAsyncOperationWithProgress};

/// Repositório público de onde vêm as versões.
pub const REPO: &str = "LucasDias777/Kage";
/// Os dois arquivos que toda release precisa ter.
pub const FILES: [&str; 2] = ["kage.exe", "kage-engine.exe"];
/// Pasta temporária do download, dentro da pasta do Kage (mesmo disco, então a troca é um rename).
const STAGING: &str = ".atualizacao";

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

fn client() -> windows_core::Result<HttpClient> {
    let client = HttpClient::new()?;
    let headers = client.DefaultRequestHeaders()?;
    headers.TryAppendWithoutValidation(&HSTRING::from("User-Agent"), &HSTRING::from(format!("Kage/{}", env!("CARGO_PKG_VERSION"))))?;
    headers.TryAppendWithoutValidation(&HSTRING::from("Accept"), &HSTRING::from("application/vnd.github+json"))?;
    Ok(client)
}

/// A última release publicada. `Ok(None)` quando o repositório ainda não tem nenhuma.
pub fn latest() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = client()
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

/// Pasta onde o Kage está instalado: a deste .exe.
pub fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Cópia compilada pelo `cargo` (pasta `target`): ela se atualiza pelo código, não pelo GitHub.
pub fn is_dev_build(dir: &Path) -> bool {
    dir.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"))
}

/// Baixa os dois arquivos da release para a pasta temporária, conferindo cada um.
/// `progress` recebe os bytes já baixados e o total.
pub fn download(release: &Release, dir: &Path, progress: impl Fn(u64, u64) + Clone + Send + 'static) -> Result<Vec<PathBuf>, String> {
    let wanted: Vec<&Asset> = FILES
        .iter()
        .map(|name| release.assets.iter().find(|a| a.name.eq_ignore_ascii_case(name)))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("a versão {} não tem os arquivos do Kage", release.version))?;
    let total: u64 = wanted.iter().map(|a| a.size).sum();
    let staging = dir.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("a pasta do Kage não aceita gravação ({e})"))?;
    let client = client().map_err(|e| e.message())?;
    let mut done = 0u64;
    let mut out = Vec::new();
    for asset in wanted {
        let bytes = fetch(&client, asset, done, total, progress.clone())?;
        if bytes.len() as u64 != asset.size {
            return Err(format!("{} chegou incompleto ({} de {} bytes)", asset.name, bytes.len(), asset.size));
        }
        if let Some(expected) = &asset.sha256
            && sha256(&bytes)? != *expected
        {
            return Err(format!("{} não confere com o SHA-256 publicado", asset.name));
        }
        let path = staging.join(&asset.name);
        std::fs::write(&path, &bytes).map_err(|e| format!("não gravou {} ({e})", asset.name))?;
        done += asset.size;
        progress(done, total);
        out.push(path);
    }
    Ok(out)
}

fn fetch(client: &HttpClient, asset: &Asset, before: u64, total: u64, progress: impl Fn(u64, u64) + Send + 'static) -> Result<Vec<u8>, String> {
    let failed = |e: windows_core::Error| format!("o download de {} parou ({})", asset.name, e.message());
    let uri = Uri::CreateUri(&HSTRING::from(asset.url.as_str())).map_err(failed)?;
    let response = client.GetWithOptionAsync(&uri, HttpCompletionOption::ResponseHeadersRead).and_then(|op| op.join()).map_err(failed)?;
    let status = response.StatusCode().map(|s| s.0).unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("o GitHub respondeu {status} ao baixar {}", asset.name));
    }
    let reading: IAsyncOperationWithProgress<IBuffer, u64> = response.Content().and_then(|c| c.ReadAsBufferAsync()).map_err(failed)?;
    let _ = reading.SetProgress(&AsyncOperationProgressHandler::new(move |_, read| {
        progress(before + *read, total);
        Ok(())
    }));
    let buffer = reading.join().map_err(failed)?;
    let mut bytes = Array::<u8>::new();
    CryptographicBuffer::CopyToByteArray(&buffer, &mut bytes).map_err(failed)?;
    Ok(bytes.to_vec())
}

fn sha256(bytes: &[u8]) -> Result<String, String> {
    let hash = (|| {
        let provider = HashAlgorithmProvider::OpenAlgorithm(&HashAlgorithmNames::Sha256()?)?;
        let digest = provider.HashData(&CryptographicBuffer::CreateFromByteArray(bytes)?)?;
        CryptographicBuffer::EncodeToHexString(&digest)
    })();
    hash.map(|h| h.to_string().to_ascii_lowercase()).map_err(|e| format!("não calculou o SHA-256 ({})", e.message()))
}

/// Troca os .exe da pasta pelos baixados. Se um falhar, devolve os que já tinha trocado.
pub fn install(dir: &Path, downloaded: &[PathBuf]) -> Result<(), String> {
    let mut swapped: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |swapped: &[(PathBuf, PathBuf)]| {
        for (target, old) in swapped.iter().rev() {
            let _ = std::fs::remove_file(target);
            let _ = std::fs::rename(old, target);
        }
    };
    for new in downloaded {
        let Some(name) = new.file_name() else { continue };
        let target = dir.join(name);
        let old = free_old_name(&target);
        if target.exists()
            && let Err(e) = std::fs::rename(&target, &old)
        {
            undo(&swapped);
            return Err(format!("não conseguiu trocar {} ({e})", name.to_string_lossy()));
        }
        if let Err(e) = std::fs::rename(new, &target) {
            let _ = std::fs::rename(&old, &target);
            undo(&swapped);
            return Err(format!("não conseguiu trocar {} ({e})", name.to_string_lossy()));
        }
        swapped.push((target, old));
    }
    let _ = std::fs::remove_dir_all(dir.join(STAGING));
    Ok(())
}

/// `kage.exe.old`, ou `kage.exe.1.old` se o anterior ainda estiver aberto e não sair.
fn free_old_name(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut candidate = target.with_file_name(format!("{name}.old"));
    let mut n = 1;
    while candidate.exists() && std::fs::remove_file(&candidate).is_err() {
        candidate = target.with_file_name(format!("{name}.{n}.old"));
        n += 1;
    }
    candidate
}

/// Apaga o que sobrou da última atualização (os .exe antigos e downloads pela metade).
pub fn clean_leftovers() {
    let Some(dir) = install_dir() else { return };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".old") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir.join(STAGING));
}

/// Markdown da release em texto simples para a janela: sem `#`, `**` e com "•" nas listas.
/// O que vem depois de um comentário `<!--` fica só na página da release (como instalar).
pub fn plain_notes(markdown: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in markdown.lines().take_while(|l| !l.trim_start().starts_with("<!--")) {
        let line = raw.trim_end().replace("**", "").replace('`', "");
        let trimmed = line.trim_start();
        let text = if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            format!("•  {rest}")
        } else {
            trimmed.trim_start_matches('#').trim_start().to_string()
        };
        if text.is_empty() && lines.last().is_none_or(|l: &String| l.is_empty()) {
            continue;
        }
        lines.push(text);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
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

    #[test]
    fn notes_lose_markdown_marks() {
        let md = "## Novidades\n\n- **Player** sem atraso\n* `kage.exe` menor\n\n\nFim\n\n<!-- só na página -->\n## Como instalar\nBaixe o zip";
        assert_eq!(plain_notes(md), "Novidades\n\n•  Player sem atraso\n•  kage.exe menor\n\nFim");
    }

    #[test]
    fn dev_builds_are_detected_by_the_target_folder() {
        assert!(is_dev_build(Path::new(r"C:\code\Kage\target\release")));
        assert!(!is_dev_build(Path::new(r"C:\Users\x\AppData\Local\Programs\Kage")));
    }

    #[test]
    fn install_swaps_files_and_keeps_the_old_ones_aside() {
        let dir = std::env::temp_dir().join(format!("kage-update-test-{}", std::process::id()));
        let staging = dir.join(STAGING);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(dir.join("kage.exe"), b"velho").unwrap();
        std::fs::write(staging.join("kage.exe"), b"novo").unwrap();
        install(&dir, &[staging.join("kage.exe")]).unwrap();
        assert_eq!(std::fs::read(dir.join("kage.exe")).unwrap(), b"novo");
        assert_eq!(std::fs::read(dir.join("kage.exe.old")).unwrap(), b"velho");
        assert!(!staging.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

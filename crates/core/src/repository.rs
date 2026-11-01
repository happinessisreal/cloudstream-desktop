use crate::{models::*, store::Store};
use anyhow::{bail, Context, Result};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use url::Url;

const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const MAX_PACKAGE: usize = 128 * 1024 * 1024;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn plugin_id(repository: &str, internal_name: &str) -> String {
    digest(format!("{repository}\0{internal_name}").as_bytes())
}
pub fn normalize_repository(input: &str) -> Result<String> {
    let input = input.trim();
    let value = if let Some(value) = input.strip_prefix("cloudstreamrepo://") {
        if value.starts_with("https://") || value.starts_with("http://") {
            value.to_owned()
        } else {
            format!("https://{value}")
        }
    } else if let Some(value) = input
        .strip_prefix("https://cs.repo/?")
        .or_else(|| input.strip_prefix("https://cs.repo/"))
    {
        if value.starts_with("https://") || value.starts_with("http://") {
            value.to_owned()
        } else {
            format!("https://{value}")
        }
    } else {
        input.to_owned()
    };
    let mut url = network_url(&value)?;
    url.set_fragment(None);
    Ok(url.to_string())
}

pub async fn resolve_repository_url(client: &Client, input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Repository URL or short-code cannot be empty");
    }

    if let Some(code) = trimmed.strip_prefix('!') {
        let code = code.trim();
        if code.is_empty() {
            bail!("Short-code after '!' cannot be empty");
        }
        let resolver_url = format!("https://py.md/{code}");
        resolve_shortcode_redirect(client, &resolver_url)
            .await
            .with_context(|| format!("Short-code resolver is unreachable or code could not be resolved: !{code}"))
    } else if trimmed.starts_with("cutt.ly/") || (!trimmed.contains("://") && !trimmed.contains('/') && !trimmed.contains('.') && trimmed.len() <= 32) {
        let code = trimmed.strip_prefix("cutt.ly/").unwrap_or(trimmed).trim();
        if code.is_empty() {
            bail!("Cutt.ly short-code cannot be empty");
        }
        let resolver_url = format!("https://cutt.ly/{code}");
        resolve_shortcode_redirect(client, &resolver_url)
            .await
            .with_context(|| format!("Short-code resolver is unreachable or code could not be resolved: {code}"))
    } else {
        normalize_repository(trimmed)
    }
}

pub async fn resolve_shortcode_redirect(client: &Client, url: &str) -> Result<String> {
    let target = network_url(url)?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.get(target).send(),
    )
    .await
    .context("Short-code resolution timed out")?
    .context("Could not connect to short-code resolver")?;

    let status = response.status();
    if !status.is_success() && !status.is_redirection() {
        bail!("Short-code resolver returned HTTP status {}", status);
    }
    let final_url = response.url().as_str();
    if (final_url.contains("py.md") || final_url.contains("cutt.ly")) && !final_url.ends_with(".json") {
        bail!("Short-code did not redirect to a repository URL: {}", final_url);
    }
    normalize_repository(final_url)
}
pub fn network_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).context("Enter a complete http:// or https:// URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Only HTTP and HTTPS URLs are supported")
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URLs containing credentials are not supported")
    }
    Ok(url)
}
pub async fn download(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client
        .get(network_url(url)?)
        .send()
        .await
        .context("Could not reach the server")?
        .error_for_status()
        .context("Server rejected the download")?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        bail!("Download exceeds the size limit")
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > limit {
            bail!("Download exceeds the size limit")
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn fetch_repository(client: &Client, input: &str) -> Result<Repository> {
    let url = normalize_repository(input)?;
    let manifest: RepositoryManifest =
        serde_json::from_slice(&download(client, &url, MAX_MANIFEST).await?)
            .context("This URL did not return a Cloudstream repository manifest")?;
    if manifest.manifest_version != 1 {
        bail!(
            "Unsupported repository manifest version: {}",
            manifest.manifest_version
        )
    }
    if manifest.plugin_lists.len() > 32 {
        bail!("Repository contains too many plugin lists")
    }
    let base = Url::parse(&url)?;
    let mut plugins = Vec::new();
    let mut identities = std::collections::HashSet::new();
    for list in &manifest.plugin_lists {
        let list_url = base.join(list)?.to_string();
        let entries: Vec<PluginEntry> =
            serde_json::from_slice(&download(client, &list_url, MAX_MANIFEST).await?)
                .context("Could not read the extension list")?;
        for mut entry in entries {
            if entry.internal_name.is_empty() {
                bail!("Extension is missing an internalName")
            }
            entry.url = Url::parse(&list_url)?.join(&entry.url)?.to_string();
            if let Some(jar) = &entry.jar_url {
                entry.jar_url = Some(Url::parse(&list_url)?.join(jar)?.to_string());
            }
            if identities.insert(entry.internal_name.clone()) {
                plugins.push(entry);
            }
        }
    }
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Repository {
        url,
        manifest,
        plugins,
        custom_name: None,
        updated_at: None,
        extra: std::collections::BTreeMap::new(),
    })
}

#[derive(Debug)]
pub struct PackageInfo {
    pub runtime: RuntimeKind,
    pub plugin_class: Option<String>,
}
pub fn inspect_package(bytes: &[u8]) -> Result<PackageInfo> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .context("Extension is not a valid ZIP/JAR package")?;
    if zip.len() > 20_000 {
        bail!("Extension contains too many files")
    }
    let has_dex = zip.file_names().any(|name| name == "classes.dex");
    let has_classes = zip.file_names().any(|name| name.ends_with(".class"));
    let plugin_class = match zip.by_name("manifest.json") {
        Ok(mut entry) => {
            if entry.size() > 64 * 1024 {
                bail!("Extension manifest is too large")
            }
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            let manifest: serde_json::Value = serde_json::from_str(&text)?;
            manifest
                .get("pluginClassName")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        }
        Err(_) => None,
    };
    if has_dex {
        Ok(PackageInfo {
            runtime: RuntimeKind::Android,
            plugin_class,
        })
    } else if has_classes {
        Ok(PackageInfo {
            runtime: RuntimeKind::Jvm,
            plugin_class,
        })
    } else {
        bail!("Package contains neither JVM classes nor Android DEX")
    }
}
pub fn verify_hash(bytes: &[u8], expected: Option<&str>) -> Result<bool> {
    let Some(expected) = expected else {
        return Ok(false);
    };
    let Some(expected) = expected.strip_prefix("sha256-") else {
        bail!("Unsupported extension hash format")
    };
    if digest(bytes) != expected.to_ascii_lowercase() {
        bail!("Extension hash mismatch. The previous version has been kept")
    }
    Ok(true)
}

pub fn extract_local_package_entry(path: &Path, bytes: &[u8]) -> Result<PluginEntry> {
    let package = inspect_package(bytes)?;
    let sha256 = digest(bytes);

    let mut name = None;
    let mut internal_name = None;
    let mut version = 1i64;
    let mut authors = Vec::new();
    let mut description = None;
    let mut tv_types = None;

    if let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) {
        if let Ok(mut entry) = zip.by_name("manifest.json") {
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_ok() {
                if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) {
                    name = manifest.get("name").and_then(|v| v.as_str()).map(str::to_owned);
                    internal_name = manifest.get("internalName").and_then(|v| v.as_str()).map(str::to_owned);
                    if let Some(ver) = manifest.get("version").and_then(|v| v.as_i64()) {
                        version = ver;
                    }
                    if let Some(arr) = manifest.get("authors").and_then(|v| v.as_array()) {
                        authors = arr.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                    }
                    description = manifest.get("description").and_then(|v| v.as_str()).map(str::to_owned);
                    if let Some(arr) = manifest.get("tvTypes").and_then(|v| v.as_array()) {
                        tv_types = Some(arr.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect());
                    }
                }
            }
        }
    }

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("local_extension");
    let internal_name = internal_name.unwrap_or_else(|| stem.to_string());
    let display_name = name.unwrap_or_else(|| {
        let title = stem.replace(['-', '_'], " ");
        let mut chars = title.chars();
        match chars.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        }
    });

    let (jar_url, jar_hash, jar_file_size, url, file_hash, file_size) = if package.runtime == RuntimeKind::Jvm {
        (
            Some(format!("file://{}", path.display())),
            Some(format!("sha256-{sha256}")),
            Some(bytes.len() as u64),
            format!("file://{}", path.display()),
            None,
            None,
        )
    } else {
        (
            None,
            None,
            None,
            format!("file://{}", path.display()),
            Some(format!("sha256-{sha256}")),
            Some(bytes.len() as u64),
        )
    };

    Ok(PluginEntry {
        url,
        name: display_name,
        internal_name,
        version,
        status: 1,
        api_version: 1,
        authors,
        description,
        language: Some("en".into()),
        icon_url: None,
        repository_url: Some("local://packages".into()),
        tv_types,
        file_size,
        file_hash,
        jar_url,
        jar_hash,
        jar_file_size,
        source_url: None,
        extra: std::collections::BTreeMap::new(),
    })
}

pub struct RepositoryManager {
    pub client: Client,
    pub directory: PathBuf,
}
impl RepositoryManager {
    pub fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        Ok(Self {
            client: Client::builder()
                .user_agent("CloudstreamDesktop/0.1.0")
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(120))
                .build()?,
            directory,
        })
    }
    pub async fn add(&self, store: &Store, url: &str) -> Result<Repository> {
        let repository = fetch_repository(&self.client, url).await?;
        store.put("repository", &repository.url, &repository)?;
        Ok(repository)
    }
    pub fn prepare_local(&self, file_path: &Path) -> Result<InstalledPlugin> {
        if !file_path.is_file() {
            bail!("Local package file not found: {}", file_path.display());
        }
        let bytes = std::fs::read(file_path)
            .with_context(|| format!("Could not read local package at {}", file_path.display()))?;
        if bytes.len() > MAX_PACKAGE {
            bail!("Local package exceeds maximum size limit ({} MB)", MAX_PACKAGE / (1024 * 1024));
        }
        let entry = extract_local_package_entry(file_path, &bytes)?;
        let package = inspect_package(&bytes)?;
        let hash = format!("sha256-{}", digest(&bytes));
        self.prepare_bytes(
            "local://packages",
            entry,
            &bytes,
            Some(&hash),
            package.runtime,
        )
    }
    pub async fn prepare(
        &self,
        repository_url: &str,
        entry: &PluginEntry,
    ) -> Result<InstalledPlugin> {
        if entry.status == 0 {
            bail!("This extension has been disabled by its repository")
        }
        let (url, hash, runtime) = match entry.jar_url.as_deref().filter(|s| !s.is_empty()) {
            Some(url) => (url, entry.jar_hash.as_deref(), RuntimeKind::Jvm),
            None => (
                entry.url.as_str(),
                entry.file_hash.as_deref(),
                RuntimeKind::Android,
            ),
        };
        let bytes = download(&self.client, url, MAX_PACKAGE).await?;
        self.prepare_bytes(repository_url, entry.clone(), &bytes, hash, runtime)
    }
    pub fn prepare_bytes(
        &self,
        repository_url: &str,
        entry: PluginEntry,
        bytes: &[u8],
        expected_hash: Option<&str>,
        expected_runtime: RuntimeKind,
    ) -> Result<InstalledPlugin> {
        let verified = verify_hash(bytes, expected_hash)?;
        let package = inspect_package(bytes)?;
        if package.runtime != expected_runtime {
            bail!("Extension package format does not match its repository entry")
        }
        let id = plugin_id(repository_url, &entry.internal_name);
        let sha256 = digest(bytes);
        let directory = self.directory.join(&id);
        std::fs::create_dir_all(&directory)?;
        let suffix = if package.runtime == RuntimeKind::Jvm {
            "jar"
        } else {
            "cs3"
        };
        let path = directory.join(format!("{sha256}.{suffix}"));
        atomic_write(&path, bytes)?;
        Ok(InstalledPlugin {
            id,
            repository_url: repository_url.into(),
            entry,
            runtime: package.runtime,
            path: path.to_string_lossy().into(),
            sha256,
            verified,
            plugin_class: package.plugin_class,
            installed_at: timestamp(),
        })
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

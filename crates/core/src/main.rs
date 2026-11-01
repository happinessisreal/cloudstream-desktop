use anyhow::{bail, Context, Result};
use cloudstream_core::{models::*, App};
use serde_json::json;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = std::env::var_os("CLOUDSTREAM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".local-data"));
    let resources = std::env::var_os("CLOUDSTREAM_RESOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("src-tauri/resources"));
    let app = App::open(data, resources)?;
    let result=match command.as_str() {
        "status"=>serde_json::to_value(app.status().await)?,
        "add-repo"=>serde_json::to_value(app.repositories.add(&app.store,&args.next().context("Expected repository URL")?).await?)?,
        "repositories"=>serde_json::to_value(app.store.repositories()?)?,
        "install"=>serde_json::to_value(app.install(&args.next().context("Expected repository URL")?,&args.next().context("Expected extension internalName")?).await?)?,
        "sample"=>serde_json::to_value(app.install_sample().await?)?,
        "providers"=>serde_json::to_value(app.providers().await?)?,
        "states"=>app.extension_states().await?,
        "home"=>app.call("getMainPage",json!({"providerId":args.next().context("Expected provider ID")?,"page":1})).await?,
        "search"=>app.call("search",json!({"providerId":args.next().context("Expected provider ID")?,"query":args.next().context("Expected search query")?,"page":1})).await?,
        "load"=>app.call("load",json!({"providerId":args.next().context("Expected provider ID")?,"url":args.next().context("Expected item URL")?})).await?,
        "links"=>app.call("loadLinks",json!({"providerId":args.next().context("Expected provider ID")?,"data":args.next().context("Expected opaque episode data")?})).await?,
        "smoke"=>{
            app.install_sample().await?;
            let providers=app.providers().await?;
            let provider=providers.first().context("Sample did not register a provider")?;
            let results=app.call("search",json!({"providerId":provider.id,"query":"Bunny","page":1})).await?;
            let item:MediaItem=serde_json::from_value(results["items"][0].clone())?;
            let details:MediaDetails=serde_json::from_value(app.call("load",json!({"providerId":provider.id,"url":item.url})).await?)?;
            let episode=details.episodes.first().context("Sample did not return an episode")?;
            let links:ResolvedLinks=serde_json::from_value(app.call("loadLinks",json!({"providerId":provider.id,"data":episode.data})).await?)?;
            if links.streams.is_empty() { bail!("Sample did not resolve a stream") }
            json!({"ok":true,"provider":provider,"details":details,"links":links})
        }
        _=>{println!("Cloudstream Desktop CLI\n\nstatus | add-repo URL | repositories | install REPO INTERNAL_NAME | sample | providers | home PROVIDER | search PROVIDER QUERY | load PROVIDER URL | links PROVIDER DATA | smoke\n\nUse CLOUDSTREAM_DATA_DIR and CLOUDSTREAM_RESOURCE_DIR to override local paths.");return Ok(())}
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

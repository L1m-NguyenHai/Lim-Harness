use crate::runtime::Runtime;
use chrono::{DateTime,Utc};
use futures_util::{stream,StreamExt};
use regex::Regex;
use serde::{Deserialize,Serialize};
use serde_json::Value;
use sha2::{Digest,Sha256};
use std::{collections::{HashMap,HashSet},sync::Arc,time::Duration};
use tauri::State;

// v2 adds Hugging Face entries; older Ollama-only caches are ignored and refetched.
const CACHE_KEY:&str="model_catalog_v2";
static REFRESH:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct CatalogModel{
    id:String,name:String,download_bytes:u64,
    #[serde(default,skip_serializing_if="Option::is_none")] weight_bytes:Option<u64>,
    quantization:String,
    #[serde(default,skip_serializing_if="Option::is_none")] parameter_size:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")] parameter_count:Option<f64>,
    #[serde(default,skip_serializing_if="Option::is_none")] max_context:Option<u64>,
    #[serde(default)] capabilities:Vec<String>,
    description:String,source:String,verified_at:String,
    #[serde(default,skip_serializing_if="Option::is_none")] digest:Option<String>,
    metadata_status:String,
    #[serde(default,skip_serializing_if="Option::is_none")] provider:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")] downloads:Option<u64>,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct CatalogSnapshot{models:Vec<CatalogModel>,updated_at:String,source:String,#[serde(default)]warnings:Vec<String>}
#[derive(Clone)]struct Tag{family:String,tag:String,context:Option<u64>,capabilities:Vec<String>}

fn bundled()->Result<CatalogSnapshot,String>{
    let mut value:CatalogSnapshot=serde_json::from_str(include_str!("../../src/data/model-catalog.json")).map_err(|_|"Danh mục đi kèm không hợp lệ")?;
    value.source="bundled".into();Ok(value)
}
fn good(snapshot:&CatalogSnapshot)->bool{snapshot.models.len()>=12 && snapshot.models.len()<=1000 && snapshot.models.iter().all(|model|model.download_bytes>0 && model.id.len()<=200 && (model.source.starts_with("https://ollama.com/library/") || model.source.starts_with("https://huggingface.co/")))}
fn fresh(snapshot:&CatalogSnapshot)->bool{DateTime::parse_from_rfc3339(&snapshot.updated_at).is_ok_and(|date|{let age=Utc::now().signed_duration_since(date);age.num_seconds()>=0 && age.num_hours()<24})}
#[tauri::command]
pub async fn get_model_catalog(runtime:State<'_,Arc<Runtime>>,refresh:bool)->Result<CatalogSnapshot,String>{
    let _lock=REFRESH.lock().await;
    let raw=runtime.store.setting(CACHE_KEY)?;
    let cached=serde_json::from_str::<CatalogSnapshot>(&raw).ok().filter(good);
    if !refresh{
        if let Some(mut cached)=cached.clone(){if fresh(&cached){cached.source="cache".into();return Ok(cached)}}
        else if let Ok(seed)=bundled(){if good(&seed) && fresh(&seed){let _=runtime.store.set_setting(CACHE_KEY,&serde_json::to_string(&seed).map_err(|e|e.to_string())?);return Ok(seed)}}
    }
    let fetched=tokio::time::timeout(Duration::from_secs(90),fetch_catalog()).await;
    let failure=match fetched{
        Ok(Ok(mut result)) if good(&result)=>{
            if cached.as_ref().is_some_and(|old|result.models.len()<old.models.len()/2){"Danh mục online trả quá ít model; giữ cache đầy đủ trước đó".to_string()}
            else{match runtime.store.set_setting(CACHE_KEY,&serde_json::to_string(&result).map_err(|e|e.to_string())?){Ok(())=>{},Err(_)=>result.warnings.push("Danh mục online đã đọc nhưng chưa lưu được cache SQLite".into())};return Ok(result)}
        },
        Ok(Ok(_))=>"Nguồn online không trả đủ metadata được xác nhận; giữ danh mục trước đó".into(),
        Ok(Err(error))=>error,
        Err(_)=>"Đọc danh mục online vượt 90 giây; dùng dữ liệu trước đó".into(),
    };
    let mut fallback=if let Some(mut cached)=cached{cached.source="cache".into();cached}else{bundled()?};
    fallback.warnings.push(failure);Ok(fallback)
}

async fn fetch_bytes(client:&reqwest::Client,url:&str,cap:usize)->Result<(Vec<u8>,Option<String>),String>{
    let parsed=reqwest::Url::parse(url).map_err(|_|"URL metadata không hợp lệ")?;
    if parsed.scheme()!="https" || !["ollama.com","registry.ollama.ai","huggingface.co"].contains(&parsed.host_str().unwrap_or_default()){return Err("Nguồn metadata ngoài máy chủ Ollama/Hugging Face chính thức".into())}
    let mut response=client.get(parsed.clone()).send().await.map_err(|_|"Không kết nối được nguồn metadata Ollama/Hugging Face")?;
    if response.status().is_redirection() && parsed.host_str()==Some("registry.ollama.ai") && cap<=64_000 {
        let hash=parsed.path().split("/blobs/sha256:").nth(1).filter(|hash|hash.len()==64 && hash.bytes().all(|byte|byte.is_ascii_hexdigit())).ok_or("Redirect không phải config metadata")?;
        let location=response.headers().get("location").and_then(|value|value.to_str().ok()).ok_or("Metadata redirect thiếu location")?;
        let target=reqwest::Url::parse(location).map_err(|_|"Metadata redirect không hợp lệ")?;
        let expected=format!("/ollama/docker/registry/v2/blobs/sha256/{}/{hash}/data",&hash[..2]);
        if target.scheme()!="https" || target.host_str()!=Some("dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com") || target.path()!=expected {return Err("CDN config ngoài nguồn đã xác nhận; giữ cache trước đó".into())}
        response=client.get(target).send().await.map_err(|_|"Không đọc được config metadata trên CDN Ollama")?;
    }
    if !response.status().is_success(){return Err(format!("Nguồn metadata trả HTTP {}",response.status()))}
    if response.content_length().is_some_and(|length|length>cap as u64){return Err("Metadata vượt giới hạn dung lượng".into())}
    let digest=response.headers().get("docker-content-digest").and_then(|value|value.to_str().ok()).map(str::to_string);
    let mut stream=response.bytes_stream();let mut bytes=Vec::new();
    while let Some(chunk)=stream.next().await{let chunk=chunk.map_err(|_|"Đọc metadata bị gián đoạn")?;if bytes.len()+chunk.len()>cap{return Err("Metadata vượt giới hạn dung lượng".into())}bytes.extend_from_slice(&chunk)}
    Ok((bytes,digest))
}
fn plain(html:&str)->String{Regex::new(r"<[^>]+>").unwrap().replace_all(html," ").to_string().replace("&nbsp;"," ").replace("&amp;","&")}
fn part(value:&str)->bool{!value.is_empty() && value.len()<=80 && value.bytes().all(|b|b.is_ascii_alphanumeric() || b"-_ .".contains(&b)) && !value.contains(' ')}
async fn family_tags(client:&reqwest::Client,family:String)->Result<Vec<Tag>,String>{
    let (bytes,_)=fetch_bytes(client,&format!("https://ollama.com/library/{family}/tags"),2_000_000).await?;
    let html=String::from_utf8(bytes).map_err(|_|"Trang tags không phải UTF-8")?;
    let links=Regex::new(&format!(r#"href="/library/{}:([^"/]+)""#,regex::escape(&family))).unwrap();
    let context=Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*([km]?)\s*context").unwrap();
    let mut tags=Vec::new();let mut seen=HashSet::new();
    for matched in links.captures_iter(&html){
        let tag=matched[1].to_string();
        if !part(&tag) || tag.contains("cloud") || !seen.insert(tag.clone()){continue}
        // Only size/default variants whose links really appeared on the tags page.
        let simple=tag=="latest" || (tag.ends_with('b') || tag.ends_with('m')) && tag.bytes().all(|b|b.is_ascii_digit() || b".bmex".contains(&b));
        if !simple{continue}
        let end=matched.get(0).unwrap().end();
        let tail=&html[end..];let mut close=tail.find("</a>").unwrap_or(tail.len()).min(3000);while !tail.is_char_boundary(close){close-=1;}let nearby=plain(&tail[..close]);
        let max_context=context.captures(&nearby).and_then(|caps|{let value:f64=caps[1].parse().ok()?;let multiplier=match caps[2].to_lowercase().as_str(){"k"=>1024.,"m"=>1_048_576.,_=>1.};Some((value*multiplier)as u64)});
        let words:HashSet<String>=nearby.split_whitespace().map(|word|word.to_lowercase()).collect();
        let capabilities=["tools","thinking","vision","embedding","audio"].iter().filter(|capability|words.contains(**capability)).map(|s|s.to_string()).collect();
        tags.push(Tag{family:family.clone(),tag,context:max_context,capabilities});
        if tags.len()>=6{break}
    }
    tags.sort_by_key(|tag|tag.tag=="latest");Ok(tags)
}
fn parameter_count(value:&str)->Option<f64>{let re=Regex::new(r"(?i)^([0-9]+(?:\.[0-9]+)?)\s*([bm])$").unwrap();let captures=re.captures(value)?;let number:f64=captures[1].parse().ok()?;Some(number*if captures[2].eq_ignore_ascii_case("b"){1e9}else{1e6})}
async fn tag_metadata(client:&reqwest::Client,tag:Tag)->Result<CatalogModel,String>{
    let base=format!("https://registry.ollama.ai/v2/library/{}",tag.family);
    let (bytes,digest)=fetch_bytes(client,&format!("{base}/manifests/{}",tag.tag),64_000).await?;
    let manifest:Value=serde_json::from_slice(&bytes).map_err(|_|"Manifest model không hợp lệ")?;
    let layers=manifest["layers"].as_array().ok_or("Manifest thiếu layers")?;
    let mut download=manifest["config"]["size"].as_u64().ok_or("Manifest thiếu kích thước config")?;
    let mut weights=0u64;
    for layer in layers{let size=layer["size"].as_u64().ok_or("Manifest thiếu kích thước layer")?;download=download.checked_add(size).ok_or("Kích thước model quá lớn")?;if layer["mediaType"].as_str().is_some_and(|media|media.ends_with(".model") || media.ends_with(".projector")){weights=weights.checked_add(size).ok_or("Kích thước weights quá lớn")?;}}
    if weights==0{return Err("Manifest không có layer weights model local".into())}
    let config_digest=manifest["config"]["digest"].as_str().ok_or("Manifest thiếu config digest")?;
    if manifest["config"]["size"].as_u64().is_none_or(|size|size>64_000) || !Regex::new(r"^sha256:[0-9a-f]{64}$").unwrap().is_match(config_digest){return Err("Config model ngoài giới hạn metadata".into())}
    // This is the small configuration blob only. Weight/projector blobs are never requested.
    let (config,_)=fetch_bytes(client,&format!("{base}/blobs/{config_digest}"),64_000).await?;
    if format!("sha256:{:x}",Sha256::digest(&config))!=config_digest{return Err("Config metadata không khớp digest".into())}
    let config:Value=serde_json::from_slice(&config).map_err(|_|"Config model không hợp lệ")?;
    let parameter_size=config["model_type"].as_str().filter(|v|!v.is_empty()).map(str::to_string);
    let quantization=config["file_type"].as_str().filter(|v|!v.is_empty()).unwrap_or("Unknown").to_string();
    let verified=parameter_size.is_some() && quantization!="Unknown" && tag.context.is_some();
    let id=format!("{}:{}",tag.family,tag.tag);
    let source=format!("https://ollama.com/library/{id}");
    let mut capabilities=tag.capabilities;
    if let Ok((page,_))=fetch_bytes(client,&source,2_000_000).await {
        let page=String::from_utf8_lossy(&page);
        for capability in ["tools","thinking","vision","embedding","audio"] {
            let pattern=Regex::new(&format!(r"(?i)>\s*{capability}\s*<")).unwrap();
            if pattern.is_match(&page) && !capabilities.iter().any(|old|old==capability){capabilities.push(capability.to_string());}
        }
    }
    let digest=digest.or_else(||Some(format!("sha256:{:x}",Sha256::digest(&bytes))));
    Ok(CatalogModel{id:id.clone(),name:id.clone(),download_bytes:download,weight_bytes:Some(weights),quantization,parameter_count:parameter_size.as_deref().and_then(parameter_count),parameter_size,max_context:tag.context,capabilities,description:format!("Tag thực {id} trong thư viện Ollama."),source,verified_at:Utc::now().to_rfc3339(),digest,metadata_status:if verified{"verified"}else{"partial"}.into(),provider:Some("ollama".into()),downloads:None})
}
fn client()->Result<reqwest::Client,String>{reqwest::Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(8)).connect_timeout(Duration::from_secs(3)).user_agent("LimHarness/0.1 catalog-metadata").build().map_err(|e|e.to_string())}
fn family_links(html:&str,limit:usize)->Vec<String>{
    let links=Regex::new(r#"href="/library/([a-zA-Z0-9._-]+)""#).unwrap();
    let mut seen=HashSet::new();links.captures_iter(html).map(|m|m[1].to_string()).filter(|family|part(family) && seen.insert(family.clone())).take(limit).collect()
}
/// Ollama families from the library page (popular) or the search page (query), then real tags per family.
async fn fetch_ollama(client:&reqwest::Client,query:Option<&str>,families:usize,max_tags:usize)->Result<(Vec<CatalogModel>,usize),String>{
    let url=match query{Some(query)=>reqwest::Url::parse_with_params("https://ollama.com/search",&[("q",query)]).map_err(|_|"Từ khóa tìm kiếm không hợp lệ")?.to_string(),None=>"https://ollama.com/library".into()};
    let (bytes,_)=fetch_bytes(client,&url,2_000_000).await?;
    let html=String::from_utf8(bytes).map_err(|_|"Thư viện Ollama không phải UTF-8")?;
    let families=family_links(&html,families);
    if families.is_empty(){return if query.is_some(){Ok((Vec::new(),0))}else{Err("Trang thư viện Ollama thay đổi cấu trúc; chưa đọc được danh sách family".into())}}
    let groups:Vec<_>=stream::iter(families).map(|family|{let client=client.clone();async move{family_tags(&client,family).await}}).buffer_unordered(4).collect().await;
    let mut failures=0usize;let mut tags=Vec::new();for group in groups{match group{Ok(group)=>tags.extend(group),Err(_)=>failures+=1}}
    tags.truncate(max_tags);
    let results:Vec<_>=stream::iter(tags).map(|tag|{let client=client.clone();async move{tag_metadata(&client,tag).await}}).buffer_unordered(4).collect().await;
    let mut models=Vec::new();for result in results{match result{Ok(model)=>models.push(model),Err(_)=>failures+=1}}
    let mut digests:HashMap<String,CatalogModel>=HashMap::new();for model in models{let key=model.digest.clone().unwrap_or_else(||model.id.clone());match digests.get(&key){Some(old) if !old.id.ends_with(":latest")=>{},_=>{digests.insert(key,model);}}}
    Ok((digests.into_values().collect(),failures))
}

const HF_QUANT_ORDER:[&str;6]=["Q4_K_M","Q4_K_S","Q5_K_M","Q4_0","IQ4_XS","Q8_0"];
fn hf_quant(path:&str)->Option<String>{
    if path.contains('/') || path.to_lowercase().contains("mmproj") || Regex::new(r"(?i)-\d{5}-of-\d{5}\.gguf$").unwrap().is_match(path){return None}
    Regex::new(r"(?i)[-_.]((?:I?Q[1-8](?:_[A-Z0-9]+)*)|BF16|F16|F32)\.gguf$").unwrap().captures(path).map(|caps|caps[1].to_uppercase())
}
/// One GGUF repo on Hugging Face, pulled by Ollama as `hf.co/<repo>:<quant>`.
/// Only the public model API and the repo file listing are read; no weights are requested.
async fn hf_repo(client:&reqwest::Client,repo:Value)->Result<Vec<CatalogModel>,String>{
    let id=repo["id"].as_str().ok_or("Repo Hugging Face thiếu id")?.to_string();
    let valid=id.split('/').count()==2 && id.split('/').all(|segment|!segment.is_empty() && segment.len()<=96 && segment.bytes().all(|b|b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
    if !valid{return Err("Tên repo Hugging Face không hợp lệ".into())}
    let (bytes,_)=fetch_bytes(client,&format!("https://huggingface.co/api/models/{id}/tree/main"),2_000_000).await?;
    let files:Vec<Value>=serde_json::from_slice(&bytes).map_err(|_|"Danh sách tệp Hugging Face không hợp lệ")?;
    let mmproj:Option<u64>=files.iter().filter(|file|file["path"].as_str().is_some_and(|path|!path.contains('/') && path.to_lowercase().contains("mmproj") && path.ends_with(".gguf"))).filter_map(|file|file["size"].as_u64()).min();
    let mut quants:Vec<(String,u64)>=files.iter().filter_map(|file|Some((hf_quant(file["path"].as_str()?)?,file["size"].as_u64().filter(|size|*size>0)?))).collect();
    quants.sort_by_key(|(quant,_)|HF_QUANT_ORDER.iter().position(|item|item==quant).unwrap_or(usize::MAX));
    quants.dedup_by(|a,b|a.0==b.0);
    // Default quant (Q4_K_M or the closest available) plus Q8_0 when present.
    let mut chosen:Vec<(String,u64)>=quants.iter().take(1).cloned().collect();
    if let Some(q8)=quants.iter().find(|(quant,_)|quant=="Q8_0"){if !chosen.iter().any(|(quant,_)|quant=="Q8_0"){chosen.push(q8.clone())}}
    let gguf=&repo["gguf"];
    let parameters=gguf["total"].as_f64().filter(|value|*value>0.);
    let context=gguf["context_length"].as_u64().filter(|value|*value>0);
    let pipeline=repo["pipeline_tag"].as_str().unwrap_or_default();
    let mut capabilities=Vec::new();
    if mmproj.is_some() || pipeline=="image-text-to-text"{capabilities.push("vision".to_string())}
    if ["feature-extraction","sentence-similarity"].contains(&pipeline){capabilities.push("embedding".to_string())}
    let downloads=repo["downloads"].as_u64();
    let architecture=gguf["architecture"].as_str().unwrap_or("chưa rõ");
    let extra=mmproj.unwrap_or(0);
    Ok(chosen.into_iter().map(|(quant,size)|{
        let tag=format!("hf.co/{id}:{quant}");
        CatalogModel{id:tag.clone(),name:tag,download_bytes:size+extra,weight_bytes:Some(size+extra),quantization:quant,
            parameter_size:parameters.map(|count|if count>=1e9{format!("{:.1}B",count/1e9)}else{format!("{:.0}M",count/1e6)}),parameter_count:parameters,max_context:context,capabilities:capabilities.clone(),
            description:format!("GGUF trên Hugging Face ({id}), kiến trúc {architecture}; Ollama tải trực tiếp qua hf.co."),source:format!("https://huggingface.co/{id}"),verified_at:Utc::now().to_rfc3339(),digest:None,
            metadata_status:if parameters.is_some() && context.is_some(){"verified"}else{"partial"}.into(),provider:Some("huggingface".into()),downloads}
    }).collect())
}
async fn fetch_huggingface(client:&reqwest::Client,query:Option<&str>,limit:usize)->Result<(Vec<CatalogModel>,usize),String>{
    let limit=limit.to_string();
    let mut params=vec![("filter","gguf"),("sort","downloads"),("direction","-1"),("limit",limit.as_str()),("expand[]","gguf"),("expand[]","downloads"),("expand[]","pipeline_tag"),("expand[]","gated"),("expand[]","private")];
    if let Some(query)=query{params.push(("search",query))}
    let url=reqwest::Url::parse_with_params("https://huggingface.co/api/models",&params).map_err(|_|"Từ khóa tìm kiếm không hợp lệ")?;
    // The list embeds GGUF chat templates, so allow a larger but still bounded response.
    let (bytes,_)=fetch_bytes(client,url.as_str(),8_000_000).await?;
    let repos:Vec<Value>=serde_json::from_slice(&bytes).map_err(|_|"API Hugging Face trả dữ liệu không hợp lệ")?;
    // Gated/private repos need a token that Ollama does not have here; skip them.
    let repos:Vec<Value>=repos.into_iter().filter(|repo|repo["gated"].as_bool()!=Some(true) && repo["gated"].as_str().is_none() && repo["private"].as_bool()!=Some(true)).collect();
    let results:Vec<_>=stream::iter(repos).map(|repo|{let client=client.clone();async move{hf_repo(&client,repo).await}}).buffer_unordered(4).collect().await;
    let mut failures=0usize;let mut models=Vec::new();for result in results{match result{Ok(group)=>models.extend(group),Err(_)=>failures+=1}}
    Ok((models,failures))
}
type Fetched=Result<(Vec<CatalogModel>,usize),String>;
fn merge(ollama:Fetched,hf:Fetched,mut warnings:Vec<String>,source:&str)->Result<CatalogSnapshot,String>{
    let mut models=Vec::new();let mut failures=0usize;let mut errors=Vec::new();
    match ollama{Ok((list,failed))=>{models.extend(list);failures+=failed},Err(error)=>errors.push(format!("Ollama: {error}"))}
    match hf{Ok((list,failed))=>{models.extend(list);failures+=failed},Err(error)=>errors.push(format!("Hugging Face: {error}"))}
    if models.is_empty() && !errors.is_empty(){return Err(errors.join(" · "))}
    warnings.extend(errors);
    models.sort_by(|a,b|a.id.cmp(&b.id));
    if failures>0{warnings.push(format!("Không đọc được metadata của {failures} family/tag/repo; các mục đó đã bỏ qua."))}
    if models.iter().any(|model|model.metadata_status=="partial"){warnings.push("Một số mục thiếu context/quantization/parameter metadata và được ghi partial; không tự suy từ tên.".into())}
    Ok(CatalogSnapshot{models,updated_at:Utc::now().to_rfc3339(),source:source.into(),warnings})
}
async fn fetch_catalog()->Result<CatalogSnapshot,String>{
    let client=client()?;
    let (ollama,hf)=futures_util::future::join(fetch_ollama(&client,None,20,80),fetch_huggingface(&client,None,24)).await;
    merge(ollama,hf,vec!["Danh mục gồm tối đa 80 tag của 20 family Ollama Library và 24 repo GGUF nhiều lượt tải nhất trên Hugging Face; dùng Tìm online để xem thêm.".into()],"online")
}

/// Live search across Ollama Library and Hugging Face GGUF repos. Results are not cached.
#[tauri::command]
pub async fn search_model_catalog(query:String,provider:String)->Result<CatalogSnapshot,String>{
    let query=query.trim().to_string();
    if query.is_empty() || query.chars().count()>80 || query.chars().any(char::is_control){return Err("Nhập từ khóa 1–80 ký tự để tìm model.".into())}
    let client=client()?;
    let ollama=async{if provider=="huggingface"{Ok((Vec::new(),0))}else{fetch_ollama(&client,Some(&query),6,24).await}};
    let hf=async{if provider=="ollama"{Ok((Vec::new(),0))}else{fetch_huggingface(&client,Some(&query),16).await}};
    match tokio::time::timeout(Duration::from_secs(45),futures_util::future::join(ollama,hf)).await{
        Ok((ollama,hf))=>merge(ollama,hf,Vec::new(),"search"),
        Err(_)=>Err("Tìm online vượt 45 giây; thử từ khóa cụ thể hơn.".into()),
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn reads_hf_quant_from_file_name(){
        assert_eq!(hf_quant("Qwen3-4B-Q4_K_M.gguf").as_deref(),Some("Q4_K_M"));
        assert_eq!(hf_quant("gemma-3-4b-it-UD-IQ4_XS.gguf").as_deref(),Some("IQ4_XS"));
        assert_eq!(hf_quant("model.BF16.gguf").as_deref(),Some("BF16"));
        assert_eq!(hf_quant("mmproj-model-f16.gguf"),None);
        assert_eq!(hf_quant("big-Q4_K_M-00001-of-00003.gguf"),None);
        assert_eq!(hf_quant("BF16/model-BF16.gguf"),None);
    }
    /// Network check: `cargo test live_sources -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_sources(){
        tauri::async_runtime::block_on(async{
            let client=client().unwrap();
            let (hf,failed)=fetch_huggingface(&client,Some("qwen3"),4).await.unwrap();
            println!("hf {} models, {failed} failed: {:?}",hf.len(),hf.iter().map(|m|(&m.id,m.download_bytes,&m.parameter_size,m.max_context)).collect::<Vec<_>>());
            assert!(!hf.is_empty());
            let (ollama,failed)=fetch_ollama(&client,Some("gemma"),2,4).await.unwrap();
            println!("ollama {} models, {failed} failed: {:?}",ollama.len(),ollama.iter().map(|m|&m.id).collect::<Vec<_>>());
            assert!(!ollama.is_empty());
            let started=std::time::Instant::now();
            let full=fetch_catalog().await.unwrap();
            let hf=full.models.iter().filter(|m|m.provider.as_deref()==Some("huggingface")).count();
            println!("full {} models ({hf} hf) in {:?}; warnings {:?}",full.models.len(),started.elapsed(),full.warnings);
            assert!(good(&full));
        });
    }
}

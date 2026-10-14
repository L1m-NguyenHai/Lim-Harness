use crate::runtime::Runtime;
use chrono::{DateTime,Utc};
use futures_util::{stream,StreamExt};
use regex::Regex;
use serde::{Deserialize,Serialize};
use serde_json::Value;
use sha2::{Digest,Sha256};
use std::{collections::{HashMap,HashSet},sync::Arc,time::Duration};
use tauri::State;

const CACHE_KEY:&str="model_catalog";
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
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct CatalogSnapshot{models:Vec<CatalogModel>,updated_at:String,source:String,#[serde(default)]warnings:Vec<String>}
#[derive(Clone)]struct Tag{family:String,tag:String,context:Option<u64>,capabilities:Vec<String>}

fn bundled()->Result<CatalogSnapshot,String>{
    let mut value:CatalogSnapshot=serde_json::from_str(include_str!("../../src/data/model-catalog.json")).map_err(|_|"Danh mục đi kèm không hợp lệ")?;
    value.source="bundled".into();Ok(value)
}
fn good(snapshot:&CatalogSnapshot)->bool{snapshot.models.len()>=12 && snapshot.models.len()<=1000 && snapshot.models.iter().all(|model|model.download_bytes>0 && model.id.len()<=200 && model.source.starts_with("https://ollama.com/library/"))}
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
    if parsed.scheme()!="https" || !["ollama.com","registry.ollama.ai"].contains(&parsed.host_str().unwrap_or_default()){return Err("Nguồn metadata ngoài máy chủ Ollama chính thức".into())}
    let mut response=client.get(parsed.clone()).send().await.map_err(|_|"Không kết nối được nguồn metadata Ollama chính thức")?;
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
    Ok(CatalogModel{id:id.clone(),name:id.clone(),download_bytes:download,weight_bytes:Some(weights),quantization,parameter_count:parameter_size.as_deref().and_then(parameter_count),parameter_size,max_context:tag.context,capabilities,description:format!("Tag thực {id} trong thư viện Ollama."),source,verified_at:Utc::now().to_rfc3339(),digest,metadata_status:if verified{"verified"}else{"partial"}.into()})
}
async fn fetch_catalog()->Result<CatalogSnapshot,String>{
    let client=reqwest::Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(8)).connect_timeout(Duration::from_secs(3)).user_agent("LimHarness/0.1 catalog-metadata").build().map_err(|e|e.to_string())?;
    let (bytes,_)=fetch_bytes(&client,"https://ollama.com/library",2_000_000).await?;
    let html=String::from_utf8(bytes).map_err(|_|"Thư viện Ollama không phải UTF-8")?;
    let links=Regex::new(r#"href="/library/([a-zA-Z0-9._-]+)""#).unwrap();
    let mut seen=HashSet::new();let families:Vec<String>=links.captures_iter(&html).map(|m|m[1].to_string()).filter(|family|part(family) && seen.insert(family.clone())).take(20).collect();
    if families.is_empty(){return Err("Trang thư viện Ollama thay đổi cấu trúc; chưa đọc được danh sách family".into())}
    let groups:Vec<_>=stream::iter(families).map(|family|{let client=client.clone();async move{family_tags(&client,family).await}}).buffer_unordered(4).collect().await;
    let mut failures=0usize;let mut tags=Vec::new();for group in groups{match group{Ok(group)=>tags.extend(group),Err(_)=>failures+=1}}
    tags.truncate(80);
    let results:Vec<_>=stream::iter(tags).map(|tag|{let client=client.clone();async move{tag_metadata(&client,tag).await}}).buffer_unordered(4).collect().await;
    let mut models=Vec::new();for result in results{match result{Ok(model)=>models.push(model),Err(_)=>failures+=1}}
    let mut digests:HashMap<String,CatalogModel>=HashMap::new();for model in models{let key=model.digest.clone().unwrap_or_else(||model.id.clone());match digests.get(&key){Some(old) if !old.id.ends_with(":latest")=>{},_=>{digests.insert(key,model);}}}
    let mut models:Vec<CatalogModel>=digests.into_values().collect();models.sort_by(|a,b|a.id.cmp(&b.id));
    let mut warnings=vec!["Danh mục giới hạn 20 family / 80 tag; metadata registry và HTML có thể thay đổi. Không phải toàn bộ Ollama Library.".into()];
    if failures>0{warnings.push(format!("Không đọc được metadata của {failures} family/tag; các mục đó đã bỏ qua."))}
    if models.iter().any(|model|model.metadata_status=="partial"){warnings.push("Một số tag thiếu context/quantization/parameter metadata và được ghi partial; không tự suy từ tên.".into())}
    Ok(CatalogSnapshot{models,updated_at:Utc::now().to_rfc3339(),source:"online".into(),warnings})
}

//! Direct, on-device adapters for the regular providers selected by Zenime Web.
//! AniList stays the canonical metadata source. These adapters only resolve anime
//! episodes and manga chapters/pages; no Zenime-hosted metadata endpoint is required.
use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use scraper::{Html, Selector};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

static NULL_VALUE: Value = Value::Null;

const UA: &str = "Mozilla/5.0 (Linux; Android 10) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36";
const ANIME_PRIORITY: &[&str] = &[
    "anikoto", "animeparadise", "4animo", "reanime", "kickassanime",
    "animepahe", "xanime", "anipm", "anidb",
];
const MANGA_PRIORITY: &[&str] = &["mangahere", "atsumaru", "mangapill", "mangakatana"];

#[derive(Clone, Debug)]
pub struct ProviderRow {
    pub id: String,
    pub number: String,
    pub title: String,
    pub provider: String,
}

pub fn provider_names() -> &'static [&'static str] { ANIME_PRIORITY }
pub fn unpack_episode_id(id: &str) -> Option<String> {
    let (_, _, encoded) = unpack(id, "zn")?;
    Some(split_ep(encoded).0)
}

pub fn manga_provider_names() -> &'static [&'static str] { MANGA_PRIORITY }
pub fn manga_referer(packed_id: &str) -> &'static str {
    match unpack(packed_id, "znm").map(|x| x.0).unwrap_or("") {
        "atsumaru" => "https://atsu.moe/",
        "mangapill" => "https://mangapill.com/",
        "mangakatana" => "https://mangakatana.com/",
        _ => "https://mangahere.cc/",
    }
}

fn response(client: &Client, url: &str, referer: &str) -> Result<Response, String> {
    client.get(url)
        .header(USER_AGENT, UA)
        .header(ACCEPT, "application/json, text/html, */*")
        .header(REFERER, referer)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("Request failed for {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Provider rejected {url}: {e}"))
}
fn get_text(client: &Client, url: &str, referer: &str) -> Result<String, String> {
    response(client, url, referer)?.text().map_err(|e| format!("Could not read provider response: {e}"))
}
fn get_json(client: &Client, url: &str, referer: &str) -> Result<Value, String> {
    let body = get_text(client, url, referer)?;
    serde_json::from_str(&body).map_err(|e| format!("Provider returned non-JSON data at {url}: {e}"))
}
fn post_json(client: &Client, url: &str, referer: &str, body: &Value) -> Result<Value, String> {
    let value = client.post(url).header(USER_AGENT, UA).header(ACCEPT, "application/json, text/plain, */*")
        .header(REFERER, referer).header(CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(10)).json(body).send()
        .map_err(|e| format!("Provider request failed: {e}"))?
        .error_for_status().map_err(|e| format!("Provider rejected request: {e}"))?;
    value.json::<Value>().map_err(|e| format!("Invalid provider JSON: {e}"))
}
fn post_text(client: &Client, url: &str, referer: &str, body: &str) -> Result<String, String> {
    client.post(url).header(USER_AGENT, UA).header(ACCEPT, "*/*")
        .header(REFERER, referer).header(ORIGIN, "https://www.animeparadise.moe")
        .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
        .header("Next-Action", "60b2d177f9d6893f2ace2dd05d5abb6fa1e6ca16bc")
        .timeout(Duration::from_secs(12)).body(body.to_string()).send()
        .map_err(|e| format!("AnimeParadise source request failed: {e}"))?
        .error_for_status().map_err(|e| format!("AnimeParadise source rejected: {e}"))?
        .text().map_err(|e| format!("Could not read AnimeParadise response: {e}"))
}
fn value_text(v: &Value) -> String {
    match v { Value::String(s) => s.trim().to_string(), Value::Number(n) => n.to_string(), _ => String::new() }
}
fn first<'a>(v: &'a Value, keys: &[&str]) -> &'a Value {
    for k in keys { if !v.get(*k).unwrap_or(&NULL_VALUE).is_null() { return v.get(*k).unwrap(); } }
    &NULL_VALUE
}
fn text_first(v: &Value, keys: &[&str]) -> String { value_text(first(v, keys)) }
fn arr<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    for key in keys { if let Some(a)=v.get(*key).and_then(Value::as_array) { return Some(a); } }
    for key in ["data","result","results","items"] {
        if let Some(sub)=v.get(key) {
            if let Some(a)=sub.as_array() { return Some(a); }
            for k in keys { if let Some(a)=sub.get(*k).and_then(Value::as_array) { return Some(a); } }
        }
    }
    None
}
fn title_of(v: &Value) -> String {
    let t = v.get("title").unwrap_or(&NULL_VALUE);
    if t.is_object() {
        let s=text_first(t,&["user_preferred","userPreferred","english","romaji","native"]);
        if !s.is_empty(){return s;}
    } else if let Some(s)=t.as_str() { return s.to_string(); }
    text_first(v,&["name","anime_title","title_en","info_title","englishTitle"])
}
fn normalize_title(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}
fn title_match(wanted: &str, candidate: &str) -> bool {
    let a=normalize_title(wanted); let b=normalize_title(candidate);
    !a.is_empty() && !b.is_empty() && (a==b || a.contains(&b) || b.contains(&a))
}
fn selector(s: &str) -> Option<Selector> { Selector::parse(s).ok() }
fn packed(kind: &str, provider: &str, anime: &str, episode: &str) -> String {
    format!("{kind}:{provider}:{anime}:{episode}")
}
fn unpack<'a>(id: &'a str, prefix: &str) -> Option<(&'a str,&'a str,&'a str)> {
    let mut parts=id.splitn(4, ':');
    if parts.next()? != prefix { return None; }
    Some((parts.next()?,parts.next()?,parts.next()?))
}
// Safer variant for rows where the caller has already selected the display title.
fn make_row(provider:&str, anime:&str, ep:&str, n:&str, title:&str)->ProviderRow {
    ProviderRow{id:packed("zn",provider,anime,&format!("{ep}~{n}")),number:n.to_string(),
        title:if title.trim().is_empty(){format!("Episode {n}")}else{title.to_string()},provider:provider.into()}
}
fn split_ep(value:&str)->(String,String) {
    if let Some((left,right))=value.rsplit_once('~') { (left.to_string(),right.to_string()) } else {(value.to_string(),String::new())}
}
fn html_links(body:&str, base:&str, selector_text:&str)->Vec<(String,String)> {
    let doc=Html::parse_document(body);
    let Some(sel)=selector(selector_text) else{return vec![]};
    doc.select(&sel).filter_map(|el| {
        let href=el.value().attr("href")?;
        if href.starts_with("javascript:") || href.starts_with('#') {return None;}
        let title=el.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        let url=if href.starts_with("http"){href.to_string()}else{format!("{}{}",base.trim_end_matches('/'),if href.starts_with('/'){href.to_string()}else{format!("/{href}")})};
        if title.is_empty(){None}else{Some((title,url))}
    }).collect()
}
fn direct_url(s: &str) -> bool {
    let l=s.to_ascii_lowercase();
    (l.starts_with("http://")||l.starts_with("https://")) &&
    [".m3u8",".mp4",".mpd",".m4v",".webm"].iter().any(|ext| l.split('?').next().unwrap_or(&l).ends_with(ext) || l.contains(&format!("{ext}?")))
}
fn collect_media(v:&Value,out:&mut Vec<(String,String)>) {
    match v {
        Value::String(s) => {
            if direct_url(s) && !out.iter().any(|(u,_)|u==s) { out.push((s.clone(),String::new())); }
        }
        Value::Array(items) => for item in items { collect_media(item,out); },
        Value::Object(map) => for (k,val) in map {
            if ["url","file","src","source","souPath","iframe","link","streamLink"].contains(&k.as_str()) {
                if let Some(s)=val.as_str(){ if direct_url(s)&&!out.iter().any(|(u,_)|u==s){out.push((s.to_string(),String::new()));} }
            }
            collect_media(val,out);
        },
        _=>{}
    }
}
fn direct_from_html(html:&str,base:&str)->Option<String> {
    let doc=Html::parse_document(html);
    for s in ["video source[src]","source[src]","video[src]","a[href*='.m3u8']","a[href*='.mp4']","iframe[src]"] {
        if let Some(sel)=selector(s) {
            for el in doc.select(&sel) {
                let attr=if s.starts_with("iframe") {"src"} else if s.starts_with("a[") {"href"} else if s=="video[src]" {"src"} else {"src"};
                if let Some(v)=el.value().attr(attr) {
                    let url=if v.starts_with("http"){v.to_string()}else{format!("{}{}",base.trim_end_matches('/'),if v.starts_with('/'){v.to_string()}else{format!("/{v}")})};
                    if direct_url(&url){return Some(url);}
                    if s.starts_with("iframe") && url.starts_with("http"){ if let Ok(page)=url::Url::parse(&url) {let _=page;} }
                }
            }
        }
    }
    // Player scripts sometimes contain escaped .m3u8 URLs rather than source tags.
    for marker in [".m3u8",".mp4",".mpd"] {
        let mut at=0;
        while let Some(pos)=html[at..].find(marker) {
            let end=at+pos+marker.len();
            let start=html[..at+pos].rfind(|c:char| c=='\''||c=='"'||c==' '||c=='='||c=='>').map(|x|x+1).unwrap_or(0);
            let found=html[start..end].replace("\\/","/");
            if direct_url(&found){return Some(found);}
            at=end;
            if at>=html.len(){break;}
        }
    }
    None
}
fn best_match<'a>(wanted:&str, items:&'a [Value])->Option<&'a Value> {
    items.iter().find(|item|title_match(wanted,&title_of(item)))
}
fn numbers(s:&str)->String {
    let mut got=String::new(); let mut dot=false;
    for c in s.chars(){if c.is_ascii_digit(){got.push(c)}else if c=='.'&&!dot&&!got.is_empty(){got.push(c);dot=true}else if !got.is_empty(){break;}}
    got.trim_end_matches('.').to_string()
}
fn maybe_direct(client:&Client,url:&str,referer:&str)->Result<(String,String),String>{
    if direct_url(url){return Ok((url.to_string(),referer.to_string()));}
    let body=get_text(client,url,referer)?;
    if let Some(media)=direct_from_html(&body,url){return Ok((media,referer.to_string()));}
    let doc=Html::parse_document(&body);
    for s in ["iframe[src]","video source[src]","source[src]"] {
        if let Some(sel)=selector(s) {
            for node in doc.select(&sel) {
                if let Some(next)=node.value().attr("src") {
                    let resolved=if next.starts_with("http"){next.to_string()}else{format!("{}{}",url.trim_end_matches('/'),next)};
                    if direct_url(&resolved){return Ok((resolved,referer.to_string()));}
                    if resolved.starts_with("http") {
                        if let Ok(text)=get_text(client,&resolved,url) {
                            if let Some(media)=direct_from_html(&text,&resolved){return Ok((media,referer.to_string()));}
                        }
                        // Preserve the provider iframe as a fallback for the Android WebView player.
                        return Ok((format!("zenime-embed://{}",urlencoding::encode(&resolved)),referer.to_string()));
                    }
                }
            }
        }
    }
    Err(format!("EMBED_URL:{url}"))
}

fn episodes_reanime(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let search=get_json(client,&format!("https://reanime.to/api/v1/search?q={}&limit=36&offset=0",urlencoding::encode(title)),"https://reanime.to/")?;
    let items=arr(&search,&["results"]).ok_or("ReAnime search returned no results")?;
    let anime=best_match(title,items).ok_or("ReAnime had no matching title")?;
    let key=text_first(anime,&["anime_id","id"]);
    if key.is_empty(){return Err("ReAnime did not provide an anime id".into());}
    let payload=get_json(client,&format!("https://reanime.to/api/v1/episodes/{}",urlencoding::encode(&key)),"https://reanime.to/")?;
    let episodes=arr(&payload,&["data","episodes","results"]).ok_or("ReAnime episode list was empty")?;
    Ok(episodes.iter().enumerate().map(|(i,e)|{
        let n=text_first(e,&["episode_number","number","episode"]).if_empty(&(i+1).to_string());
        let id=if key.is_empty(){n.clone()}else{n.clone()};
        make_row("reanime",&key,&id,&n,&text_first(e,&["title","name"]))
    }).collect())
}
trait IfEmpty { fn if_empty(self, fallback:&str)->String; }
impl IfEmpty for String {fn if_empty(self,fallback:&str)->String{if self.is_empty(){fallback.into()}else{self}}}

fn episodes_animeparadise(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let s=get_json(client,&format!("https://api.animeparadise.moe/search?q={}&page=1",urlencoding::encode(title)),"https://www.animeparadise.moe/")?;
    let items=arr(&s,&["data","results"]).ok_or("AnimeParadise search was empty")?;
    let hit=best_match(title,items).ok_or("AnimeParadise had no matching title")?;
    let slug=text_first(hit,&["slug","id"]);
    if slug.is_empty(){return Err("AnimeParadise search result lacks slug".into())}
    let detail=get_json(client,&format!("https://api.animeparadise.moe/anime/{}",urlencoding::encode(&slug)),"https://www.animeparadise.moe/")?;
    let data=detail.get("data").unwrap_or(&detail);
    let aid=text_first(data,&["_id","id"]).if_empty(&slug);
    let ep=get_json(client,&format!("https://api.animeparadise.moe/anime/{}/episode",urlencoding::encode(&aid)),"https://www.animeparadise.moe/")?;
    let rows=arr(&ep,&["data","episodes"]).ok_or("AnimeParadise returned no episodes")?;
    Ok(rows.iter().enumerate().map(|(i,e)|{
        let n=text_first(e,&["number","episode"]).if_empty(&(i+1).to_string());
        let uid=text_first(e,&["uid","_id","id"]).if_empty(&n);
        let origin=text_first(e,&["origin"]).if_empty(&aid);
        make_row("animeparadise",&slug,&format!("{uid}|{origin}|{n}"),&n,&text_first(e,&["title","name"]))
    }).collect())
}

fn episodes_anipm(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let s=get_json(client,&format!("https://ani.pm/api/anime/catalog?page=1&limit=24&q={}&sfw=1",urlencoding::encode(title)),"https://ani.pm/anime")?;
    let items=arr(&s,&["items","results"]).ok_or("AniPM catalog was empty")?;
    let hit=best_match(title,items).ok_or("AniPM had no matching title")?;
    let handle=text_first(hit,&["handle","slug","id","anilistId"]);
    if handle.is_empty(){return Err("AniPM search result lacks id".into())}
    let mut series=Value::Null;
    for kind in ["ani","series"] {
        if let Ok(v)=get_json(client,&format!("https://ani.pm/api/anime/{kind}/{}?routes=e4",urlencoding::encode(&handle)),"https://ani.pm/anime"){series=v;break;}
    }
    if series.is_null(){return Err("AniPM detail request failed".into())}
    let key=text_first(&series,&["handle","slug","id"]).if_empty(&handle);
    let eps=arr(&series,&["episodes"]).ok_or("AniPM returned no episodes")?;
    Ok(eps.iter().map(|e|{
        let n=text_first(e,&["number","episodeNumber"]).if_empty("1");
        let source_provider=text_first(&series,&["source"]).if_empty("settlar");
        let source_id=text_first(&series,&["id","sourceId"]);
        let route=text_first(e,&["routeId","sourceNumber","number"]).if_empty(&n);
        make_row("anipm",&key,&format!("{}|{}|{}|{}|{}",source_provider,source_id,route,n,if e.get("dub").and_then(Value::as_bool)==Some(true){"dub"}else{"sub"}),&n,&text_first(e,&["title"]))
    }).collect())
}

fn episodes_kickass(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let s=post_json(client,"https://kaa.lt/api/fsearch","https://kaa.lt/",&json!({"page":1,"query":title}))?;
    let items=arr(&s,&["result","results"]).ok_or("KickAssAnime search was empty")?;
    let hit=best_match(title,items).ok_or("KickAssAnime had no matching title")?;
    let slug=text_first(hit,&["slug","id"]);
    if slug.is_empty(){return Err("KickAssAnime result lacks slug".into())}
    let body=get_json(client,&format!("https://kaa.lt/api/show/{}/episodes?page=1&lang=ja-JP",urlencoding::encode(&slug)),"https://kaa.lt/")?;
    let eps=arr(&body,&["episodes","result","data"]).ok_or("KickAssAnime episode list was empty")?;
    Ok(eps.iter().enumerate().map(|(i,e)|{
        let n=text_first(e,&["episode","number","ep","episode_number"]).if_empty(&(i+1).to_string());
        let epath=text_first(e,&["id","slug","episode_slug"]).if_empty(&format!("episode/ep-{}-{}",n,slug));
        make_row("kickassanime",&slug,&epath,&n,&text_first(e,&["title","name"]))
    }).collect())
}

fn episodes_animepahe(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let body=get_json(client,&format!("https://animepahe.pw/api?m=search&q={}",urlencoding::encode(title)),"https://animepahe.pw/")?;
    let items=arr(&body,&["data","results"]).ok_or("AnimePahe search was empty")?;
    let hit=best_match(title,items).ok_or("AnimePahe had no matching title")?;
    let session=text_first(hit,&["session","id"]);
    if session.is_empty(){return Err("AnimePahe result lacks session".into())}
    let eps=get_json(client,&format!("https://animepahe.pw/api?m=release&id={}&sort=episode_asc&page=1",urlencoding::encode(&session)),"https://animepahe.pw/")?;
    let items=arr(&eps,&["data","results"]).ok_or("AnimePahe episode list was empty")?;
    Ok(items.iter().enumerate().map(|(i,e)|{
        let n=text_first(e,&["episode","number"]).if_empty(&(i+1).to_string());
        let es=text_first(e,&["session","id"]).if_empty(&n);
        make_row("animepahe",&session,&format!("{session}/{es}"),&n,&text_first(e,&["title"]))
    }).collect())
}

fn episodes_anikoto(client:&Client,title:&str,dub:bool)->Result<Vec<ProviderRow>,String>{
    let url=format!("https://anikototv.to/filter?keyword={}&page=1",urlencoding::encode(title));
    let html=get_text(client,&url,"https://anikototv.to/")?;
    let links=html_links(&html,"https://anikototv.to","a.name.d-title, a[href*='/watch/']");
    let (_,anime_url)=links.iter().find(|(t,_)|title_match(title,t)).or_else(||links.first()).ok_or("AniKoto search has no matches")?;
    let page=get_text(client,anime_url,"https://anikototv.to/")?;
    let doc=Html::parse_document(&page);
    let mut rows=Vec::new();
    for selector_text in ["a[data-ids]","a.ep-item","a[href*='ep=']",".ss-list a"] {
        if let Some(sel)=selector(selector_text) {
            for (i,node) in doc.select(&sel).enumerate(){
                let n=node.value().attr("data-number").map(numbers).unwrap_or_default().if_empty(&(i+1).to_string());
                let epid=node.value().attr("data-ids").or_else(||node.value().attr("data-id")).or_else(||node.value().attr("href")).unwrap_or("");
                if epid.is_empty(){continue;}
                let typ=node.value().attr("data-type").unwrap_or("");
                if dub && !typ.to_lowercase().contains("dub") && selector_text=="a[data-ids]" { continue; }
                rows.push(make_row("anikoto",anime_url,epid,&n,&node.text().collect::<Vec<_>>().join(" ").trim().to_string()));
            }
        }
        if !rows.is_empty(){break;}
    }
    if rows.is_empty(){return Err("AniKoto episode IDs were not present in the page response".into())}
    rows.sort_by(|a,b|a.number.parse::<f64>().unwrap_or(0.0).partial_cmp(&b.number.parse::<f64>().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    rows.dedup_by(|a,b|a.id==b.id);
    Ok(rows)
}

fn episodes_4animo(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let base="https://zanix.tr";
    let page=get_text(client,&format!("{base}/search?query={}",urlencoding::encode(title)),base)?;
    let links=html_links(&page,base,"a[href*='/watch/'], a[href*='?ep=']");
    let (_,watch)=links.iter().find(|(t,_)|title_match(title,t)).or_else(||links.first()).ok_or("4Animo search had no matching title")?;
    let watch=if watch.contains("/watch/"){watch.to_string()}else{watch.clone()};
    let html=get_text(client,&watch,base)?;
    let doc=Html::parse_document(&html);
    let mut rows=Vec::new();
    for sel_text in ["a[href*='?ep=']","a[data-episode]","a.ep-item","button[data-episode]"] {
        if let Some(sel)=selector(sel_text) {
            for (i,n) in doc.select(&sel).enumerate(){
                let href=n.value().attr("href").or_else(||n.value().attr("data-episode")).unwrap_or("");
                let text=n.text().collect::<Vec<_>>().join(" ");
                let num=n.value().attr("data-episode-number").map(numbers).unwrap_or_default().if_empty(&numbers(&text)).if_empty(&(i+1).to_string());
                if !href.is_empty(){rows.push(make_row("4animo",&watch,href,&num,&text.trim().to_string()))}
            }
        }
        if !rows.is_empty(){break;}
    }
    if rows.is_empty(){return Err("4Animo episode list could not be extracted".into())}
    Ok(rows)
}

fn xanime_payload(payload:&Value)->Result<String,String>{
    let mut iv=[0u8;12]; OsRng.fill_bytes(&mut iv);
    let digest=Sha256::digest(b"xanime-ph25-obfuscation-secret-key-2026");
    let cipher=Aes256Gcm::new_from_slice(&digest).map_err(|e|e.to_string())?;
    let encrypted=cipher.encrypt(Nonce::from_slice(&iv),payload.to_string().as_bytes()).map_err(|_|"XAnime request encryption failed".to_string())?;
    Ok(json!({"v":1,"iv":B64.encode(iv),"ct":B64.encode(encrypted)}).to_string())
}
fn xanime_query(client:&Client,query:&str,variables:Value)->Result<Value,String>{
    let encrypted=xanime_payload(&json!({"query":query,"variables":variables}))?;
    let body=client.post("https://xanime.me/z2/").header(USER_AGENT,UA).header(ACCEPT,"application/json")
        .header(CONTENT_TYPE,"application/json").header(ORIGIN,"https://xanime.me").header(REFERER,"https://xanime.me/")
        .timeout(Duration::from_secs(12)).body(encrypted).send().map_err(|e|e.to_string())?
        .error_for_status().map_err(|e|format!("XAnime request was rejected: {e}"))?
        .json::<Value>().map_err(|e|format!("XAnime response is not JSON: {e}"))?;
    let iv=B64.decode(body.get("iv").and_then(Value::as_str).unwrap_or("")).map_err(|_|"Bad XAnime IV")?;
    let ct=B64.decode(body.get("ct").and_then(Value::as_str).unwrap_or("")).map_err(|_|"Bad XAnime ciphertext")?;
    if iv.len()!=12{return Err("Bad XAnime IV length".into())}
    let digest=Sha256::digest(b"xanime-ph25-obfuscation-secret-key-2026");
    let cipher=Aes256Gcm::new_from_slice(&digest).map_err(|e|e.to_string())?;
    let plain=cipher.decrypt(Nonce::from_slice(&iv),ct.as_ref()).map_err(|_|"XAnime response decryption failed")?;
    let value:Value=serde_json::from_slice(&plain).map_err(|e|e.to_string())?;
    Ok(value.get("data").cloned().unwrap_or(value))
}
fn episodes_xanime(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let q="query get_q27($select: SearchAnime_Select) { get_q27(select:$select) { paging { pages } items { id data { ani_id al_id info_title } } } }";
    let data=xanime_query(client,q,json!({"select":{"word":title,"sortby":"field_date_create","page":1,"incGenres":[],"excGenres":[],"origStatus":"","type":"","year_from":null,"year_to":null,"season":"","sources":"","ep_total":"","ignoreGlobalGenres":false,"ignoreGlobalBlocks":false,"size":24}}))?;
    let items=data.pointer("/get_q27/items").and_then(Value::as_array).ok_or("XAnime search returned no results")?;
    let hit=best_match(title,items).ok_or("XAnime had no matching title")?;
    let anime=text_first(hit.get("data").unwrap_or(hit),&["ani_id"]).if_empty(&text_first(hit,&["id"]));
    let epq="query get_q01($select: AnimesEpisodesList_Select) { get_q01(select:$select) { paging { pages } items { id data { ani_id ep_id ep_index ep_title ep_sub_index } } } }";
    let mut items_out=Vec::new();
    for page in 1..=4 {
        let data=xanime_query(client,epq,json!({"select":{"ani_id":anime,"init":50,"size":50,"page":page}}))?;
        let items=data.pointer("/get_q01/items").and_then(Value::as_array).cloned().unwrap_or_default();
        for (i,e) in items.iter().enumerate(){
            let d=e.get("data").unwrap_or(e);
            let epid=text_first(d,&["ep_id"]).if_empty(&text_first(e,&["id"]));
            let n=text_first(d,&["ep_index"]).if_empty(&(i+1).to_string());
            items_out.push(make_row("xanime",&anime,&epid,&n,&text_first(d,&["ep_title"])));
        }
        let pages=data.pointer("/get_q01/paging/pages").and_then(Value::as_u64).unwrap_or(1);
        if page as u64>=pages||items.is_empty(){break}
    }
    if items_out.is_empty(){return Err("XAnime returned no episodes".into())}
    Ok(items_out)
}

fn episodes_anidb(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let s=get_json(client,&format!("https://anilab2.amdapi.click/api/search?query={}&page=1",urlencoding::encode(title)),"https://anilab2.amdapi.click/")?;
    let items=arr(&s,&["posts","results"]).ok_or("AniDB search was empty")?;
    let hit=best_match(title,items).ok_or("AniDB had no matching title")?;
    let key=text_first(hit,&["id"]);
    let post=get_json(client,&format!("https://anilab2.amdapi.click/api/post?id={}",urlencoding::encode(&key)),"https://anilab2.amdapi.click/")?;
    let episode_payload=get_json(client,&format!("https://play.anidb.app/api/anime/{}/episodes",urlencoding::encode(&key)),"https://play.anidb.app/")?;
    let eps=arr(&episode_payload,&["list","episodes","results"]).ok_or("AniDB has no episodes")?;
    Ok(eps.iter().enumerate().map(|(i,e)|{
        let n=text_first(e,&["number","episode","episode_number","name"]).if_empty(&(i+1).to_string());
        let eid=text_first(e,&["id","episodeId"]);
        make_row("anidb",&key,&eid,&n,&text_first(e,&["title","name"]))
    }).collect())
}

fn episodes_for_provider(client:&Client,provider:&str,title:&str,dub:bool)->Result<Vec<ProviderRow>,String>{
    match provider {
        "anikoto"=>episodes_anikoto(client,title,dub),
        "animeparadise"=>episodes_animeparadise(client,title),
        "4animo"=>episodes_4animo(client,title),
        "reanime"=>episodes_reanime(client,title),
        "kickassanime"=>episodes_kickass(client,title),
        "animepahe"=>episodes_animepahe(client,title),
        "xanime"=>episodes_xanime(client,title),
        "anipm"=>episodes_anipm(client,title),
        "anidb"=>episodes_anidb(client,title),
        _=>Err(format!("Unknown provider: {provider}"))
    }
}
pub fn anime_episodes(client:&Client,title:&str,_anilist_id:&str,dub:bool)->Result<Vec<ProviderRow>,String>{
    let mut last=String::from("No provider returned episodes.");
    // Website's initial info provider is AniKoto; provider fallback order follows Zenime Web.
    let order=if dub{vec!["anikoto","animeparadise","4animo","reanime","kickassanime","animepahe","xanime","anipm","anidb"]}else{ANIME_PRIORITY.to_vec()};
    for provider in order {
        match episodes_for_provider(client,provider,title,dub){
            Ok(mut rows) if !rows.is_empty()=> {
                rows.sort_by(|a,b|a.number.parse::<f64>().unwrap_or(0.0).partial_cmp(&b.number.parse::<f64>().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
                rows.dedup_by(|a,b|a.id==b.id);
                return Ok(rows);
            }
            Ok(_)=>last=format!("{provider} returned no episode rows."),
            Err(e)=>last=e,
        }
    }
    Err(format!("No configured Zenime provider could list episodes. Last error: {last}"))
}

fn resolve_ap(client:&Client,slug:&str,episode:&str)->Result<(String,String),String>{
    let mut p=episode.split('|'); let uid=p.next().unwrap_or(""); let origin=p.next().unwrap_or("");
    let aid=get_json(client,&format!("https://api.animeparadise.moe/anime/{}",urlencoding::encode(slug)),"https://www.animeparadise.moe/")?;
    let data=aid.get("data").unwrap_or(&aid); let anime_id=text_first(data,&["_id","id"]).if_empty(origin);
    let uid=if uid.is_empty() {
        let eps=get_json(client,&format!("https://api.animeparadise.moe/anime/{}/episode",urlencoding::encode(&anime_id)),"https://www.animeparadise.moe/")?;
        let arr=arr(&eps,&["data"]).ok_or("AnimeParadise episode list not found")?;
        text_first(arr.first().ok_or("AnimeParadise episode missing")?,&["uid","id"])
    }else{uid.to_string()};
    let url=format!("https://www.animeparadise.moe/watch/{}?origin={}",urlencoding::encode(&uid),urlencoding::encode(&anime_id));
    let body=post_text(client,&url,"https://www.animeparadise.moe/",&json!([uid,anime_id]).to_string())?;
    let mut found=String::new();
    for line in body.lines(){
        if let Some(i)=line.find("\"streamLink\""){
            let part=&line[i..];
            if let Some(start)=part.find(':'){let raw=part[start+1..].trim().trim_matches(',').trim_matches('"'); if raw.starts_with("http"){found=raw.to_string();break}}
        }
    }
    if found.is_empty(){
        for token in body.split('"'){if token.starts_with("http")&&token.contains(".m3u8"){found=token.replace("\\/","/");break}}
    }
    if found.is_empty(){return Err("AnimeParadise source resolver returned no stream link".into())}
    if !found.contains(".m3u8"){found=format!("https://stream.animeparadise.moe/m3u8?url={}",urlencoding::encode(&found))}
    Ok((found,"https://www.animeparadise.moe/".into()))
}
fn resolve_anipm(client:&Client,anime:&str,episode:&str,dub:bool)->Result<(String,String),String>{
    let parts:Vec<&str>=episode.split('|').collect();
    let provider=parts.first().copied().unwrap_or("settlar");
    let source=parts.get(1).copied().unwrap_or("");
    let route=parts.get(2).copied().unwrap_or("");
    let ep=parts.get(3).copied().unwrap_or(route);
    let language=if dub{"dub"}else{parts.get(4).copied().unwrap_or("sub")};
    let boot=get_json(client,&format!("https://ani.pm/api/anime/playback-bootstrap/{}/{}?ep={}&lang={}",urlencoding::encode(provider),urlencoding::encode(source),urlencoding::encode(route),language),"https://ani.pm/")?;
    let selection=text_first(&boot,&["settlarSelection"]);
    if selection.is_empty(){return Err("AniPM playback bootstrap did not return a session".into())}
    let session=get_json(client,&format!("https://ani.pm/api/anime/settlar/session?selection={}&provider=anipm&ep={}&channel={}&telemetry=0",urlencoding::encode(&selection),urlencoding::encode(ep),language),"https://ani.pm/")?;
    let embed=text_first(&session,&["embedUrl"]);
    let token=url::Url::parse(&embed).ok().and_then(|u|u.query_pairs().find(|(k,_)|k=="t").map(|(_,v)|v.to_string())).ok_or("AniPM embed session did not include a token")?;
    let embed_json=get_json(client,&format!("https://embed.settlar.io/api/embed/session?t={}",urlencoding::encode(&token)),"https://embed.settlar.io/")?;
    let stream=text_first(&embed_json,&["source","url"]);
    if direct_url(&stream){Ok((stream,"https://embed.settlar.io/".into()))}else{Err("AniPM session returned no direct stream".into())}
}
fn resolve_animepahe(client:&Client,episode:&str)->Result<(String,String),String>{
    let url=format!("https://animepahe.pw/play/{}",episode);
    maybe_direct(client,&url,"https://animepahe.pw/")
}
fn resolve_anikoto(client:&Client,episode:&str)->Result<(String,String),String>{
    let server_html=get_json(client,&format!("https://anikototv.to/ajax/server/list?servers={}",urlencoding::encode(episode)),"https://anikototv.to/")?;
    let result=text_first(&server_html,&["result"]);
    let doc=Html::parse_fragment(&result);
    for seltext in [".servers .type li[data-link-id]","li[data-link-id]","a[href]"]{
        if let Some(sel)=selector(seltext){
            for el in doc.select(&sel){
                let link_id=el.value().attr("data-link-id").or_else(||el.value().attr("href")).unwrap_or("");
                if link_id.is_empty(){continue}
                let resolved=get_json(client,&format!("https://anikototv.to/ajax/server?get={}",urlencoding::encode(link_id)),"https://anikototv.to/")?;
                let embed=text_first(&resolved,&["url"]);
                if embed.starts_with("http"){if let Ok(media)=maybe_direct(client,&embed,"https://anikototv.to/"){return Ok(media)}}
            }
        }
    }
    Err("AniKoto servers did not expose a direct HLS/MP4 stream.".into())
}
fn resolve_kickass(client:&Client,episode:&str)->Result<(String,String),String>{
    let path=if episode.starts_with("http"){episode.to_string()}else{format!("https://kaa.lt/api/show/{}",episode.trim_start_matches('/'))};
    let data=get_json(client,&path,"https://kaa.lt/")?;
    let mut urls=Vec::new();collect_media(&data,&mut urls);
    for (url,_) in urls {if direct_url(&url){return Ok((url,"https://kaa.lt/".into()))}}
    if let Some(servers)=arr(&data,&["servers","result"]){
        for server in servers{
            let src=text_first(server,&["src","url","link"]);
            if src.starts_with("http"){if let Ok(x)=maybe_direct(client,&src,"https://kaa.lt/"){return Ok(x)}}
        }
    }
    Err("KickAssAnime returned embed servers but no direct stream.".into())
}
fn resolve_reanime(client:&Client,anime:&str,episode:&str)->Result<(String,String),String>{
    let (raw,n)=split_ep(episode);
    let ep=if raw.contains("?ep="){raw.split("?ep=").nth(1).unwrap_or(&n)}else{raw.as_str()};
    let payload=get_json(client,&format!("https://reanime.to/api/flix/{}/{}",urlencoding::encode(anime),urlencoding::encode(ep)),"https://reanime.to/")?;
    let mut urls=Vec::new();collect_media(&payload,&mut urls);
    for (url,_) in urls{if direct_url(&url){return Ok((url,"https://reanime.to/".into()))}}
    if let Some(servers)=arr(&payload,&["servers"]){for s in servers{
        let url=text_first(s,&["dataLink","url","link"]);
        if url.starts_with("http"){if let Ok(media)=maybe_direct(client,&url,"https://reanime.to/"){return Ok(media)}}
    }}
    Err("ReAnime returned embed links but no direct stream.".into())
}
fn resolve_xanime(client:&Client,anime:&str,episode:&str,dub:bool)->Result<(String,String),String>{
    let (source_id,_)=split_ep(episode);
    let q="query get_q07($select: Episodes_Select) { get_q07(select:$select) { data { sourcesNode_list { id data { src_type souPath m3u8_lists { iframe } track { label trackPath } } } } }";
    let payload=xanime_query(client,q,json!({"select":{"id":source_id}}))?;
    let sources=payload.pointer("/get_q07/data/sourcesNode_list").and_then(Value::as_array).ok_or("XAnime has no source list")?;
    let mut urls=Vec::new();
    for source in sources{
        let d=source.get("data").unwrap_or(source);
        let typ=text_first(d,&["src_type"]).to_lowercase();
        if (dub && typ!="dub")||(!dub && typ=="dub"){continue}
        collect_media(d,&mut urls);
        let raw=text_first(d,&["souPath"]);
        if direct_url(&raw){urls.push((raw,String::new()))}
        if let Some(list)=d.get("m3u8_lists").and_then(Value::as_array){for x in list{
            let url=text_first(x,&["iframe","url"]);
            if direct_url(&url){urls.push((url,String::new()))}
            else if url.starts_with("http"){if let Ok(x)=maybe_direct(client,&url,"https://xanime.me/"){return Ok(x)}}
        }}
    }
    urls.into_iter().find(|(u,_)|direct_url(u)).map(|(u,_)|(u,"https://xanime.me/".into())).ok_or_else(||format!("XAnime resolved metadata for {anime} but not a direct media URL."))
}
fn resolve_anidb(client:&Client,episode:&str)->Result<(String,String),String>{
    let servers=get_json(client,&format!("https://play.anidb.app/api/episode/{}/servers",urlencoding::encode(episode)),"https://play.anidb.app/")?;
    let list=arr(&servers,&["list","servers","results"]).ok_or("AniDB had no server list")?;
    for s in list{
        let id=text_first(s,&["id","serverId"]);
        if id.is_empty(){continue}
        let iframe=get_json(client,&format!("https://play.anidb.app/api/episode/{}/iframe",urlencoding::encode(&id)),"https://play.anidb.app/")?;
        let link=text_first(&iframe,&["link","url"]);
        if link.starts_with("http"){if let Ok(x)=maybe_direct(client,&link,"https://anidb.app/"){return Ok(x)}}
    }
    Err("AniDB servers did not expose a direct stream.".into())
}

pub fn resolve_episode(client:&Client,packed_id:&str,title:&str,dub:bool)->Result<(String,String),String>{
    let (provider,anime,encoded)=unpack(packed_id,"zn").ok_or("This episode does not contain a native provider reference")?;
    let (episode,n)=split_ep(encoded);
    let attempt=match provider {
        "animeparadise"=>resolve_ap(client,anime,&episode),
        "anipm"=>resolve_anipm(client,anime,&episode,dub),
        "animepahe"=>resolve_animepahe(client,&episode),
        "anikoto"=>resolve_anikoto(client,&episode),
        "kickassanime"=>resolve_kickass(client,&episode),
        "reanime"=>resolve_reanime(client,anime,&episode),
        "xanime"=>resolve_xanime(client,anime,&episode,dub),
        "anidb"=>resolve_anidb(client,&episode),
        "4animo"=>{let target=if episode.starts_with("http"){episode.clone()}else if episode.starts_with("?"){format!("{}{}",anime,episode)}else{format!("https://zanix.tr/watch/{}",episode.trim_start_matches("/"))};maybe_direct(client,&target,"https://zanix.tr/")},
        _=>Err(format!("No resolver for {provider}"))
    };
    if let Ok(media)=attempt{return Ok(media)}
    let mut last=format!("{provider} source failed");
    // A provider may be down for one episode; retry this episode number on the web
    // application's fallback providers without changing its AniList metadata identity.
    for fallback in ANIME_PRIORITY {
        if *fallback==provider{continue}
        match episodes_for_provider(client,fallback,title,dub){
            Ok(rows)=>{
                let candidate=rows.iter().find(|r|r.number==n).or_else(||rows.iter().find(|r|r.number==episode));
                if let Some(row)=candidate {
                    if let Some((p,a,e))=unpack(&row.id,"zn"){
                        let (ep,_)=split_ep(e);
                        let res=match p{
                            "animeparadise"=>resolve_ap(client,a,&ep),
                            "anipm"=>resolve_anipm(client,a,&ep,dub),
                            "animepahe"=>resolve_animepahe(client,&ep),
                            "anikoto"=>resolve_anikoto(client,&ep),
                            "kickassanime"=>resolve_kickass(client,&ep),
                            "reanime"=>resolve_reanime(client,a,&ep),
                            "xanime"=>resolve_xanime(client,a,&ep,dub),
                            "anidb"=>resolve_anidb(client,&ep),
                            "4animo"=>{let target=if ep.starts_with("http"){ep.clone()}else if ep.starts_with("?"){format!("{}{}",a,ep)}else{format!("https://zanix.tr/watch/{}",ep.trim_start_matches("/"))};maybe_direct(client,&target,"https://zanix.tr/")},
                            _=>Err("Unsupported fallback".into())
                        };
                        if let Ok(result)=res{return Ok(result)}
                    }
                }
            }
            Err(e)=>last=e,
        }
    }
    Err(format!("No native playable source was found for this episode. Embed-only hosts cannot be rendered inside the native player. Last provider error: {last}"))
}

// ---- Manga source adapters: regular providers only, in Zenime Web priority order. ----
fn manga_search(client:&Client,provider:&str,title:&str)->Result<(String,String),String>{
    match provider{
        "atsumaru"=>{
            let url=format!("https://atsu.moe/collections/manga/documents/search?q={}&filter_by=hidden%3A%21%3Dtrue&sort_by=views%3Adesc&query_by=title%2CenglishTitle%2CotherNames%2Cauthors&query_by_weights=4%2C3%2C2%2C1&num_typos=4%2C3%2C2%2C1&page=1&per_page=20",urlencoding::encode(title));
            let s=get_json(client,&url,"https://atsu.moe/")?;
            let hits=arr(&s,&["hits"]).ok_or("Atsumaru search empty")?;
            let documents:Vec<Value>=hits.iter().map(|hit|hit.get("document").cloned().unwrap_or_else(||hit.clone())).collect();
            let hit=best_match(title,&documents).ok_or("Atsumaru title not matched")?;
            let id=text_first(hit,&["id","mangaId"]);
            if id.is_empty(){Err("Atsumaru result lacks id".into())}else{Ok((id,title_of(hit)))}
        },
        "mangapill"=>{
            let body=get_text(client,&format!("https://mangapill.com/search?q={}",urlencoding::encode(title)),"https://mangapill.com/")?;
            let links=html_links(&body,"https://mangapill.com","div.container div.my-3.justify-end > div a");
            let (name,url)=links.iter().find(|(t,_)|title_match(title,t)).or_else(||links.first()).ok_or("MangaPill search empty")?;
            let id=url.split("/manga/").nth(1).unwrap_or("").split('/').next().unwrap_or("");
            if id.is_empty(){Err("MangaPill result lacks id".into())}else{Ok((id.to_string(),name.to_string()))}
        },
        "mangakatana"=>{
            let body=get_text(client,&format!("https://mangakatana.com/page/1?search={}&search_by=book_name",urlencoding::encode(title)),"https://mangakatana.com/")?;
            let links=html_links(&body,"https://mangakatana.com","div#book_list > div.item div.text > h3 > a");
            let (name,url)=links.iter().find(|(t,_)|title_match(title,t)).or_else(||links.first()).ok_or("MangaKatana search empty")?;
            let path=url.strip_prefix("https://mangakatana.com/").unwrap_or(&url);
            Ok((path.to_string(),name.to_string()))
        },
        "mangahere"=>{
            let body=get_text(client,&format!("https://mangahere.cc/search?title={}&page=1",urlencoding::encode(title)),"https://mangahere.cc/")?;
            let links=html_links(&body,"https://mangahere.cc","div.container > div > div > ul > li p.manga-list-4-item-title > a");
            let (name,url)=links.iter().find(|(t,_)|title_match(title,t)).or_else(||links.first()).ok_or("MangaHere search empty")?;
            let id=url.split("/manga/").nth(1).unwrap_or(url.split('/').last().unwrap_or(""));
            Ok((id.trim_end_matches('/').to_string(),name.to_string()))
        },
        _=>Err("Unknown regular manga provider".into())
    }
}
fn manga_chapters_for(client:&Client,provider:&str,title:&str)->Result<Vec<ProviderRow>,String>{
    let (id,matched_title)=manga_search(client,provider,title)?;
    let referer=match provider{"atsumaru"=>"https://atsu.moe/","mangapill"=>"https://mangapill.com/","mangakatana"=>"https://mangakatana.com/",_=>"https://mangahere.cc/"};
    let mut out=Vec::new();
    match provider {
        "atsumaru"=>{
            let data=get_json(client,&format!("https://atsu.moe/api/manga/allChapters?mangaId={}",urlencoding::encode(&id)),referer)?;
            if let Some(eps)=arr(&data,&["chapters"]){for c in eps{
                let cid=text_first(c,&["id","chapterId"]);if cid.is_empty(){continue}
                let title=text_first(c,&["title","number","chapter"]).if_empty("Chapter");
                let n=numbers(&title).if_empty("0");
                out.push(ProviderRow{id:packed("znm",provider,&id,&cid),number:n,title,provider:provider.into()});
            }}
        },
        "mangapill"=>{
            let html=get_text(client,&format!("https://mangapill.com/manga/{}",urlencoding::encode(&id)),referer)?;
            let doc=Html::parse_document(&html);
            if let Some(sel)=selector("div.container div.border-border div#chapters div.grid-cols-1 a"){
                for a in doc.select(&sel){let href=a.value().attr("href").unwrap_or("");
                    let cid=href.split("/chapters/").nth(1).unwrap_or("");
                    if cid.is_empty(){continue}
                    let t=a.text().collect::<Vec<_>>().join(" ").trim().to_string();
                    out.push(ProviderRow{id:packed("znm",provider,&id,cid),number:numbers(&t),title:if t.is_empty(){"Chapter".into()}else{t},provider:provider.into()});
                }
            }
        },
        "mangakatana"=>{
            let page=get_text(client,&format!("https://mangakatana.com/{}",id.trim_start_matches('/')),referer)?;
            let doc=Html::parse_document(&page);
            if let Some(sel)=selector("tr"){
                for tr in doc.select(&sel){
                    let txt=tr.text().collect::<Vec<_>>().join(" ");
                    if !txt.to_lowercase().contains("chapter"){continue}
                    if let Some(a_sel)=selector("a"){
                        if let Some(a)=tr.select(&a_sel).next(){
                            let href=a.value().attr("href").unwrap_or("");
                            if href.is_empty(){continue}
                            let cid=href.strip_prefix("https://mangakatana.com/").unwrap_or(href);
                            let t=a.text().collect::<Vec<_>>().join(" ").trim().to_string();
                            out.push(ProviderRow{id:packed("znm",provider,&id,cid),number:numbers(&t),title:t,provider:provider.into()});
                        }
                    }
                }
            }
        },
        _=>{
            let page=get_text(client,&format!("https://mangahere.cc/manga/{}",id.trim_start_matches('/')),referer)?;
            let doc=Html::parse_document(&page);
            if let Some(sel)=selector("ul.detail-main-list > li a"){
                for a in doc.select(&sel){
                    let href=a.value().attr("href").unwrap_or("");
                    let cid=href.split("/manga/").nth(1).unwrap_or("").trim_end_matches(".html");
                    if cid.is_empty(){continue}
                    let t= a.text().collect::<Vec<_>>().join(" ").trim().to_string();
                    out.push(ProviderRow{id:packed("znm",provider,&id,cid),number:numbers(&t),title:if t.is_empty(){"Chapter".into()}else{t},provider:provider.into()});
                }
            }
        }
    }
    if out.is_empty(){Err(format!("{provider} found {matched_title}, but chapter extraction returned no rows"))}else{
        out.sort_by(|a,b|b.number.parse::<f64>().unwrap_or(-1.0).partial_cmp(&a.number.parse::<f64>().unwrap_or(-1.0)).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }
}
pub fn manga_chapters(client:&Client,title:&str)->Result<Vec<ProviderRow>,String>{
    let mut last=String::from("No manga provider returned chapters.");
    for provider in MANGA_PRIORITY{
        match manga_chapters_for(client,provider,title){
            Ok(rows) if !rows.is_empty()=>return Ok(rows),
            Ok(_)=>last=format!("{provider} returned no chapters"),
            Err(e)=>last=e,
        }
    }
    Err(format!("Unable to load regular manga chapters: {last}"))
}
pub fn manga_pages(client:&Client,packed_id:&str)->Result<Vec<String>,String>{
    let (provider,manga,chapter)=unpack(packed_id,"znm").ok_or("Chapter has no native manga provider reference")?;
    let referer=match provider{"atsumaru"=>"https://atsu.moe/","mangapill"=>"https://mangapill.com/","mangakatana"=>"https://mangakatana.com/",_=>"https://mangahere.cc/"};
    match provider {
        "atsumaru"=>{
            let body=get_json(client,&format!("https://atsu.moe/api/read/chapter?mangaId={}&chapterId={}",urlencoding::encode(manga),urlencoding::encode(chapter)),referer)?;
            let pages=body.pointer("/readChapter/pages").or_else(||body.pointer("/pages")).and_then(Value::as_array).ok_or("Atsumaru returned no pages")?;
            Ok(pages.iter().filter_map(|p|{
                let image=first(p,&["image","url","img"]);let s=value_text(image);
                if s.starts_with("http"){Some(s)}else if s.starts_with("//"){Some(format!("https:{s}"))}else{None}
            }).collect())
        },
        "mangapill"=>{
            let html=get_text(client,&format!("https://mangapill.com/chapters/{}",urlencoding::encode(chapter)),referer)?;
            let doc=Html::parse_document(&html);let mut pages=Vec::new();
            if let Some(sel)=selector("chapter-page div picture img, chapter-page img[data-src], img[data-src*='jpg'], img[data-src*='webp']"){
                for img in doc.select(&sel){let u=img.value().attr("data-src").or_else(||img.value().attr("src")).unwrap_or("");
                    if u.starts_with("http")&&!pages.contains(&u.to_string()){pages.push(u.to_string())}}
            }
            if pages.is_empty(){Err("MangaPill page image extraction returned nothing".into())}else{Ok(pages)}
        },
        "mangakatana"=>{
            let html=get_text(client,&format!("https://mangakatana.com/{}",chapter.trim_start_matches('/')),referer)?;
            let doc=Html::parse_document(&html);let mut pages=Vec::new();
            if let Some(sel)=selector("img[src], img[data-src]"){
                for img in doc.select(&sel){let u=img.value().attr("data-src").or_else(||img.value().attr("src")).unwrap_or("");
                    if u.starts_with("http")&&u.contains('.')&&!u.contains("logo")&&!u.contains("favicon")&&!pages.contains(&u.to_string()){pages.push(u.to_string())}}
            }
            for script in doc.select(&selector("script").unwrap()){
                let js=script.inner_html();
                for ext in [".jpg",".jpeg",".png",".webp",".avif"]{
                    let mut rest=js.as_str();
                    while let Some(i)=rest.find(ext){
                        let e=i+ext.len();let st=rest[..i].rfind('"').or_else(||rest[..i].rfind('\'')).map(|p|p+1).unwrap_or(0);
                        let u=rest[st..e].replace("\\/","/");
                        if u.starts_with("http")&&!pages.contains(&u){pages.push(u)}
                        rest=&rest[e..];
                    }
                }
            }
            if pages.is_empty(){Err("MangaKatana returned no page URLs".into())}else{Ok(pages)}
        },
        _=>{
            let url=format!("https://mangahere.cc/manga/{}/1.html",chapter.trim_start_matches('/'));
            let html=get_text(client,&url,referer)?;
            let doc=Html::parse_document(&html);let mut pages=Vec::new();
            if let Some(sel)=selector("img[src], img[data-src], script"){
                for el in doc.select(&sel){
                    let candidate=el.value().attr("data-src").or_else(||el.value().attr("src")).unwrap_or("");
                    if candidate.starts_with("http")&&(candidate.contains(".jpg")||candidate.contains(".png")||candidate.contains(".webp"))&&!pages.contains(&candidate.to_string()){pages.push(candidate.to_string())}
                    let js=el.inner_html();
                    for ext in [".jpg",".jpeg",".png",".webp"]{
                        let mut rest=js.as_str();
                        while let Some(i)=rest.find(ext){
                            let e=i+ext.len();let st=rest[..i].rfind('"').or_else(||rest[..i].rfind('\'')).map(|p|p+1).unwrap_or(0);
                            let u=rest[st..e].replace("\\/","/");
                            if u.starts_with("http")&&!pages.contains(&u){pages.push(u)}
                            rest=&rest[e..];
                        }
                    }
                }
            }
            if pages.is_empty(){Err("MangaHere returned no readable page URLs (its packed chapter script may need a dedicated decoder).".into())}else{Ok(pages)}
        }
    }
}

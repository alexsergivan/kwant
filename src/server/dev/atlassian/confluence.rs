//! Confluence reads, always inside the site's chosen spaces: CQL built by `jql::cql` (never a bot's
//! query), hits filtered by space again on the way back, and `page(id)` refused for a page whose
//! space wasn't chosen. Cloud: v1 search plus v2 pages; Data Center: v1 content search and content.

use std::collections::HashMap;

use reqwest::Method;
use serde_json::Value;

use super::client::{Api, AtlassianClient, Kind};
use super::jql;
use super::text::{decode, html_to_text, strip_highlight};
use crate::server::dev::tickets::source::*;

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// A Confluence page or content id: digits only.
pub fn valid_page_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// A space the token can see.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SpaceInfo {
    pub key: String,
    pub name: String,
}

/// `/spaces/OPS` or `/display/OPS` → `OPS`.
fn space_from_path(p: &str) -> Option<String> {
    let mut parts = p.split('/').filter(|x| !x.is_empty());
    while let Some(x) = parts.next() {
        if x == "spaces" || x == "display" {
            return parts.next().map(str::to_string);
        }
    }
    None
}

impl AtlassianClient {
    fn chosen(&self, space: &str) -> bool {
        !space.is_empty() && self.site.spaces.iter().any(|s| s == space)
    }

    fn conf(&self) -> Result<String, SourceError> {
        if self.site.spaces.is_empty() {
            return Err(SourceError::NoSpaces);
        }
        self.confluence_base().ok_or(SourceError::NoSpaces)
    }

    /// Spaces the token can see (first 100, global and personal skipped), for the sheet's pills.
    pub async fn spaces(&self) -> Result<Vec<SpaceInfo>, SourceError> {
        let base = self.confluence_base().ok_or(SourceError::NoSpaces)?;
        let (url, list_key) = match self.site.kind {
            Kind::Cloud => (format!("{base}/api/v2/spaces?limit=100&type=global"), "results"),
            Kind::Dc => (format!("{base}/rest/api/space?limit=100&type=global"), "results"),
        };
        let v = self.call(Api::Confluence, Method::GET, &url, None).await?;
        Ok(v[list_key].as_array().into_iter().flatten().map(|x| SpaceInfo { key: s(&x["key"]), name: s(&x["name"]) }).filter(|x| jql::valid_space(&x.key)).collect())
    }

    /// Cloud: the chosen spaces' numeric ids, looked up once per client.
    async fn space_ids(&self, base: &str) -> Result<HashMap<String, String>, SourceError> {
        if let Some(m) = self.space_ids.lock().unwrap().clone() {
            return Ok(m);
        }
        let keys = self.site.spaces.iter().map(|k| enc(k)).collect::<Vec<_>>().join(",");
        let v = self.call(Api::Confluence, Method::GET, &format!("{base}/api/v2/spaces?keys={keys}&limit=25"), None).await?;
        let m: HashMap<String, String> = v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|x| self.chosen(x["key"].as_str().unwrap_or_default()))
            .map(|x| (x["id"].as_str().map(str::to_string).unwrap_or_else(|| x["id"].to_string()), s(&x["key"])))
            .collect();
        *self.space_ids.lock().unwrap() = Some(m.clone());
        Ok(m)
    }

    pub(super) async fn confluence_search(&self, q: &PageQuery) -> Result<Vec<PageHit>, SourceError> {
        let base = self.conf()?;
        let cql = jql::cql(&self.site.spaces, &q.text, q.title_only).ok_or_else(|| SourceError::Invalid("say what to search for, in words".into()))?;
        let limit = q.limit.clamp(1, MAX_PAGE_HITS);
        let web = self.confluence_web();
        let hits: Vec<PageHit> = match self.site.kind {
            Kind::Cloud => {
                let url = format!("{base}/rest/api/search?cql={}&limit={limit}&expand=content.space,content.version", enc(&cql));
                let v = self.call(Api::Confluence, Method::GET, &url, None).await?;
                v["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|r| r["content"]["type"] == "page")
                    .map(|r| {
                        let c = &r["content"];
                        let space = c["space"]["key"].as_str().map(str::to_string).or_else(|| r["resultGlobalContainer"]["displayUrl"].as_str().and_then(space_from_path)).unwrap_or_default();
                        PageHit {
                            id: s(&c["id"]),
                            title: decode(&strip_highlight(&s(&c["title"]).replace("@@@hl@@@", "").replace("@@@endhl@@@", ""))),
                            version: c["version"]["number"].as_u64().unwrap_or(0) as u32,
                            url: format!("{web}/spaces/{space}/pages/{}", s(&c["id"])),
                            excerpt: cut(&html_to_text(&strip_highlight(&s(&r["excerpt"]))), 300).0,
                            updated: c["version"]["when"].as_str().or(r["lastModified"].as_str()).and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|d| d.with_timezone(&chrono::Utc)),
                            space_key: space,
                        }
                    })
                    .collect()
            }
            Kind::Dc => {
                let url = format!("{base}/rest/api/content/search?cql={}&limit={limit}&expand=space,version", enc(&cql));
                let v = self.call(Api::Confluence, Method::GET, &url, None).await?;
                v["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["type"] == "page")
                    .map(|c| PageHit {
                        id: s(&c["id"]),
                        title: s(&c["title"]),
                        space_key: s(&c["space"]["key"]),
                        version: c["version"]["number"].as_u64().unwrap_or(0) as u32,
                        url: format!("{web}/pages/viewpage.action?pageId={}", s(&c["id"])),
                        excerpt: String::new(),
                        updated: c["version"]["when"].as_str().and_then(|t| super::jira::jira_date(&Value::String(t.to_string()))),
                    })
                    .collect()
            }
        };
        // The CQL already says `space in (…)`; this is the second lock on the same door.
        Ok(hits.into_iter().filter(|h| valid_page_id(&h.id) && self.chosen(&h.space_key)).take(limit).collect())
    }

    pub(super) async fn confluence_page(&self, id: &str) -> Result<Page, SourceError> {
        let base = self.conf()?;
        if !valid_page_id(id) {
            return Err(SourceError::Invalid("a Confluence page id is a number".into()));
        }
        let web = self.confluence_web();
        let (title, space, version, storage) = match self.site.kind {
            Kind::Cloud => {
                let v = self.call(Api::Confluence, Method::GET, &format!("{base}/api/v2/pages/{id}?body-format=storage"), None).await?;
                let space_id = v["spaceId"].as_str().map(str::to_string).unwrap_or_else(|| v["spaceId"].to_string());
                let ids = self.space_ids(&base).await?;
                let Some(space) = ids.get(&space_id) else { return Err(SourceError::OutOfScope) };
                (s(&v["title"]), space.clone(), v["version"]["number"].as_u64().unwrap_or(0) as u32, s(&v["body"]["storage"]["value"]))
            }
            Kind::Dc => {
                let v = self.call(Api::Confluence, Method::GET, &format!("{base}/rest/api/content/{id}?expand=body.storage,version,space"), None).await?;
                if v["type"] != "page" {
                    return Err(SourceError::NotFound);
                }
                (s(&v["title"]), s(&v["space"]["key"]), v["version"]["number"].as_u64().unwrap_or(0) as u32, s(&v["body"]["storage"]["value"]))
            }
        };
        if !self.chosen(&space) {
            return Err(SourceError::OutOfScope);
        }
        let (body, truncated) = cut(&html_to_text(&storage), PAGE_CHARS);
        let url = match self.site.kind {
            Kind::Cloud => format!("{web}/spaces/{space}/pages/{id}"),
            Kind::Dc => format!("{web}/pages/viewpage.action?pageId={id}"),
        };
        Ok(Page { id: id.to_string(), title, space_key: space, version, url, body, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ids_and_space_paths() {
        assert!(valid_page_id("65601") && !valid_page_id("65601/../x") && !valid_page_id(""));
        assert_eq!(space_from_path("/spaces/OPS").as_deref(), Some("OPS"));
        assert_eq!(space_from_path("/display/HR").as_deref(), Some("HR"));
        assert_eq!(space_from_path("/x"), None);
    }
}

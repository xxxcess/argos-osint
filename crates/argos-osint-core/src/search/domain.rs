//! Domain stage. RDAP, crt.sh, DNS-over-HTTPS, and Wayback run only when the
//! question contains a domain. InternetDB runs for the public addresses that
//! DNS returns. A resolver or registry that is down does not fail the others.

use std::net::IpAddr;

use futures_util::future::join_all;
use serde_json::Value;

use super::{
    clip, get_json, ip_blocked, merge_adapter_results, request, tag, Job, RawHttp, SearchHit,
};

pub async fn gather(domains: Vec<String>) -> Result<Vec<SearchHit>, String> {
    let mut jobs: Vec<Job> = Vec::new();
    for domain in domains.into_iter().take(2) {
        let rdap_domain = domain.clone();
        jobs.push(Box::pin(async move { ("rdap", rdap(&rdap_domain).await) }));
        let crt_domain = domain.clone();
        jobs.push(Box::pin(async move {
            ("crt.sh", certificates(&crt_domain).await)
        }));
        let wayback_domain = domain.clone();
        jobs.push(Box::pin(async move {
            ("wayback", wayback(&wayback_domain).await)
        }));
        jobs.push(Box::pin(async move {
            ("doh", dns_and_internetdb(&domain).await)
        }));
    }
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    merge_adapter_results(join_all(jobs).await)
}

async fn rdap(domain: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!("https://rdap.org/domain/{domain}");
    let value = get_json(&url).await?;
    if value.get("errorCode").is_some() {
        return Ok(Vec::new());
    }
    Ok(tag(parse_rdap(domain, &value), "rdap"))
}

pub(crate) fn parse_rdap(domain: &str, value: &Value) -> Vec<SearchHit> {
    if value.get("errorCode").is_some() {
        return Vec::new();
    }
    let name = value
        .get("ldhName")
        .and_then(|v| v.as_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(domain);
    let status = value
        .get("status")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let nameservers = value
        .get("nameservers")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("ldhName").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let registrar = value
        .get("entities")
        .and_then(|v| v.as_array())
        .and_then(|rows| {
            rows.iter().find_map(|entity| {
                let roles = entity.get("roles").and_then(|v| v.as_array())?;
                if !roles.iter().any(|role| role.as_str() == Some("registrar")) {
                    return None;
                }
                vcard_fn(entity)
            })
        })
        .unwrap_or_default();
    let registered = event_date(value, "registration");
    let mut parts = Vec::new();
    if !status.is_empty() {
        parts.push(format!("status {status}"));
    }
    if !registrar.is_empty() {
        parts.push(format!("registrar {registrar}"));
    }
    if !nameservers.is_empty() {
        parts.push(format!("ns {nameservers}"));
    }
    if !registered.is_empty() {
        parts.push(format!("registered {registered}"));
    }
    vec![SearchHit {
        title: name.to_string(),
        url: format!("https://rdap.org/domain/{domain}"),
        snippet: clip(&parts.join("; "), 360),
    }]
}

fn vcard_fn(entity: &Value) -> Option<String> {
    let rows = entity.get("vcardArray")?.get(1)?.as_array()?;
    for row in rows {
        let Some(items) = row.as_array() else {
            continue;
        };
        if items.first().and_then(|item| item.as_str()) == Some("fn") {
            return items
                .get(3)
                .and_then(|item| item.as_str())
                .map(|name| name.to_string());
        }
    }
    None
}

fn event_date(value: &Value, action: &str) -> String {
    value
        .get("events")
        .and_then(|v| v.as_array())
        .and_then(|rows| {
            rows.iter().find_map(|event| {
                if event.get("eventAction").and_then(|v| v.as_str()) == Some(action) {
                    event
                        .get("eventDate")
                        .and_then(|v| v.as_str())
                        .map(|date| date.to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default()
}

async fn certificates(domain: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!("https://crt.sh/?q=%25.{domain}&output=json");
    let value = get_json(&url).await?;
    Ok(tag(parse_crtsh(&value), "crt.sh"))
}

pub(crate) fn parse_crtsh(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    let mut seen = Vec::new();
    for item in rows {
        let name = item
            .get("common_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let name_value = item
            .get("name_value")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let title = if name.is_empty() {
            name_value.lines().next().unwrap_or("").trim()
        } else {
            name
        };
        if title.is_empty() || seen.iter().any(|existing: &String| existing == title) {
            continue;
        }
        seen.push(title.to_string());
        let id = item.get("id").and_then(|v| v.as_i64()).or_else(|| {
            item.get("id")
                .and_then(|v| v.as_str())
                .and_then(|text| text.parse().ok())
        });
        let url = if let Some(id) = id {
            format!("https://crt.sh/?id={id}")
        } else {
            format!("https://crt.sh/?q={title}")
        };
        let issuer = item
            .get("issuer_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let not_before = item
            .get("not_before")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let not_after = item.get("not_after").and_then(|v| v.as_str()).unwrap_or("");
        let names = name_value.lines().take(4).collect::<Vec<_>>().join(", ");
        let snippet = format!("{issuer}; {not_before} → {not_after}; {names}");
        hits.push(SearchHit {
            title: title.to_string(),
            url,
            snippet: clip(&snippet, 360),
        });
        if hits.len() == 5 {
            break;
        }
    }
    hits
}

async fn wayback(domain: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://web.archive.org/cdx/search/cdx?url={}&output=json&fl=original,timestamp,statuscode,mimetype&filter=statuscode:200&limit=5&collapse=digest",
        urlencoding::encode(domain)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_wayback(&value), "wayback"))
}

pub(crate) fn parse_wayback(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let cells = row.as_array()?;
            let original = cells.first().and_then(|v| v.as_str()).unwrap_or("").trim();
            let timestamp = cells.get(1).and_then(|v| v.as_str()).unwrap_or("").trim();
            if original.is_empty() || timestamp.is_empty() || !original.starts_with("http") {
                return None;
            }
            let status = cells.get(2).and_then(|v| v.as_str()).unwrap_or("");
            let mime = cells.get(3).and_then(|v| v.as_str()).unwrap_or("");
            Some(SearchHit {
                title: original.to_string(),
                url: format!("https://web.archive.org/web/{timestamp}/{original}"),
                snippet: clip(&format!("{timestamp} {status} {mime}"), 360),
            })
        })
        .take(5)
        .collect()
}

async fn dns_and_internetdb(domain: &str) -> Result<Vec<SearchHit>, String> {
    let (v4, v6) = tokio::join!(doh(domain, "A"), doh(domain, "AAAA"));
    let mut hits = Vec::new();
    let mut ips = Vec::new();
    let mut errors = Vec::new();
    for result in [v4, v6] {
        match result {
            Ok(answer) => {
                if let Some(hit) = answer.hit {
                    hits.push(hit);
                }
                ips.extend(answer.ips);
            }
            Err(err) => errors.push(err),
        }
    }
    if hits.is_empty() && !errors.is_empty() && ips.is_empty() {
        return Err(errors.join("; "));
    }
    ips.sort();
    ips.dedup();
    let mut db_errors = Vec::new();
    for ip in ips.into_iter().take(4) {
        match internetdb(&ip).await {
            Ok(rows) => hits.extend(rows),
            Err(err) => db_errors.push(err),
        }
    }
    if hits.is_empty() {
        if !db_errors.is_empty() {
            return Err(db_errors.join("; "));
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
    }
    Ok(hits)
}

pub(crate) struct DnsAnswer {
    hit: Option<SearchHit>,
    ips: Vec<String>,
}

async fn doh(domain: &str, kind: &str) -> Result<DnsAnswer, String> {
    let url = format!(
        "https://cloudflare-dns.com/dns-query?name={}&type={kind}",
        urlencoding::encode(domain)
    );
    let raw = request(
        reqwest::Method::GET,
        &url,
        None,
        "application/dns-json",
        &[],
    )
    .await?;
    let value = super::parse_json_ok(raw)?;
    Ok(parse_doh(domain, kind, &value))
}

pub(crate) fn parse_doh(domain: &str, kind: &str, value: &Value) -> DnsAnswer {
    let rows = value
        .get("Answer")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut ips = Vec::new();
    for row in rows {
        let data = row
            .get("data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if data.is_empty() {
            continue;
        }
        let record = row.get("type").and_then(|v| v.as_u64()).unwrap_or(0);
        let label = match record {
            1 => "A",
            28 => "AAAA",
            5 => "CNAME",
            _ => kind,
        };
        lines.push(format!("{label} {data}"));
        if record == 1 || record == 28 {
            if let Ok(ip) = data.parse::<IpAddr>() {
                if !ip_blocked(ip) {
                    ips.push(ip.to_string());
                }
            }
        }
    }
    let hit = if lines.is_empty() {
        None
    } else {
        Some(SearchHit {
            title: format!("{domain} {kind}"),
            url: format!(
                "https://cloudflare-dns.com/dns-query?name={}&type={kind}",
                urlencoding::encode(domain)
            ),
            snippet: clip(&lines.join("; "), 360),
        })
    };
    DnsAnswer {
        hit: hit.map(|hit| tag(vec![hit], "doh").remove(0)),
        ips,
    }
}

async fn internetdb(ip: &str) -> Result<Vec<SearchHit>, String> {
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return Ok(Vec::new());
    };
    if ip_blocked(addr) {
        return Ok(Vec::new());
    }
    let url = format!("https://internetdb.shodan.io/{ip}");
    let raw: RawHttp = request(reqwest::Method::GET, &url, None, "application/json", &[]).await?;
    if raw.status == 404 {
        return Ok(Vec::new());
    }
    let value = super::parse_json_ok(raw)?;
    Ok(tag(parse_internetdb(&value), "internetdb"))
}

pub(crate) fn parse_internetdb(value: &Value) -> Vec<SearchHit> {
    let ip = value
        .get("ip")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if ip.is_empty() {
        return Vec::new();
    }
    let ports = join_list(value.get("ports"));
    let hosts = join_list(value.get("hostnames"));
    let tags = join_list(value.get("tags"));
    let vulns = value
        .get("vulns")
        .and_then(|v| v.as_array())
        .map(|rows| {
            let names: Vec<&str> = rows.iter().filter_map(|row| row.as_str()).take(5).collect();
            if rows.len() > names.len() {
                format!("{} ({})", rows.len(), names.join(", "))
            } else if names.is_empty() {
                String::new()
            } else {
                names.join(", ")
            }
        })
        .unwrap_or_default();
    let mut parts = Vec::new();
    if !ports.is_empty() {
        parts.push(format!("ports {ports}"));
    }
    if !hosts.is_empty() {
        parts.push(format!("hosts {hosts}"));
    }
    if !tags.is_empty() {
        parts.push(format!("tags {tags}"));
    }
    if !vulns.is_empty() {
        parts.push(format!("vulns {vulns}"));
    }
    vec![SearchHit {
        title: ip.to_string(),
        url: format!("https://internetdb.shodan.io/{ip}"),
        snippet: clip(&parts.join("; "), 360),
    }]
}

fn join_list(value: Option<&Value>) -> String {
    value
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.as_str()
                        .map(|text| text.to_string())
                        .or_else(|| row.as_i64().map(|n| n.to_string()))
                        .or_else(|| row.as_u64().map(|n| n.to_string()))
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rdap_certificate_dns_wayback_and_internetdb() {
        let rdap: Value = serde_json::from_str(
            r#"{"ldhName":"example.com","status":["active"],"nameservers":[{"ldhName":"a.iana-servers.net"}],"entities":[{"roles":["registrar"],"vcardArray":["vcard",[["fn",{},"text","Reserved"]]]}],"events":[{"eventAction":"registration","eventDate":"1995-08-14T04:00:00Z"}]}"#,
        )
        .unwrap();
        let hit = &parse_rdap("example.com", &rdap)[0];
        assert_eq!(hit.title, "example.com");
        assert!(hit.snippet.contains("Reserved"));
        assert!(hit.snippet.contains("a.iana-servers.net"));
        assert!(hit.snippet.contains("1995-08-14"));

        let crt: Value = serde_json::from_str(
            r#"[{"id":1,"issuer_name":"Let's Encrypt","common_name":"example.com","name_value":"example.com\nwww.example.com","not_before":"2026-01-01","not_after":"2026-04-01"}]"#,
        )
        .unwrap();
        let hit = &parse_crtsh(&crt)[0];
        assert_eq!(hit.url, "https://crt.sh/?id=1");
        assert!(hit.snippet.contains("www.example.com"));

        let dns: Value = serde_json::from_str(
            r#"{"Status":0,"Answer":[{"name":"example.com","type":1,"TTL":300,"data":"93.184.216.34"},{"name":"example.com","type":5,"TTL":300,"data":"example.com"}]}"#,
        )
        .unwrap();
        let answer = parse_doh("example.com", "A", &dns);
        assert_eq!(answer.ips, vec!["93.184.216.34".to_string()]);
        assert!(answer.hit.unwrap().snippet.contains("A 93.184.216.34"));

        let private: Value =
            serde_json::from_str(r#"{"Answer":[{"type":1,"data":"127.0.0.1"}]}"#).unwrap();
        assert!(parse_doh("example.com", "A", &private).ips.is_empty());

        let wayback: Value = serde_json::from_str(
            r#"[["original","timestamp","statuscode","mimetype"],["http://example.com/","20200101120000","200","text/html"]]"#,
        )
        .unwrap();
        let hit = &parse_wayback(&wayback)[0];
        assert_eq!(
            hit.url,
            "https://web.archive.org/web/20200101120000/http://example.com/"
        );

        let db: Value = serde_json::from_str(
            r#"{"ip":"93.184.216.34","ports":[80,443],"hostnames":["example.com"],"tags":["self-signed"],"vulns":["CVE-2021-1"]}"#,
        )
        .unwrap();
        let hit = &parse_internetdb(&db)[0];
        assert!(hit.snippet.contains("ports 80, 443"));
        assert!(hit.snippet.contains("CVE-2021-1"));
    }
}

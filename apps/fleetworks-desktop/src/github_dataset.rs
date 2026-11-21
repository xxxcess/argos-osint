//! Download public GitHub dataset bytes in the workstation process.

use std::time::Duration;

use fleetworks_contracts::dataset_upload::dataset_upload_limit;
use fleetworks_contracts::readiness::DatasetKind;
use reqwest::header;
use reqwest::redirect::Policy;
use reqwest::Client;
use reqwest::Response;
use reqwest::StatusCode;
use sha2::Digest;
use sha2::Sha256;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use url::Url;

const ACTORS: &str = concat!(
    "https://media.githubusercontent.com/media/xxxcess/argos-osint/",
    "cd371e39772a157bb791dc0d0dc6ac76c84e0ea5/datasets/argos_actors.csv",
);
const ACTORS_SIZE: u64 = 139_885_998;
const ACTORS_SHA256: &str = "c1866dd7b295ef1e62bd0b818d30c6eafd5c3a59f7b2c1417d22653295dbfedd";
const TRAINING: &str = concat!(
    "https://raw.githubusercontent.com/xxxcess/argos-osint/",
    "cd371e39772a157bb791dc0d0dc6ac76c84e0ea5/datasets/argos_atlas_strategic_domains_v2.csv",
);
const MAX_URL_BYTES: usize = 2_048;
const MAX_REDIRECTS: usize = 4;
const MAX_POINTER_BYTES: u64 = 1_024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const URL_ERROR: &str = "Use the public HTTPS Raw link to a GitHub CSV without a token or query.";
const REDIRECT_ERROR: &str = "This link redirected away from a supported public GitHub CSV.";
const DOWNLOAD_ERROR: &str = "GitHub download was interrupted. Check your connection and retry.";
const HTML_ERROR: &str =
    "GitHub returned a web page. Open the CSV on GitHub and copy its Raw link.";
const LIMIT_ERROR: &str = "This file exceeds the upload limit for this dataset.";
const LFS_ERROR: &str = "GitHub could not provide the matching CSV for this Git LFS file.";
const STAGING_ERROR: &str = "Could not save the downloaded CSV. Check available space and retry.";
const POINTER_HEADER: &str = "version https://git-lfs.github.com/spec/v1";

pub(crate) fn default_url(kind: DatasetKind) -> Option<&'static str> {
    match kind {
        DatasetKind::ActorCatalog => Some(ACTORS),
        DatasetKind::ClassifierTraining => Some(TRAINING),
        _ => None,
    }
}

/// Stage one complete download. The caller owns this file through its usual
/// CSV validation and upload so confirmation uses the same bytes.
pub(crate) async fn download(source: &str, kind: DatasetKind) -> Result<NamedTempFile, String> {
    let url = parse_source(source.trim()).ok_or(URL_ERROR)?;
    let limit = dataset_upload_limit(kind);
    if limit == 0 {
        return Err("This dataset does not support a GitHub CSV source.".to_owned());
    }
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(12))
        .read_timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| DOWNLOAD_ERROR)?;
    tokio::time::timeout(DOWNLOAD_TIMEOUT, fetch_csv(&client, url, limit))
        .await
        .map_err(|_| "GitHub download timed out. Retry or choose a local CSV.")?
}

fn parse_source(source: &str) -> Option<Url> {
    if !safe_url_text(source) {
        return None;
    }
    let url = Url::parse(source).ok()?;
    supported_url(&url).then_some(url)
}

fn safe_url_text(source: &str) -> bool {
    if source.len() > MAX_URL_BYTES || source.contains('\\') || source.chars().any(char::is_control)
    {
        return false;
    }
    // URL normalization can discard an empty user-info delimiter.
    source
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| source.strip_prefix("//"))
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_none_or(|authority| !authority.contains('@'))
}

fn supported_url(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > MAX_URL_BYTES
    {
        return false;
    }
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let parts: Vec<_> = segments.collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || unsafe_segment(part))
        || !parts.last().is_some_and(|filename| {
            filename.rsplit_once('.').is_some_and(|(name, suffix)| {
                !name.is_empty() && suffix.eq_ignore_ascii_case("csv")
            })
        })
    {
        return false;
    }
    let (owner, repo) = match (url.host_str(), parts.as_slice()) {
        (Some("raw.githubusercontent.com"), [owner, repo, _, _, ..]) => (owner, repo),
        (Some("github.com"), [owner, repo, "raw", _, _, ..]) => (owner, repo),
        (Some("media.githubusercontent.com"), ["media", owner, repo, _, _, ..]) => (owner, repo),
        _ => return false,
    };
    [*owner, *repo].into_iter().all(|segment| {
        segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}

fn unsafe_segment(segment: &str) -> bool {
    segment.as_bytes().windows(3).any(|window| {
        if window[0] != b'%' {
            return false;
        }
        let high = char::from(window[1]).to_digit(16);
        let low = char::from(window[2]).to_digit(16);
        match high.zip(low).map(|(high, low)| (high * 16 + low) as u8) {
            Some(byte) => byte.is_ascii_control() || matches!(byte, b'/' | b'\\'),
            None => true,
        }
    })
}

fn redirect_url(current: &Url, location: &str) -> Option<Url> {
    if location.is_empty() || !safe_url_text(location) {
        return None;
    }
    let url = current.join(location).ok()?;
    supported_url(&url).then_some(url)
}

async fn get(client: &Client, mut url: Url) -> Result<(Url, Response), String> {
    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| DOWNLOAD_ERROR)?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .ok_or(REDIRECT_ERROR)?;
            url = redirect_url(&url, location).ok_or(REDIRECT_ERROR)?;
            continue;
        }
        if response.status() != StatusCode::OK {
            return Err(format!(
                "GitHub could not download this CSV (HTTP {}). Check the public Raw link and retry.",
                response.status().as_u16(),
            ));
        }
        return Ok((url, response));
    }
    Err("GitHub redirected too many times. Copy the direct Raw link and retry.".to_owned())
}

#[derive(Debug, Eq, PartialEq)]
struct LfsPointer {
    sha256: String,
    size: u64,
}

fn lfs_pointer(bytes: &[u8]) -> Result<Option<LfsPointer>, &'static str> {
    if !bytes.starts_with(POINTER_HEADER.as_bytes()) {
        return Ok(None);
    }
    let mut lines = std::str::from_utf8(bytes).map_err(|_| LFS_ERROR)?.lines();
    if lines.next() != Some(POINTER_HEADER) {
        return Err(LFS_ERROR);
    }
    let sha256 = lines
        .next()
        .and_then(|line| line.strip_prefix("oid sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or(LFS_ERROR)?;
    let size = lines
        .next()
        .and_then(|line| line.strip_prefix("size "))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size > 0)
        .ok_or(LFS_ERROR)?;
    if lines.next().is_some() {
        return Err(LFS_ERROR);
    }
    Ok(Some(LfsPointer {
        sha256: sha256.to_ascii_lowercase(),
        size,
    }))
}

fn media_url(raw: &Url) -> Option<Url> {
    let media = match raw.host_str()? {
        "raw.githubusercontent.com" => {
            format!("https://media.githubusercontent.com/media{}", raw.path())
        }
        "github.com" => {
            let mut parts = raw.path_segments()?;
            let owner = parts.next()?;
            let repo = parts.next()?;
            if parts.next()? != "raw" {
                return None;
            }
            let path = parts.collect::<Vec<_>>().join("/");
            format!("https://media.githubusercontent.com/media/{owner}/{repo}/{path}")
        }
        _ => return None,
    };
    parse_source(&media)
}

enum Staged {
    Csv(NamedTempFile),
    Lfs(LfsPointer),
}

async fn fetch_csv(client: &Client, url: Url, limit: u64) -> Result<NamedTempFile, String> {
    let expected = (url.as_str() == ACTORS).then(|| LfsPointer {
        sha256: ACTORS_SHA256.to_owned(),
        size: ACTORS_SIZE,
    });
    let (resolved, response) = get(client, url).await?;
    match stage(response, limit, expected.as_ref()).await? {
        Staged::Csv(csv) => Ok(csv),
        Staged::Lfs(pointer) => {
            if pointer.size > limit {
                return Err(LIMIT_ERROR.to_owned());
            }
            let media = media_url(&resolved).ok_or(LFS_ERROR)?;
            let (_, response) = get(client, media).await?;
            match stage(response, limit, Some(&pointer)).await? {
                Staged::Csv(csv) => Ok(csv),
                Staged::Lfs(_) => Err(LFS_ERROR.to_owned()),
            }
        }
    }
}

async fn stage(
    mut response: Response,
    limit: u64,
    expected: Option<&LfsPointer>,
) -> Result<Staged, String> {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type.is_some_and(|media| {
        media.eq_ignore_ascii_case("text/html")
            || media.eq_ignore_ascii_case("application/xhtml+xml")
    }) {
        return Err(HTML_ERROR.to_owned());
    }
    if expected.is_some_and(|pointer| pointer.size > limit)
        || response.content_length().is_some_and(|size| size > limit)
    {
        return Err(LIMIT_ERROR.to_owned());
    }
    let (staged, output) = tokio::task::spawn_blocking(|| {
        let staged = tempfile::Builder::new()
            .prefix("fleetworks-github-")
            .suffix(".csv")
            .tempfile()?;
        let output = staged.reopen()?;
        Ok::<_, std::io::Error>((staged, output))
    })
    .await
    .map_err(|_| STAGING_ERROR)?
    .map_err(|_| STAGING_ERROR)?;
    let mut output = tokio::fs::File::from_std(output);
    let mut total = 0_u64;
    let mut pointer_bytes = Vec::new();
    let mut checksum = expected.map(|_| Sha256::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| DOWNLOAD_ERROR)? {
        total = total.saturating_add(chunk.len() as u64);
        if total > limit {
            return Err(LIMIT_ERROR.to_owned());
        }
        if let Some(checksum) = &mut checksum {
            checksum.update(&chunk);
        } else if total <= MAX_POINTER_BYTES {
            pointer_bytes.extend_from_slice(&chunk);
        }
        output.write_all(&chunk).await.map_err(|_| STAGING_ERROR)?;
    }
    if total == 0 {
        return Err("The GitHub CSV download was empty. Check the Raw link and retry.".to_owned());
    }
    output.flush().await.map_err(|_| STAGING_ERROR)?;
    if let Some((checksum, expected)) = checksum.zip(expected) {
        if total != expected.size || format!("{:x}", checksum.finalize()) != expected.sha256 {
            return Err(LFS_ERROR.to_owned());
        }
    } else if total <= MAX_POINTER_BYTES {
        if let Some(pointer) = lfs_pointer(&pointer_bytes)? {
            return Ok(Staged::Lfs(pointer));
        }
    }
    Ok(Staged::Csv(staged))
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn accepts_public_raw_branch_commit_and_lfs_urls() {
        for source in [
            ACTORS,
            TRAINING,
            "https://raw.githubusercontent.com/example/repo/main/Actor.CSV",
            "https://raw.githubusercontent.com/example/repo/refs/heads/feature/data/datasets/actors.csv",
            "https://github.com/example/repo/raw/main/datasets/data%20set.csv",
            "https://github.com/example/repo/raw/0123456789abcdef/actors.csv",
        ] {
            assert!(parse_source(source).is_some(), "{source}");
        }
    }

    #[test]
    fn rejects_web_pages_credentials_queries_ports_and_untrusted_hosts() {
        for source in [
            "https://github.com/example/repo/blob/main/datasets/actors.csv",
            "http://raw.githubusercontent.com/example/repo/main/actors.csv",
            "https://github.com.evil.test/example/repo/raw/main/actors.csv",
            "https://raw.githubusercontent.com.evil.test/example/repo/main/actors.csv",
            "https://media.githubusercontent.com/example/repo/main/actors.csv",
            "https://raw.githubusercontent.com:8443/example/repo/main/actors.csv",
            "https://token@raw.githubusercontent.com/example/repo/main/actors.csv",
            "https://@raw.githubusercontent.com/example/repo/main/actors.csv",
            "https://raw.githubusercontent.com/example/repo/main/actors.csv?token=secret",
            "https://raw.githubusercontent.com/example/repo/main/actors.csv#download",
            "https://raw.githubusercontent.com/example/repo/main/%2factors.csv",
            "https://raw.githubusercontent.com/example/repo/main/%5Cactors.csv",
            "https://raw.githubusercontent.com/example/repo/main/%00actors.csv",
            "https://raw.githubusercontent.com/example/repo/main/a.html",
        ] {
            assert!(parse_source(source).is_none(), "{source}");
        }
    }

    #[test]
    fn revalidates_each_redirect_and_pins_media_fallback_to_the_same_path() {
        let source = parse_source("https://github.com/example/repo/raw/main/actors.csv").unwrap();
        let raw = redirect_url(
            &source,
            "https://raw.githubusercontent.com/example/repo/refs/heads/main/datasets/actors.csv",
        )
        .unwrap();
        assert_eq!(
            media_url(&raw).unwrap().as_str(),
            "https://media.githubusercontent.com/media/example/repo/refs/heads/main/datasets/actors.csv"
        );
        assert_eq!(
            media_url(&source).unwrap().as_str(),
            "https://media.githubusercontent.com/media/example/repo/main/actors.csv"
        );
        for location in [
            "//127.0.0.1/private.csv",
            "https://example.test/actors.csv",
            "https://github.com/example/repo/blob/main/actors.csv",
            "https://raw.githubusercontent.com/example/repo/main/actors.csv?token=x",
        ] {
            assert!(redirect_url(&source, location).is_none(), "{location}");
        }
    }

    async fn response(wire: &[u8]) -> Response {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let client = Client::builder().no_proxy().build().unwrap();
        let serve = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2_048];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(wire).await.unwrap();
            socket.shutdown().await.unwrap();
        };
        let (response, ()) = tokio::join!(client.get(origin).send(), serve);
        response.unwrap()
    }

    #[tokio::test]
    async fn stages_exact_bytes_and_verifies_a_git_lfs_payload() {
        let bytes = b"name\r\nexample\r\n";
        let expected = LfsPointer {
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        let pointer_text = format!(
            "{POINTER_HEADER}\noid sha256:{}\nsize {}\n",
            expected.sha256, expected.size,
        );
        assert_eq!(
            lfs_pointer(pointer_text.as_bytes()).unwrap().as_ref(),
            Some(&expected)
        );
        assert!(lfs_pointer(POINTER_HEADER.as_bytes()).is_err());
        let mut wire =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bytes.len()).into_bytes();
        wire.extend_from_slice(bytes);
        let Staged::Csv(file) = stage(response(&wire).await, 64, Some(&expected))
            .await
            .unwrap()
        else {
            panic!("verified Git LFS content must leave a staged CSV");
        };
        assert_eq!(std::fs::read(file.path()).unwrap(), bytes);
        let wrong = LfsPointer {
            sha256: "0".repeat(64),
            ..expected
        };
        assert_eq!(
            stage(response(&wire).await, 64, Some(&wrong))
                .await
                .err()
                .as_deref(),
            Some(LFS_ERROR)
        );
    }

    #[tokio::test]
    async fn rejects_html_oversized_and_interrupted_downloads_before_using_the_file() {
        for (wire, limit, error) in [
            (b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 6\r\n\r\n<html>".as_slice(), 16, HTML_ERROR),
            (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n12345\r\n0\r\n\r\n".as_slice(), 4, LIMIT_ERROR),
            (b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n123".as_slice(), 16, DOWNLOAD_ERROR),
        ] {
            assert_eq!(stage(response(wire).await, limit, None).await.err().as_deref(), Some(error));
        }
    }
}

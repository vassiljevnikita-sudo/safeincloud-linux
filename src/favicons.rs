//! Bounded HTTPS favicon fetching. Run `fetch` on a worker, never the GTK thread.
//! No account data, cookies, saved URL paths, or query strings are sent.
use anyhow::{bail, Result};
use reqwest::{
    blocking::Client,
    header::{CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use scraper::{Html, Selector};
use std::{
    collections::HashSet,
    io::Read,
    time::{Duration, Instant},
};
use url::{Host, Url};

pub const MAX_BYTES: usize = 512 * 1024;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_REDIRECTS: usize = 2;
const MAX_CANDIDATES: usize = 8;

/// Canonical HTTPS origin only, discarding saved credentials, path, query, fragment.
/// HTTP entries are upgraded to HTTPS; local placeholder hostnames are ignored.
pub fn website_origin(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let input = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url = Url::parse(&input).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host()?;
    if let Host::Domain(host) = host {
        let host = host.trim_end_matches('.');
        if host == "localhost"
            || [".test", ".invalid", ".localhost"]
                .iter()
                .any(|suffix| host.ends_with(suffix))
        {
            return None;
        }
    }
    let port = url.port().filter(|port| !matches!(*port, 80 | 443));
    Some(format!(
        "https://{host}{}",
        port.map(|p| format!(":{p}")).unwrap_or_default()
    ))
}

fn safe_url(mut url: Url) -> Option<Url> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || website_origin(url.as_str()).is_none()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

#[derive(Debug, Clone)]
struct Response {
    status: u16,
    location: Option<String>,
    content_type: String,
    body: Vec<u8>,
}

// Small transport seam permits complete workflow tests without network contacts.
trait Transport {
    fn get(&mut self, url: &Url, timeout: Duration) -> Result<Response>;
}
struct HttpTransport {
    client: Client,
}
impl Transport for HttpTransport {
    fn get(&mut self, url: &Url, timeout: Duration) -> Result<Response> {
        let mut response = self
            .client
            .get(url.clone())
            .timeout(timeout)
            .send()
            .map_err(|_| anyhow::anyhow!("Das Website-Symbol konnte nicht geladen werden."))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Redirect bodies and error pages need not be downloaded.
        if !(200..300).contains(&status) {
            return Ok(Response {
                status,
                location,
                content_type,
                body: Vec::new(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            bail!("Das Website-Symbol überschreitet die Größenbegrenzung.");
        }
        let mut body = Vec::new();
        response
            .by_ref()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| anyhow::anyhow!("Das Website-Symbol konnte nicht geladen werden."))?;
        if body.len() > MAX_BYTES {
            bail!("Das Website-Symbol überschreitet die Größenbegrenzung.");
        }
        Ok(Response {
            status,
            location,
            content_type,
            body,
        })
    }
}

fn request(
    transport: &mut impl Transport,
    url: Url,
    deadline: Instant,
    redirects: &mut usize,
) -> Result<(Url, Response)> {
    let mut current = safe_url(url).ok_or_else(|| anyhow::anyhow!("Ungültige Symbol-Adresse."))?;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| anyhow::anyhow!("Zeitlimit beim Laden des Website-Symbols."))?;
        let response = transport.get(&current, remaining)?;
        if Instant::now() >= deadline {
            bail!("Zeitlimit beim Laden des Website-Symbols.");
        }
        if (300..400).contains(&response.status) {
            if *redirects >= MAX_REDIRECTS {
                bail!("Zu viele Weiterleitungen für das Website-Symbol.");
            }
            let target = response
                .location
                .as_deref()
                .and_then(|s| current.join(s).ok())
                .and_then(safe_url)
                .ok_or_else(|| anyhow::anyhow!("Ungültige Symbol-Weiterleitung."))?;
            *redirects += 1;
            current = target;
        } else if (200..300).contains(&response.status) && response.body.len() <= MAX_BYTES {
            return Ok((current, response));
        } else {
            bail!("Das Website-Symbol ist nicht verfügbar.");
        }
    }
}

fn raster(response: &Response) -> bool {
    if response.content_type.contains("svg") {
        return false;
    }
    let Ok(mut reader) =
        image::ImageReader::new(std::io::Cursor::new(&response.body)).with_guessed_format()
    else {
        return false;
    };
    if !matches!(
        reader.format(),
        Some(
            image::ImageFormat::Png
                | image::ImageFormat::Ico
                | image::ImageFormat::Jpeg
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
        )
    ) {
        return false;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().is_ok()
}

fn icon_links(html: &[u8], base: &Url) -> Vec<Url> {
    let document = Html::parse_document(&String::from_utf8_lossy(html));
    let selector = Selector::parse("link[href]").expect("constant selector");
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for element in document.select(&selector) {
        let element = element.value();
        let rel = element.attr("rel").unwrap_or_default();
        if !rel.split_ascii_whitespace().any(|part| {
            part.eq_ignore_ascii_case("icon") || part.eq_ignore_ascii_case("apple-touch-icon")
        }) {
            continue;
        }
        let kind = element.attr("type").unwrap_or_default();
        if kind.to_ascii_lowercase().contains("svg") {
            continue;
        }
        let Some(url) = element
            .attr("href")
            .and_then(|href| base.join(href).ok())
            .and_then(safe_url)
        else {
            continue;
        };
        if url.path().to_ascii_lowercase().ends_with(".svg") {
            continue;
        }
        if seen.insert(url.as_str().to_owned()) {
            output.push(url);
        }
        if output.len() == MAX_CANDIDATES {
            break;
        }
    }
    output
}

/// Fetch bytes suitable for raster decoding, with a six-second whole-job budget.
/// Raster decoding is checked with dimension and allocation limits. All errors
/// are sanitized, and neither errors nor caches retain the input URL.
pub fn fetch(origin: &str) -> Result<Vec<u8>> {
    let deadline = Instant::now() + TOTAL_TIMEOUT;
    let origin =
        website_origin(origin).ok_or_else(|| anyhow::anyhow!("Ungültige Website-Adresse."))?;
    let client = Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .referer(false)
        .no_proxy()
        .timeout(TOTAL_TIMEOUT)
        .connect_timeout(TOTAL_TIMEOUT)
        .pool_max_idle_per_host(0)
        .user_agent("SafeInCloud-Linux")
        .build()
        .map_err(|_| anyhow::anyhow!("Der Symbolabruf konnte nicht gestartet werden."))?;
    fetch_with(&origin, &mut HttpTransport { client }, deadline)
}

fn fetch_with(origin: &str, transport: &mut impl Transport, deadline: Instant) -> Result<Vec<u8>> {
    let base = Url::parse(origin).map_err(|_| anyhow::anyhow!("Ungültige Website-Adresse."))?;
    let mut redirects = 0;
    if let Ok((_, response)) = request(
        transport,
        base.join("/favicon.ico")?,
        deadline,
        &mut redirects,
    ) {
        if raster(&response) && Instant::now() < deadline {
            return Ok(response.body);
        }
    }
    let (page_url, response) = request(transport, base.join("/")?, deadline, &mut redirects)?;
    for candidate in icon_links(&response.body, &page_url) {
        if let Ok((_, response)) = request(transport, candidate, deadline, &mut redirects) {
            if raster(&response) && Instant::now() < deadline {
                return Ok(response.body);
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    bail!("Kein verwendbares Website-Symbol gefunden.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        replies: VecDeque<Response>,
        urls: Vec<String>,
    }
    impl Transport for Fake {
        fn get(&mut self, url: &Url, timeout: Duration) -> Result<Response> {
            assert!(timeout <= TOTAL_TIMEOUT);
            self.urls.push(url.to_string());
            self.replies
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("Keine Testantwort."))
        }
    }
    fn response(status: u16, content_type: &str, body: &[u8]) -> Response {
        Response {
            status,
            location: None,
            content_type: content_type.into(),
            body: body.into(),
        }
    }
    fn run(replies: Vec<Response>) -> (Result<Vec<u8>>, Fake) {
        let mut fake = Fake {
            replies: replies.into(),
            urls: Vec::new(),
        };
        let result = fetch_with(
            "https://example.com",
            &mut fake,
            Instant::now() + TOTAL_TIMEOUT,
        );
        (result, fake)
    }
    #[test]
    fn origins_discard_every_saved_secret_component() {
        assert_eq!(
            website_origin("http://alice:secret@Example.COM/private?a=secret#secret"),
            Some("https://example.com".into())
        );
        assert_eq!(
            website_origin("example.com:8443/private"),
            Some("https://example.com:8443".into())
        );
        assert_eq!(
            website_origin("https://[2001:db8::1]/private"),
            Some("https://[2001:db8::1]".into())
        );
        assert_eq!(
            website_origin("https://bücher.example/a"),
            Some("https://xn--bcher-kva.example".into())
        );
        for invalid in [
            "",
            "file:///tmp/a",
            "javascript:alert(1)",
            "https://local.test/",
            "https://localhost",
            "https://a b.com",
            "https://example.com:99999",
        ] {
            assert_eq!(website_origin(invalid), None, "{invalid}");
        }
    }
    #[test]
    fn html_filters_svg_unsafe_links_and_supports_cdn() {
        let links = icon_links(br#"<link rel="icon" href="a.svg"><link rel="icon" type="image/svg+xml" href="/vector"><link rel="ICON" href="https://cdn.example.com/icon.png"><link rel="apple-touch-icon" href="/touch.png"><link rel="icon" href="http://bad.example/a.png"><link rel="icon" href="https://u:p@bad.example/a.png"><link rel="icon" href="/a.png?secret=1">"#,
            &Url::parse("https://example.com/").unwrap());
        assert_eq!(
            links.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec![
                "https://cdn.example.com/icon.png",
                "https://example.com/touch.png"
            ]
        );
    }
    #[test]
    fn svg_response_does_not_hide_later_raster_fallback() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        let png = buffer.into_inner();
        let (result, fake) = run(vec![response(404, "", b""),
            response(200, "text/html", br#"<link rel="icon" href="/not-obviously-svg"><link rel="icon" href="https://cdn.example.com/icon.png">"#),
            response(200, "image/svg+xml", b"<svg/>"), response(200, "image/png", &png)]);
        assert_eq!(result.unwrap(), png);
        assert_eq!(
            fake.urls,
            vec![
                "https://example.com/favicon.ico",
                "https://example.com/",
                "https://example.com/not-obviously-svg",
                "https://cdn.example.com/icon.png"
            ]
        );
    }
    #[test]
    fn invalid_or_oversized_rasters_are_not_usable() {
        assert!(!raster(&response(
            200,
            "image/png",
            b"\x89PNG\r\n\x1a\ninvalid"
        )));
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1025, 1)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        assert!(!raster(&response(200, "image/png", &buffer.into_inner())));
        assert!(!raster(&response(
            200,
            "image/png",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>"
        )));
    }
    #[test]
    fn redirects_are_https_only_and_globally_bounded() {
        let mut redirect = response(302, "", b"");
        redirect.location = Some("/again".into());
        let (result, fake) = run(vec![
            redirect.clone(),
            redirect.clone(),
            redirect.clone(),
            redirect,
        ]);
        assert!(result.is_err());
        assert_eq!(fake.urls.len(), 4);
        let mut redirect = response(302, "", b"");
        redirect.location = Some("http://example.com/icon.png".into());
        let (_, fake) = run(vec![redirect, response(404, "", b"")]);
        assert_eq!(
            fake.urls,
            vec!["https://example.com/favicon.ico", "https://example.com/"]
        );
    }
    #[test]
    fn byte_and_candidate_bounds_and_deadline() {
        let (result, _) = run(vec![
            response(200, "image/png", &vec![0; MAX_BYTES + 1]),
            response(404, "", b""),
        ]);
        assert!(result.is_err());
        let html = (0..100)
            .map(|i| format!("<link rel='icon' href='/{i}.png'>"))
            .collect::<String>();
        assert_eq!(
            icon_links(
                html.as_bytes(),
                &Url::parse("https://example.com/").unwrap()
            )
            .len(),
            MAX_CANDIDATES
        );
        let mut fake = Fake {
            replies: VecDeque::new(),
            urls: Vec::new(),
        };
        assert!(fetch_with("https://example.com", &mut fake, Instant::now()).is_err());
        assert!(fake.urls.is_empty());
    }
}

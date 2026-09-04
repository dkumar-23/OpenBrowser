use super::{BrowserError, NavigationResult, ObservedCookie, NavigationPolicy};
use runtime_network::HttpClient;
use runtime_dom::HtmlParser;


pub(super) async fn do_navigate(
    client: &HttpClient,
    url: &str,
) -> Result<NavigationResult, BrowserError> {
    let req = runtime_network::Request::get(url);
    let resp = client.execute(req).await.map_err(|e| BrowserError::Navigation(e.to_string()))?;
    let body = resp.text().map_err(|e| BrowserError::Navigation(e.to_string()))?;
    let root = HtmlParser::parse(&body).map_err(|e| BrowserError::Navigation(e.to_string()))?;
    let dom = runtime_dom::DomTree::new(root);
    let cookies = extract_cookies(&resp);
    Ok(NavigationResult {
        dom,
        cookies,
        final_url: url.to_string(),
        status: resp.status(),
    })
}

fn extract_cookies(resp: &runtime_network::Response) -> Vec<ObservedCookie> {
    let mut out = Vec::new();
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("set-cookie") {
            if let Some(c) = parse_cookie(v) { out.push(c); }
        }
    }
    out
}

fn parse_cookie(header: &str) -> Option<ObservedCookie> {
    let mut parts = header.split(';');
    let first = parts.next()?;
    let eq = first.find('=')?;
    let mut c = ObservedCookie {
        name: first[..eq].trim().to_string(),
        value: first[eq+1..].trim().to_string(),
        domain: None, path: None, secure: false, http_only: false,
    };
    for p in parts {
        let p = p.trim();
        if p.eq_ignore_ascii_case("Secure") { c.secure = true; }
        else if p.eq_ignore_ascii_case("HttpOnly") { c.http_only = true; }
        else if let Some(d) = p.strip_prefix("Domain=") { c.domain = Some(d.trim().to_string()); }
        else if let Some(pp) = p.strip_prefix("Path=") { c.path = Some(pp.trim().to_string()); }
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn policy_default() { let _ = NavigationPolicy::default(); }
    #[test] fn cookie_parse() { assert!(parse_cookie("a=b; Path=/").is_some()); }
    #[test] fn cookie_parse_minimal() { assert!(parse_cookie("x=y").is_some()); }
}

//! Input checks for socket API methods that hand a URL to the browser.

/// Normalize a URL for `browser.open` and browser navigation.
///
/// Bare hosts (`example.com`, `localhost:3000`) get `https://` prepended, as before.
/// Only `http`, `https` and `about:blank` are accepted; `javascript:`, `file:`,
/// `data:`, `chrome:` and any other scheme are rejected.
pub fn normalize_browser_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("url is required".into());
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("url must not contain whitespace or control characters".into());
    }
    if url.eq_ignore_ascii_case("about:blank") {
        return Ok("about:blank".into());
    }
    match url_scheme(url) {
        Some(scheme) if scheme == "http" || scheme == "https" => Ok(url.to_string()),
        Some(scheme) => Err(format!(
            "unsupported URL scheme '{scheme}': only http, https and about:blank are allowed"
        )),
        None => Ok(format!("https://{url}")),
    }
}

/// Lowercased scheme of `url`, or None when the text looks like a bare `host[:port]`.
fn url_scheme(url: &str) -> Option<String> {
    let (candidate, rest) = url.split_once(':')?;
    let mut chars = candidate.chars();
    let valid_name = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_name {
        return None;
    }
    // "localhost:3000" and "example.com:8080/path" are host:port, not schemes.
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    let port_like = digits > 0
        && rest[digits..]
            .chars()
            .next()
            .map_or(true, |c| matches!(c, '/' | '?' | '#'));
    if port_like {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hosts_get_https() {
        assert_eq!(normalize_browser_url("example.com").unwrap(), "https://example.com");
        assert_eq!(normalize_browser_url("localhost:3000").unwrap(), "https://localhost:3000");
        assert_eq!(
            normalize_browser_url("example.com:8080/path").unwrap(),
            "https://example.com:8080/path"
        );
    }

    #[test]
    fn http_https_and_about_blank_pass_through() {
        assert_eq!(normalize_browser_url("https://example.com/a?b=1").unwrap(), "https://example.com/a?b=1");
        assert_eq!(normalize_browser_url("HTTP://example.com").unwrap(), "HTTP://example.com");
        assert_eq!(normalize_browser_url("about:blank").unwrap(), "about:blank");
        assert_eq!(normalize_browser_url("  https://example.com  ").unwrap(), "https://example.com");
    }

    #[test]
    fn dangerous_schemes_are_rejected() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "chrome://settings",
            "ws://127.0.0.1:9222",
            "ftp://example.com",
            "about:srcdoc",
        ] {
            assert!(normalize_browser_url(url).is_err(), "should reject {url:?}");
        }
    }

    #[test]
    fn empty_and_whitespace_urls_are_rejected() {
        assert!(normalize_browser_url("").is_err());
        assert!(normalize_browser_url("   ").is_err());
        assert!(normalize_browser_url("https://example.com\nHost: evil").is_err());
        assert!(normalize_browser_url("https://exa mple.com").is_err());
    }
}

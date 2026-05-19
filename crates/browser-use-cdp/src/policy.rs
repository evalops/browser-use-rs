use crate::{BrowserError, BrowserProfile};
use percent_encoding::percent_decode_str;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UrlAccessPolicy {
    allowed_domains: Vec<String>,
    prohibited_domains: Vec<String>,
    block_ip_addresses: bool,
}

impl UrlAccessPolicy {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            allowed_domains: profile.allowed_domains.clone(),
            prohibited_domains: profile.prohibited_domains.clone(),
            block_ip_addresses: profile.block_ip_addresses,
        }
    }

    pub(crate) fn validate(&self, url: &str) -> Result<(), BrowserError> {
        if self.is_allowed(url) {
            return Ok(());
        }

        Err(BrowserError::NavigationBlocked {
            url: url.to_owned(),
            reason: self.block_reason(url).to_owned(),
        })
    }

    pub(crate) fn is_unrestricted(&self) -> bool {
        self.allowed_domains.is_empty()
            && self.prohibited_domains.is_empty()
            && !self.block_ip_addresses
    }

    pub(crate) fn is_allowed(&self, url: &str) -> bool {
        if is_internal_browser_url(url) {
            return true;
        }

        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };

        if matches!(parsed.scheme(), "data" | "blob") {
            return true;
        }

        let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };

        if self.block_ip_addresses && is_ip_address(&host) {
            return false;
        }

        if self.allowed_domains.is_empty() && self.prohibited_domains.is_empty() {
            return true;
        }

        if !self.allowed_domains.is_empty() {
            return self
                .allowed_domains
                .iter()
                .any(|pattern| is_url_pattern_match(url, &host, parsed.scheme(), pattern));
        }

        !self
            .prohibited_domains
            .iter()
            .any(|pattern| is_url_pattern_match(url, &host, parsed.scheme(), pattern))
    }

    pub(crate) fn block_reason(&self, url: &str) -> &'static str {
        let Ok(parsed) = url::Url::parse(url) else {
            return "invalid_url";
        };
        if self.block_ip_addresses
            && parsed
                .host_str()
                .map(str::to_ascii_lowercase)
                .is_some_and(|host| is_ip_address(&host))
        {
            return "ip_address_blocked";
        }
        if !self.allowed_domains.is_empty() {
            return "not_in_allowed_domains";
        }
        "in_prohibited_domains"
    }
}

fn is_internal_browser_url(url: &str) -> bool {
    matches!(
        url,
        "about:blank"
            | "chrome://new-tab-page/"
            | "chrome://new-tab-page"
            | "chrome://newtab/"
            | "chrome://newtab"
    )
}

pub(crate) fn is_ip_address(host: &str) -> bool {
    let canonical_host = canonical_ip_host(host);
    canonical_host.parse::<std::net::IpAddr>().is_ok()
        || parse_non_standard_ipv4(&canonical_host).is_some()
}

fn canonical_ip_host(host: &str) -> String {
    percent_decode_str(host.trim_matches(['[', ']']))
        .decode_utf8_lossy()
        .nfkc()
        .collect::<String>()
        .replace(['\u{3002}', '\u{ff61}'], ".")
}

fn parse_non_standard_ipv4(host: &str) -> Option<u32> {
    if host.is_empty()
        || host.contains(':')
        || host.contains('/')
        || host.chars().any(char::is_whitespace)
    {
        return None;
    }
    let parts = host
        .split('.')
        .map(parse_non_standard_ipv4_part)
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [a] if *a <= u32::MAX as u64 => Some(*a as u32),
        [a, b] if *a <= 0xff && *b <= 0x00ff_ffff => Some(((*a as u32) << 24) | (*b as u32)),
        [a, b, c] if *a <= 0xff && *b <= 0xff && *c <= 0xffff => {
            Some(((*a as u32) << 24) | ((*b as u32) << 16) | (*c as u32))
        }
        [a, b, c, d] if *a <= 0xff && *b <= 0xff && *c <= 0xff && *d <= 0xff => {
            Some(((*a as u32) << 24) | ((*b as u32) << 16) | ((*c as u32) << 8) | (*d as u32))
        }
        _ => None,
    }
}

fn parse_non_standard_ipv4_part(part: &str) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (radix, digits) =
        if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            (16, hex)
        } else if part.len() > 1 && part.starts_with('0') {
            (8, &part[1..])
        } else {
            (10, part)
        };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_digit(radix)) {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
}

fn is_url_pattern_match(url: &str, host: &str, scheme: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }

    let url = url.to_ascii_lowercase();
    let full_url_pattern = format!("{scheme}://{host}");

    if pattern.contains('*') {
        if let Some(domain) = pattern.strip_prefix("*.") {
            return matches!(scheme, "http" | "https")
                && (host == domain || host.ends_with(&format!(".{domain}")));
        }

        if pattern.ends_with("/*") && glob_match(&url, &pattern) {
            return true;
        }

        let value = if pattern.contains("://") {
            full_url_pattern.as_str()
        } else {
            host
        };
        return glob_match(value, &pattern);
    }

    if pattern.contains("://") {
        return url.starts_with(&pattern);
    }

    host == pattern || (is_root_domain(&pattern) && host == format!("www.{pattern}"))
}

fn is_root_domain(domain: &str) -> bool {
    !domain.contains('*') && !domain.contains("://") && domain.matches('.').count() == 1
}

fn glob_match(value: &str, pattern: &str) -> bool {
    let mut remaining = value;
    let mut parts = pattern.split('*').peekable();
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');

    if let Some(first) = parts.next() {
        if anchored_start {
            let Some(rest) = remaining.strip_prefix(first) else {
                return false;
            };
            remaining = rest;
        } else if !first.is_empty() {
            let Some(index) = remaining.find(first) else {
                return false;
            };
            remaining = &remaining[index + first.len()..];
        }
    }

    while let Some(part) = parts.next() {
        if part.is_empty() {
            continue;
        }
        let Some(index) = remaining.find(part) else {
            return false;
        };
        remaining = &remaining[index + part.len()..];
        if parts.peek().is_none() && anchored_end {
            return remaining.is_empty();
        }
    }

    !anchored_end || remaining.is_empty()
}

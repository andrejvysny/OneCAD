//! The local-provider gateway — ADR-0017, `docs/assistant/wire-protocol.md` §5.
//!
//! The sidecar holds the chat loop; it does not hold an endpoint. What crosses
//! the bridge on `provider.fetch` is a **registered provider id and an operation
//! path** (`assistant-host/src/bridge/verbs.ts`, `createGatewayFetch`), and this
//! module is what turns that pair into a URL. The id is looked up in a registry
//! Rust populated from OneCAD settings; the base was validated when it was
//! registered; the path is appended to it. There is no code path here by which a
//! caller names a host.
//!
//! **What this is not.** This is application policy over code we ship. It
//! constrains a cooperating sidecar and a misbehaving model. It does **not**
//! sandbox anything: a compromised child process can open its own socket and
//! nothing in this file would know. ADR-0017 says so in as many words, and no
//! name or comment in this module may imply otherwise. The honest proof that
//! inference is local is an offline acceptance run with non-loopback networking
//! disabled — a loopback URL is not evidence, because a local server can proxy a
//! cloud model and no amount of URL validation can tell.
//!
//! Three things carry the weight:
//!
//! 1. [`validate_provider_base`] runs **once, at registration**, and reduces a
//!    configured base to a literal address, a port and a path prefix. The
//!    per-request check is then against that canonical form rather than against a
//!    name that could resolve somewhere else the second time it is looked up.
//! 2. The HTTP client is built with redirects off and proxies off, for the
//!    reasons given at [`ProviderGateway::build_client`].
//! 3. [`ProviderOperation`] is an allowlist of the two calls a chat loop makes.
//!    Locality is not enough on its own: a local inference server's HTTP surface
//!    is much larger than those two calls, and its model-management and admin
//!    routes live inside the very same loopback origin. Method, path, headers,
//!    content type, JSON shape and **model identity** are checked per operation,
//!    and anything not on the list is refused by not being on it.
//! 4. Everything is bounded — request body, response body, total time — and a
//!    cancelled request drops the upstream response rather than leaving it
//!    running (the bridge owns that half; see `bridge::serve_provider_fetch`).

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};
use reqwest::{Method, Url};
use serde::Deserialize;
use serde_json::{json, Value};

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Why the gateway refused. Every rejection names its reason, because "refused"
/// with no cause is indistinguishable from a bug in the caller.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// A configured base that is not a URL at all.
    #[error("provider base {base:?} is not a URL: {detail}")]
    NotAUrl { base: String, detail: String },
    /// A scheme other than `http` or `https`.
    #[error("provider base {base:?} uses scheme {scheme:?}; only http and https are accepted")]
    Scheme { base: String, scheme: String },
    /// Userinfo in the authority. `http://127.0.0.1@evil.com/` has host
    /// `evil.com`, and a reader who stops at the first dotted quad reads it the
    /// other way round.
    #[error("provider base {base:?} carries userinfo in its authority")]
    Userinfo { base: String },
    /// A query or fragment in the *base*. The base is a prefix, not a request.
    #[error("provider base {base:?} carries a query or fragment")]
    QueryOrFragment { base: String },
    /// The host is not one of the accepted literal loopback forms.
    #[error("provider base host {host:?} is refused: {reason}")]
    Host { host: String, reason: &'static str },
    /// The base path prefix is not usable as a prefix.
    #[error("provider base {base:?} has an unusable path prefix: {reason}")]
    BasePath { base: String, reason: &'static str },
    /// An `https` base on a build whose HTTP client has no TLS backend.
    #[error(
        "provider {id:?} is configured with an https base ({base:?}), and this build's gateway \
         client ships no TLS backend; loopback inference servers are configured over http"
    )]
    TlsNotBuilt { id: String, base: String },
    /// Two registry entries claim the same id.
    #[error("provider id {id:?} is registered twice")]
    DuplicateProvider { id: String },
    /// A registry entry with no id, or no model.
    #[error("provider entry is incomplete: {reason}")]
    IncompleteProvider { reason: &'static str },
    /// The HTTP client could not be built.
    #[error("the provider gateway's HTTP client could not be built: {0}")]
    ClientBuild(String),

    /// The `provider.fetch` payload is not the shape §5 defines — including a
    /// payload carrying an extra field such as a URL or a base override.
    #[error("provider.fetch payload is not the shape this gateway accepts: {detail}")]
    BadPayload { detail: String },
    /// A provider id that is not in the registry.
    #[error("no provider {id:?} is registered (registered: {registered})")]
    UnknownProvider { id: String, registered: String },
    /// The operation path is not usable.
    #[error("provider.fetch path {path:?} is refused: {reason}")]
    BadPath { path: String, reason: &'static str },
    /// A header the sidecar may not set, or one that is not a legal header.
    #[error("provider.fetch header {name:?} is refused: {reason}")]
    BadHeader { name: String, reason: &'static str },
    /// A method that is not an HTTP token.
    #[error("provider.fetch method {method:?} is not a valid HTTP method")]
    BadMethod { method: String },
    /// A method/path pair that is not one of the supported [`ProviderOperation`]s.
    #[error(
        "provider.fetch {method} {path} is not a supported provider operation \
         (supported: {supported})"
    )]
    UnsupportedOperation {
        method: String,
        path: String,
        supported: String,
    },
    /// A supported operation reached by the wrong method.
    #[error("provider operation {operation} is {allowed} only; got {method}")]
    MethodNotAllowed {
        operation: &'static str,
        allowed: &'static str,
        method: String,
    },
    /// A content type the operation does not accept.
    #[error("provider operation {operation} does not accept content type {got:?}")]
    BadContentType {
        operation: &'static str,
        got: String,
    },
    /// A request body that is not the shape the operation defines.
    #[error("provider operation {operation} body is refused: {reason}")]
    BadBodyShape {
        operation: &'static str,
        reason: String,
    },
    /// A body naming a model other than the one this provider is registered for.
    #[error(
        "provider.fetch names model {got:?}, but this provider is registered for {expected:?}"
    )]
    ModelMismatch { expected: String, got: String },
    /// An image part whose source is not inline data — a URL the gateway would
    /// have to fetch, or the provider would.
    #[error(
        "provider.fetch carries a non-inline image source ({url}); only data: URLs are accepted"
    )]
    RemoteImageSource { url: String },
    /// The request body is over the cap, or not base64.
    #[error("provider.fetch body is refused: {reason}")]
    BadBody { reason: String },
    /// The request body exceeds [`GatewayLimits::max_request_body_bytes`].
    #[error("provider.fetch request body is {len} bytes, over the {cap}-byte cap")]
    RequestTooLarge { len: usize, cap: usize },
    /// The response exceeds [`GatewayLimits::max_response_bytes`].
    #[error("provider response exceeded its {cap}-byte cap")]
    ResponseTooLarge { cap: u64 },
    /// The upstream call failed: refused, timed out, or died mid-body.
    #[error("provider {id:?} call failed: {detail}")]
    Upstream { id: String, detail: String },
}

impl GatewayError {
    /// The machine-readable `code` this failure travels back under in an OCAK1
    /// `res { ok: false }` / `end { ok: false }`. A caller branches on this, never
    /// on the message.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            GatewayError::UnknownProvider { .. } => "unknown_provider",
            GatewayError::BadPayload { .. }
            | GatewayError::BadPath { .. }
            | GatewayError::BadHeader { .. }
            | GatewayError::BadMethod { .. }
            | GatewayError::BadContentType { .. }
            | GatewayError::BadBodyShape { .. }
            | GatewayError::BadBody { .. } => "bad_request",
            GatewayError::UnsupportedOperation { .. } | GatewayError::MethodNotAllowed { .. } => {
                "unsupported_operation"
            }
            GatewayError::ModelMismatch { .. } => "model_mismatch",
            GatewayError::RemoteImageSource { .. } => "remote_image_source",
            GatewayError::RequestTooLarge { .. } => "request_too_large",
            GatewayError::ResponseTooLarge { .. } => "response_too_large",
            GatewayError::Upstream { .. } => "provider_unreachable",
            // Everything else is a registration-time failure: by the time a
            // request runs, the registry is already known good.
            _ => "internal",
        }
    }
}

/// Named host-rejection reasons. Constants rather than inline strings so the
/// table-driven test asserts the same text the refusal carries.
pub mod host_reason {
    /// Not an IP literal this gateway accepts in any form.
    pub const NOT_A_LITERAL: &str =
        "not a literal address in dotted-quad IPv4 or bracketed IPv6 form";
    /// A literal, but not a loopback one.
    pub const NOT_LOOPBACK: &str = "not a loopback address";
    /// A literal that does not round-trip to its own canonical text — a leading
    /// zero, an expanded IPv6, anything with a second reading.
    pub const NOT_CANONICAL: &str = "not the canonical text of the address it parses to";
    /// An IPv4-mapped IPv6 address.
    pub const IPV4_MAPPED: &str =
        "an IPv4-mapped IPv6 address; write the IPv4 loopback address directly";
    /// Nothing between `://` and the path.
    pub const EMPTY: &str = "empty";
}

/// Named path-rejection reasons, for the same purpose.
pub mod path_reason {
    /// Not rooted at `/`.
    pub const NOT_ROOTED: &str = "not rooted at /";
    /// `//…`, which a URL parser reads as an authority.
    pub const PROTOCOL_RELATIVE: &str = "protocol-relative: a leading // reads as an authority";
    /// Contains `://`, i.e. it is a URL and not a path.
    pub const IS_A_URL: &str = "a URL, not an operation path";
    /// Contains a `..` segment.
    pub const DOT_DOT: &str = "contains a .. segment";
    /// A byte outside printable ASCII — a control character, a space, a newline.
    pub const NOT_PRINTABLE_ASCII: &str = "contains a byte outside printable ASCII";
    /// Carries a fragment.
    pub const FRAGMENT: &str = "carries a fragment";
    /// Over the length cap.
    pub const TOO_LONG: &str = "longer than the path cap";
    /// The path moved the built URL off the registered endpoint.
    pub const ESCAPED_BASE: &str = "resolves to a URL outside the registered provider base";
    /// A percent-encoded dot. `%2e%2e` carries no raw `..` segment, and a URL
    /// parser decodes it back into one — so a text check that looked only for
    /// `..` would pass it straight through into a normalisation it never saw.
    pub const ENCODED_DOT: &str = "contains a percent-encoded dot segment";
    /// A backslash. WHATWG reads `\` as `/` in a special scheme, so the path a
    /// reader sees and the path the parser builds are two different paths.
    pub const BACKSLASH: &str = "contains a backslash, which a URL parser reads as a separator";
    /// The parsed URL's path is not the text that was appended to the prefix —
    /// the parser normalised something, and a path with two readings has no safe
    /// reading.
    pub const NOT_NORMALIZED: &str =
        "does not survive URL parsing unchanged: it normalises to a different path";
    /// The built path is not the prefix or a child of it. A STRING prefix test
    /// accepts `/v1-admin` for a `/v1` prefix; a segment-boundary test does not.
    pub const NOT_UNDER_PREFIX: &str =
        "is not the registered path prefix or a path segment beneath it";
}

// ─────────────────────────────────────────────────────────────────────────────
// The validated base
// ─────────────────────────────────────────────────────────────────────────────

/// The two schemes a provider base may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseScheme {
    /// `http`.
    Http,
    /// `https`.
    Https,
}

impl BaseScheme {
    /// The scheme's wire text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            BaseScheme::Http => "http",
            BaseScheme::Https => "https",
        }
    }

    const fn default_port(self) -> u16 {
        match self {
            BaseScheme::Http => 80,
            BaseScheme::Https => 443,
        }
    }
}

/// A provider base that has passed [`validate_provider_base`]: a scheme, a
/// **literal** address, a port, and the path prefix every operation hangs off.
///
/// There is no constructor other than the validator, and no field is public, so
/// a `ProviderBase` in hand is proof the check ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBase {
    scheme: BaseScheme,
    host: IpAddr,
    port: u16,
    /// `""` or `/v1` — never a trailing slash.
    path_prefix: String,
}

impl ProviderBase {
    /// The validated scheme.
    #[must_use]
    pub const fn scheme(&self) -> BaseScheme {
        self.scheme
    }

    /// The literal address `localhost` was normalised to, or the literal that was
    /// configured.
    #[must_use]
    pub const fn host(&self) -> IpAddr {
        self.host
    }

    /// The port, defaulted from the scheme when the base named none.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The allowed path family: every operation path is appended to this.
    #[must_use]
    pub fn path_prefix(&self) -> &str {
        &self.path_prefix
    }

    /// The host as it appears in a URL — bracketed for IPv6.
    #[must_use]
    pub fn host_literal(&self) -> String {
        match self.host {
            IpAddr::V4(addr) => addr.to_string(),
            IpAddr::V6(addr) => format!("[{addr}]"),
        }
    }

    /// `scheme://host:port`, with no path.
    #[must_use]
    pub fn origin(&self) -> String {
        format!(
            "{}://{}:{}",
            self.scheme.as_str(),
            self.host_literal(),
            self.port
        )
    }

    /// Builds the URL for one operation path.
    ///
    /// The path is checked, appended to the prefix, and the **result is parsed
    /// back and re-compared to this base**. The second check is the one that
    /// matters: it does not care which encoding trick got a path past the first,
    /// only whether the URL that came out still names the registered endpoint.
    ///
    /// Two properties beyond "same origin", and both are there because a string
    /// prefix test is not a path-subtree test:
    ///
    /// * **The parse must change nothing.** `url.path()` has to be byte-identical
    ///   to the text that was appended. `/%2e%2e/v1-admin/reset` carries no raw
    ///   `..` segment, so a text scan passes it; the parser then decodes the dots
    ///   and pops a segment, and what comes out is `/v1-admin/reset`. Comparing
    ///   the parser's output against the input is what notices, whatever the
    ///   encoding was.
    /// * **The prefix must match on a segment boundary.** `starts_with("/v1")` is
    ///   true of `/v1-admin`, which is a sibling of the registered subtree and not
    ///   a member of it. `== prefix || starts_with(prefix + "/")` is the same test
    ///   done on segments.
    pub fn resolve(&self, path: &str) -> Result<Url, GatewayError> {
        validate_operation_path(path)?;
        let refuse = |reason: &'static str| GatewayError::BadPath {
            path: path.to_string(),
            reason,
        };
        let text = format!("{}{}{}", self.origin(), self.path_prefix, path);
        // A path that makes the joined text unparseable is malformed, and there is
        // no reading of it that is safe to guess at.
        let url = Url::parse(&text).map_err(|_| refuse(path_reason::ESCAPED_BASE))?;
        let on_base = url.scheme() == self.scheme.as_str()
            && url.host_str() == Some(self.host_literal().as_str())
            && url.port_or_known_default() == Some(self.port)
            && url.username().is_empty()
            && url.password().is_none();
        if !on_base {
            return Err(refuse(path_reason::ESCAPED_BASE));
        }

        // Boundary first, so that an input which DID escape is refused for
        // escaping rather than for the encoding it escaped with.
        if !path_is_under_prefix(url.path(), &self.path_prefix) {
            return Err(refuse(path_reason::NOT_UNDER_PREFIX));
        }
        let appended = path.split('?').next().unwrap_or(path);
        if url.path() != format!("{}{}", self.path_prefix, appended) {
            return Err(refuse(path_reason::NOT_NORMALIZED));
        }
        Ok(url)
    }
}

/// Whether a built path lies inside a prefix's subtree, **on a segment
/// boundary**.
///
/// The property a `starts_with` test does not have: `/v1-admin/reset` starts with
/// `/v1` and is not inside `/v1`. It is a sibling — a different first segment
/// that happens to share a few bytes — and the difference between "shares a byte
/// prefix" and "is in the subtree" is the whole of F12's escape.
fn path_is_under_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// The longest operation path this gateway will build a URL from.
const MAX_PATH_BYTES: usize = 2048;

/// Reduces a configured provider base to a canonical, literal, loopback
/// [`ProviderBase`] — or says exactly why it cannot.
///
/// **This runs once, when a provider is registered**, and that is the point
/// (ADR-0017). Resolving `localhost` here means the per-request check compares
/// against `127.0.0.1`, not against a name that the resolver is free to answer
/// differently the second time. A name checked per request is a name that can
/// change between the check and the connect.
///
/// The accepted host forms are deliberately narrow, because every wider form has
/// two readings:
///
/// * `localhost` → normalised to `127.0.0.1`;
/// * a canonical dotted-quad IPv4 literal in `127.0.0.0/8`;
/// * `[::1]`.
///
/// Everything else is refused with a named reason: integer, octal and hex IPv4
/// (`2130706433`, `0177.0.0.1`, `0x7f.1` — all of which the WHATWG URL parser
/// happily reads as `127.0.0.1`, which is exactly why the *raw* host text is
/// classified here rather than the parser's normalised output), userinfo
/// (`http://127.0.0.1@evil.com/`), expanded IPv6, and IPv4-mapped IPv6.
///
/// **IPv4-mapped IPv6 is refused, deliberately.** `::ffff:127.0.0.1` denotes the
/// IPv4 loopback address, so refusing it costs a user nothing — they can write
/// `127.0.0.1`. Accepting it would mean this function has two spellings for one
/// address and every later comparison has to know that; a form with one spelling
/// is a form that cannot be compared wrong.
pub fn validate_provider_base(raw: &str) -> Result<ProviderBase, GatewayError> {
    let base = raw.trim();
    let url = Url::parse(base).map_err(|e| GatewayError::NotAUrl {
        base: base.to_string(),
        detail: e.to_string(),
    })?;

    let scheme = match url.scheme() {
        "http" => BaseScheme::Http,
        "https" => BaseScheme::Https,
        other => {
            return Err(GatewayError::Scheme {
                base: base.to_string(),
                scheme: other.to_string(),
            })
        }
    };

    if url.query().is_some() || url.fragment().is_some() {
        return Err(GatewayError::QueryOrFragment {
            base: base.to_string(),
        });
    }

    // The authority is taken from the RAW text, not from the parser. The parser
    // has already normalised `2130706433` into a dotted quad by this point, and
    // classifying its output would accept every encoded form this function
    // exists to refuse.
    let authority = raw_authority(base).ok_or_else(|| GatewayError::NotAUrl {
        base: base.to_string(),
        detail: "no authority".to_string(),
    })?;
    // Catches both a populated userinfo and the empty-userinfo form
    // (`http://@evil.com/`), which `Url::username()` reports as absent.
    if authority.contains('@') {
        return Err(GatewayError::Userinfo {
            base: base.to_string(),
        });
    }
    let host = classify_loopback_host(split_host(authority))?;

    let port = url.port().unwrap_or_else(|| scheme.default_port());
    let path_prefix = normalize_path_prefix(url.path(), base)?;

    Ok(ProviderBase {
        scheme,
        host,
        port,
        path_prefix,
    })
}

/// The raw `host[:port]` slice of a URL, userinfo included, exactly as written.
fn raw_authority(base: &str) -> Option<&str> {
    let after_scheme = base.split_once("://")?.1;
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    Some(&after_scheme[..end])
}

/// Splits the port off a raw authority, leaving the host text.
fn split_host(authority: &str) -> &str {
    if authority.starts_with('[') {
        // IPv6: the port, if any, follows the closing bracket.
        return match authority.find(']') {
            Some(close) => &authority[..=close],
            None => authority,
        };
    }
    match authority.rsplit_once(':') {
        Some((host, _port)) => host,
        None => authority,
    }
}

/// Classifies raw host text as one of the accepted literal loopback forms.
fn classify_loopback_host(host: &str) -> Result<IpAddr, GatewayError> {
    let refuse = |reason: &'static str| GatewayError::Host {
        host: host.to_string(),
        reason,
    };

    if host.is_empty() {
        return Err(refuse(host_reason::EMPTY));
    }
    if host.eq_ignore_ascii_case("localhost") {
        // The normalisation ADR-0017 asks for, and the only name accepted here.
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    if let Some(rest) = host.strip_prefix('[') {
        let inner = rest
            .strip_suffix(']')
            .ok_or_else(|| refuse(host_reason::NOT_A_LITERAL))?;
        let addr: Ipv6Addr = inner
            .parse()
            .map_err(|_| refuse(host_reason::NOT_A_LITERAL))?;
        // Checked before the loopback test so the message names the real reason:
        // `::ffff:127.0.0.1` IS the IPv4 loopback, and "not a loopback address"
        // would be a lie about why it was refused.
        if addr.to_ipv4_mapped().is_some() {
            return Err(refuse(host_reason::IPV4_MAPPED));
        }
        if !addr.is_loopback() {
            return Err(refuse(host_reason::NOT_LOOPBACK));
        }
        if inner.to_ascii_lowercase() != addr.to_string() {
            return Err(refuse(host_reason::NOT_CANONICAL));
        }
        return Ok(IpAddr::V6(addr));
    }

    let addr: Ipv4Addr = host
        .parse()
        .map_err(|_| refuse(host_reason::NOT_A_LITERAL))?;
    // The round trip is what closes the encoded-form family: anything that parses
    // to an address but is not that address's own canonical text has a second
    // reading, and a second reading is what the checks downstream cannot survive.
    if host != addr.to_string() {
        return Err(refuse(host_reason::NOT_CANONICAL));
    }
    if !addr.is_loopback() {
        return Err(refuse(host_reason::NOT_LOOPBACK));
    }
    Ok(IpAddr::V4(addr))
}

/// Reduces a base URL's path to the prefix operations hang off: `""` or `/v1`.
fn normalize_path_prefix(path: &str, base: &str) -> Result<String, GatewayError> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.split('/').any(|segment| segment == "..") {
        return Err(GatewayError::BasePath {
            base: base.to_string(),
            reason: path_reason::DOT_DOT,
        });
    }
    if !trimmed.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(GatewayError::BasePath {
            base: base.to_string(),
            reason: path_reason::NOT_PRINTABLE_ASCII,
        });
    }
    Ok(trimmed.to_string())
}

/// Checks one sidecar-supplied operation path before it is appended to a base.
fn validate_operation_path(path: &str) -> Result<(), GatewayError> {
    let refuse = |reason: &'static str| GatewayError::BadPath {
        path: path.to_string(),
        reason,
    };
    if path.len() > MAX_PATH_BYTES {
        return Err(refuse(path_reason::TOO_LONG));
    }
    if !path.starts_with('/') {
        return Err(refuse(path_reason::NOT_ROOTED));
    }
    if path.starts_with("//") {
        return Err(refuse(path_reason::PROTOCOL_RELATIVE));
    }
    if path.contains("://") {
        return Err(refuse(path_reason::IS_A_URL));
    }
    if path.contains('#') {
        return Err(refuse(path_reason::FRAGMENT));
    }
    if path.contains('\\') {
        return Err(refuse(path_reason::BACKSLASH));
    }
    // Cheapest form of the normalisation check, and the one that names the real
    // reason: `%2e` is a dot however it is cased, and a dot the text checks below
    // cannot see is a dot the URL parser will act on.
    let lower = path.to_ascii_lowercase();
    if lower.contains("%2e") {
        return Err(refuse(path_reason::ENCODED_DOT));
    }
    if !path.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(refuse(path_reason::NOT_PRINTABLE_ASCII));
    }
    let query_free = path.split('?').next().unwrap_or(path);
    if query_free.split('/').any(|segment| segment == "..") {
        return Err(refuse(path_reason::DOT_DOT));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// The operation allowlist
// ─────────────────────────────────────────────────────────────────────────────

/// The provider operations this build supports, and the only ones the gateway
/// forwards.
///
/// **Locality is not the whole policy.** An Ollama or llama.cpp server answers a
/// great deal more than the two calls a chat loop makes — model pull, model
/// delete, and in some builds an administrative surface — and all of it lives at
/// the same loopback origin behind the same path prefix. A gateway that checked
/// only where a request went would forward every one of those. Naming the two
/// operations means everything else is refused by not being on the list, which is
/// a property a reader can verify by reading the list rather than by imagining
/// what a path could spell.
///
/// The two entries are what `OpenAiCompatibleClient` calls: `GET /models` when it
/// probes, and `POST /chat/completions` for every turn, streamed or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOperation {
    /// `GET /models` — the provider's model list.
    ListModels,
    /// `POST /chat/completions` — one turn, streamed or not.
    ChatCompletions,
}

impl ProviderOperation {
    /// Every supported operation.
    pub const ALL: [ProviderOperation; 2] = [
        ProviderOperation::ListModels,
        ProviderOperation::ChatCompletions,
    ];

    /// The one path this operation is reached at, appended to the provider's
    /// prefix. An exact path, not a pattern: a pattern is a place for a second
    /// reading to hide.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            ProviderOperation::ListModels => "/models",
            ProviderOperation::ChatCompletions => "/chat/completions",
        }
    }

    /// The one method this operation accepts.
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            ProviderOperation::ListModels => "GET",
            ProviderOperation::ChatCompletions => "POST",
        }
    }

    /// The name this operation is reported under.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ProviderOperation::ListModels => "models.list",
            ProviderOperation::ChatCompletions => "chat.completions",
        }
    }

    /// `GET /models, POST /chat/completions` — what a refusal lists.
    fn supported() -> String {
        ProviderOperation::ALL
            .iter()
            .map(|op| format!("{} {}", op.method(), op.path()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The operation a method and an operation path name, or a refusal.
    ///
    /// The path must match exactly, query included — which is how a query string
    /// is refused without a separate rule for it: neither supported operation
    /// takes one, so `/models?limit=10` simply is not `/models`.
    fn classify(method: &Method, path: &str) -> Result<ProviderOperation, GatewayError> {
        match ProviderOperation::ALL
            .iter()
            .copied()
            .find(|op| op.path() == path)
        {
            Some(op) if op.method() == method.as_str() => Ok(op),
            Some(op) => Err(GatewayError::MethodNotAllowed {
                operation: op.as_str(),
                allowed: op.method(),
                method: method.to_string(),
            }),
            None => Err(GatewayError::UnsupportedOperation {
                method: method.to_string(),
                path: path.to_string(),
                supported: ProviderOperation::supported(),
            }),
        }
    }

    /// Whether this operation carries a JSON request body.
    const fn takes_json_body(self) -> bool {
        matches!(self, ProviderOperation::ChatCompletions)
    }

    /// Checks the body this operation was given, and returns the bytes to
    /// forward.
    ///
    /// The return value is the body rather than `()` because of the one case
    /// where the gateway edits: a chat body with no `model` gets the registered
    /// one **injected**. A body that already names the right model is forwarded
    /// byte-for-byte, so the common path re-serialises nothing.
    fn check_body(self, body: Vec<u8>, model: &str) -> Result<Vec<u8>, GatewayError> {
        let refuse = |reason: &str| GatewayError::BadBodyShape {
            operation: self.as_str(),
            reason: reason.to_string(),
        };
        match self {
            ProviderOperation::ListModels => {
                if !body.is_empty() {
                    return Err(refuse("a model list request carries no body"));
                }
                Ok(body)
            }
            ProviderOperation::ChatCompletions => {
                if body.is_empty() {
                    return Err(refuse("a chat completion needs a JSON body"));
                }
                let mut value: Value =
                    serde_json::from_slice(&body).map_err(|e| refuse(&format!("not JSON: {e}")))?;
                let object = value
                    .as_object_mut()
                    .ok_or_else(|| refuse("not a JSON object"))?;
                match object.get("messages") {
                    Some(Value::Array(messages)) if !messages.is_empty() => {}
                    Some(Value::Array(_)) => return Err(refuse("messages is empty")),
                    Some(_) => return Err(refuse("messages is not an array")),
                    None => return Err(refuse("no messages")),
                }
                // Before the model check, so a body carrying BOTH a wrong model
                // and a remote image is refused for whichever reason the reader
                // would consider worse.
                reject_non_inline_image_sources(&value)?;
                let Some(object) = value.as_object_mut() else {
                    unreachable!("checked above")
                };
                match object.get("model") {
                    // The overwhelmingly common case: forward the bytes the
                    // sidecar built, unmodified.
                    Some(Value::String(named)) if named == model => Ok(body),
                    Some(Value::String(named)) => Err(GatewayError::ModelMismatch {
                        expected: model.to_string(),
                        got: named.clone(),
                    }),
                    Some(_) => Err(refuse("model is not a string")),
                    None => {
                        object.insert("model".to_string(), Value::String(model.to_string()));
                        serde_json::to_vec(&value)
                            .map_err(|e| refuse(&format!("re-serialising the body failed: {e}")))
                    }
                }
            }
        }
    }
}

/// Refuses any image part whose source is not inline data.
///
/// An OpenAI `image_url` part may name a URL, and the party that would fetch it
/// is the inference server — off this machine, at an address nothing in this
/// module validated, chosen by whatever produced the request. Until OneCAD has an
/// attachment resolver that turns a local file into inline data, the honest
/// answer is that only `data:` URLs go out.
///
/// The walk is over the whole body rather than over `messages[*].content[*]`,
/// because "the only place an image part can appear" is a claim about a schema
/// that the provider, not this gateway, owns. Depth is bounded by `serde_json`'s
/// own parse recursion limit, so the recursion here is bounded by the parse that
/// already happened.
fn reject_non_inline_image_sources(value: &Value) -> Result<(), GatewayError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "image_url" {
                    let url = child
                        .get("url")
                        .and_then(Value::as_str)
                        .or_else(|| child.as_str());
                    match url {
                        Some(url) if url.starts_with("data:") => {}
                        Some(url) => {
                            return Err(GatewayError::RemoteImageSource { url: elide(url) })
                        }
                        None => {
                            return Err(GatewayError::BadBodyShape {
                                operation: ProviderOperation::ChatCompletions.as_str(),
                                reason: "an image part with no url".to_string(),
                            })
                        }
                    }
                }
                reject_non_inline_image_sources(child)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(reject_non_inline_image_sources),
        _ => Ok(()),
    }
}

/// The first 96 characters of a string, for an error message that must name what
/// it refused without pasting a megabyte of base64 into a log line.
fn elide(text: &str) -> String {
    const LIMIT: usize = 96;
    match text.char_indices().nth(LIMIT) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The registry
// ─────────────────────────────────────────────────────────────────────────────

/// One provider as OneCAD settings describe it, before validation.
///
/// Constructed only by Rust, from settings. **Nothing that arrives over the
/// bridge can reach this type**: `provider.fetch` carries an id, the wire verb
/// table (`SIDECAR_TO_HOST_VERBS`) has no other entry, and there is deliberately
/// no verb that adds, edits or selects a provider.
#[derive(Clone)]
pub struct ProviderConfig {
    /// The id the sidecar names. Matches `AiProviderConfig.id` on the TS side.
    pub id: String,
    /// The configured base, e.g. `http://127.0.0.1:11434/v1`.
    pub base_url: String,
    /// The model this provider serves.
    pub model: String,
    /// The credential the gateway injects, if the local runtime wants one. The
    /// sidecar never holds it — `createGatewayFetch` strips `Authorization`
    /// before the payload is sent.
    pub api_key: Option<String>,
}

/// Redacted: a provider config is reachable from [`crate::assistant::HostConfig`],
/// which derives `Debug`, and a credential must not be one `{:?}` away from a log
/// line.
impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A registry entry whose base has been validated.
struct RegisteredProvider {
    id: String,
    base: ProviderBase,
    model: String,
    api_key: Option<String>,
}

/// The bounds every gateway request runs inside.
#[derive(Debug, Clone)]
pub struct GatewayLimits {
    /// Cap on the decoded request body.
    pub max_request_body_bytes: usize,
    /// Cap on the total response body, across every chunk.
    pub max_response_bytes: u64,
    /// Total time from the start of connecting to the last body byte.
    pub total_timeout: Duration,
    /// Time allowed for the connect phase alone.
    pub connect_timeout: Duration,
}

impl Default for GatewayLimits {
    /// Production bounds.
    ///
    /// The request cap is 512 KiB. OCAK1's 1 MiB `MAX_JSON_LEN` already bounds a
    /// `provider.fetch` payload — the body rides base64 inside the JSON envelope,
    /// so framing alone stops it near 750 KiB — but a limit that exists only as a
    /// side effect of another layer's constant is a limit nobody can find.
    ///
    /// The response cap is 64 MiB: a token stream is small, and anything an order
    /// of magnitude past a long completion is a runaway, not an answer.
    ///
    /// Ten minutes total is sized for a slow local model on CPU, which is the
    /// case this gateway exists to serve. The connect phase gets five seconds
    /// because the peer is on this machine: if it has not accepted by then it is
    /// not listening.
    fn default() -> Self {
        GatewayLimits {
            max_request_body_bytes: 512 * 1024,
            max_response_bytes: 64 * 1024 * 1024,
            total_timeout: Duration::from_secs(600),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// The `provider.fetch` payload, exactly as `createGatewayFetch` sends it.
///
/// `deny_unknown_fields` is load-bearing rather than tidy. It is what turns a
/// payload that tries to carry a `url`, a `baseUrl` or an `origin` into a named
/// refusal instead of a field that is quietly ignored while the id is honoured.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderFetchPayload {
    provider_id: String,
    method: String,
    path: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body_base64: Option<String>,
}

/// The only headers the sidecar may set.
///
/// An allowlist, for the same reason the operations are one: the set of headers
/// a chat client needs is two, and every header beyond them is either something
/// the HTTP client owns (framing, length, host) or something that changes where
/// or as whom the request arrives.
const PERMITTED_REQUEST_HEADERS: [&str; 2] = ["accept", "content-type"];

/// Headers with a refusal of their own, so the message names the real reason
/// instead of "not on the list". Every one of these is also absent from
/// [`PERMITTED_REQUEST_HEADERS`] — this table only improves the diagnosis.
const REFUSED_REQUEST_HEADERS: [(&str, &str); 14] = [
    (
        "authorization",
        "the gateway injects the provider credential; the sidecar holds none",
    ),
    (
        "proxy-authorization",
        "a credential for a proxy, and this gateway has no proxy",
    ),
    ("cookie", "a credential the sidecar has no business holding"),
    (
        "x-api-key",
        "a credential the sidecar has no business holding",
    ),
    (
        "api-key",
        "a credential the sidecar has no business holding",
    ),
    (
        "host",
        "the host is the registered provider's, not a header",
    ),
    (
        "content-length",
        "set from the body the gateway actually sends",
    ),
    (
        "transfer-encoding",
        "framing belongs to the HTTP client, not to the caller",
    ),
    (
        "proxy-connection",
        "proxying is disabled; a header cannot turn it back on",
    ),
    (
        "forwarded",
        "the gateway does not forward on anyone's behalf",
    ),
    (
        "x-forwarded-for",
        "the gateway does not forward on anyone's behalf",
    ),
    (
        "x-forwarded-host",
        "the gateway does not forward on anyone's behalf",
    ),
    (
        "x-forwarded-proto",
        "the gateway does not forward on anyone's behalf",
    ),
    ("via", "the gateway does not forward on anyone's behalf"),
];

/// The content type a JSON operation is sent with, and the only one it accepts.
const CONTENT_TYPE_JSON: &str = "application/json";

/// Checks the sidecar's headers against the allowlist and the operation's
/// content-type policy, and returns the headers to actually send.
///
/// The content type is supplied when the operation takes a body and the sidecar
/// did not name one, rather than left off: the body IS JSON, and a provider that
/// guesses at an unlabelled body is a provider guessing.
fn validate_request_headers(
    headers: &BTreeMap<String, String>,
    operation: ProviderOperation,
) -> Result<Vec<(HeaderName, HeaderValue)>, GatewayError> {
    let mut validated: Vec<(HeaderName, HeaderValue)> = Vec::with_capacity(headers.len() + 1);
    let mut saw_content_type = false;
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if let Some((_, reason)) = REFUSED_REQUEST_HEADERS
            .iter()
            .find(|(refused, _)| *refused == lower)
        {
            return Err(GatewayError::BadHeader {
                name: name.clone(),
                reason,
            });
        }
        if !PERMITTED_REQUEST_HEADERS.contains(&lower.as_str()) {
            return Err(GatewayError::BadHeader {
                name: name.clone(),
                reason: "not one of the headers a provider operation may carry",
            });
        }
        if lower == "content-type" {
            // `application/json; charset=utf-8` is the same content type; the
            // parameters after the `;` are not part of the decision.
            let essence = value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase();
            let accepted = operation.takes_json_body() && essence == CONTENT_TYPE_JSON;
            if !accepted {
                return Err(GatewayError::BadContentType {
                    operation: operation.as_str(),
                    got: value.clone(),
                });
            }
            saw_content_type = true;
        }
        let header_name =
            HeaderName::from_bytes(lower.as_bytes()).map_err(|_| GatewayError::BadHeader {
                name: name.clone(),
                reason: "not a legal HTTP header name",
            })?;
        let header_value = HeaderValue::from_str(value).map_err(|_| GatewayError::BadHeader {
            name: name.clone(),
            reason: "not a legal HTTP header value",
        })?;
        validated.push((header_name, header_value));
    }
    if operation.takes_json_body() && !saw_content_type {
        validated.push((
            HeaderName::from_static("content-type"),
            HeaderValue::from_static(CONTENT_TYPE_JSON),
        ));
    }
    Ok(validated)
}

/// The Rust-owned registry of local providers, plus the one HTTP client they
/// share.
pub struct ProviderGateway {
    providers: HashMap<String, RegisteredProvider>,
    client: reqwest::Client,
    limits: GatewayLimits,
}

/// Ids only. The registry holds credentials; `{:?}` must never be the way one
/// escapes.
impl std::fmt::Debug for ProviderGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ids: Vec<&str> = self.providers.keys().map(String::as_str).collect();
        ids.sort_unstable();
        f.debug_struct("ProviderGateway")
            .field("providers", &ids)
            .field("limits", &self.limits)
            .finish()
    }
}

impl ProviderGateway {
    /// Builds a gateway over a settings-supplied registry.
    ///
    /// A bad entry fails the whole construction rather than being skipped. The
    /// input is trusted configuration, so a rejected entry is a misconfiguration
    /// the user needs told about — a gateway that silently drops the provider
    /// they just typed in and then reports "unknown provider" on every call has
    /// hidden the actual fault one layer down.
    pub fn new(
        configs: Vec<ProviderConfig>,
        limits: GatewayLimits,
    ) -> Result<ProviderGateway, GatewayError> {
        let mut providers = HashMap::with_capacity(configs.len());
        for config in configs {
            if config.id.trim().is_empty() {
                return Err(GatewayError::IncompleteProvider { reason: "no id" });
            }
            if config.model.trim().is_empty() {
                return Err(GatewayError::IncompleteProvider {
                    reason: "no model id",
                });
            }
            let base = validate_provider_base(&config.base_url)?;
            if base.scheme() == BaseScheme::Https {
                return Err(GatewayError::TlsNotBuilt {
                    id: config.id,
                    base: config.base_url,
                });
            }
            if providers.contains_key(&config.id) {
                return Err(GatewayError::DuplicateProvider { id: config.id });
            }
            providers.insert(
                config.id.clone(),
                RegisteredProvider {
                    id: config.id,
                    base,
                    model: config.model,
                    api_key: config.api_key,
                },
            );
        }
        let client = Self::build_client(&limits)?;
        Ok(ProviderGateway {
            providers,
            client,
            limits,
        })
    }

    /// The HTTP client, with the two behaviours that must be off, off.
    ///
    /// **Redirects are disabled.** A redirect is a second hop chosen by whatever
    /// answered the first. Every check in this module happens before the request
    /// goes out, so a followed `302` would be a request to an address nothing
    /// validated — the one way a loopback-only client reaches somewhere else
    /// without a single line of this file being wrong. A 3xx is handed back to
    /// the caller verbatim instead, `Location` and all, and never acted on.
    ///
    /// **Proxies are disabled, including the environment's.** `HTTPS_PROXY`,
    /// `HTTP_PROXY` and `ALL_PROXY` are ambient process state that the user's
    /// shell, an IT profile, or another tool may have set for entirely unrelated
    /// reasons. An inherited proxy would take a request this module validated as
    /// loopback and send it off the machine, which is precisely the property
    /// ADR-0017 is about. `default-features = false` on the dependency already
    /// drops reqwest's `system-proxy` feature; `no_proxy()` says it again at the
    /// call site, because a feature flag in a manifest is not where a reader
    /// looks for this.
    fn build_client(limits: &GatewayLimits) -> Result<reqwest::Client, GatewayError> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(limits.connect_timeout)
            // reqwest applies this "from when the request starts connecting until
            // the response body has finished", so it bounds the whole exchange
            // including a stalled stream — which is the bound that matters here.
            .timeout(limits.total_timeout)
            .build()
            .map_err(|e| GatewayError::ClientBuild(e.to_string()))
    }

    /// The registered ids, sorted. Used in refusal messages and by tests.
    #[must_use]
    pub fn provider_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.providers.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    /// The model id registered for a provider.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&str> {
        self.providers.get(id).map(|p| p.model.as_str())
    }

    /// The validated base registered for a provider.
    #[must_use]
    pub fn base(&self, id: &str) -> Option<&ProviderBase> {
        self.providers.get(id).map(|p| &p.base)
    }

    /// The bounds this gateway enforces.
    #[must_use]
    pub fn limits(&self) -> &GatewayLimits {
        &self.limits
    }

    /// Performs one `provider.fetch` and returns the response head plus an
    /// unread body.
    ///
    /// The body is **not** collected here. A completion stream has no length the
    /// caller knows in advance, so reading it into a `Vec` would make the memory
    /// cost a function of how much the model felt like saying.
    pub async fn fetch(&self, payload: Value) -> Result<ProviderStream, GatewayError> {
        let request: ProviderFetchPayload =
            serde_json::from_value(payload).map_err(|e| GatewayError::BadPayload {
                detail: e.to_string(),
            })?;

        let provider = self.providers.get(&request.provider_id).ok_or_else(|| {
            GatewayError::UnknownProvider {
                id: request.provider_id.clone(),
                registered: self.provider_ids().join(", "),
            }
        })?;

        let url = provider.base.resolve(&request.path)?;
        let method =
            Method::from_bytes(request.method.as_bytes()).map_err(|_| GatewayError::BadMethod {
                method: request.method.clone(),
            })?;
        let operation = ProviderOperation::classify(&method, &request.path)?;
        let headers = validate_request_headers(&request.headers, operation)?;

        let body = match &request.body_base64 {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| GatewayError::BadBody {
                    reason: format!("bodyBase64 is not base64: {e}"),
                })?,
            None => Vec::new(),
        };
        // The cap is checked on the decoded bytes before the body is parsed, so
        // an oversized body costs a base64 decode and not a JSON parse.
        if body.len() > self.limits.max_request_body_bytes {
            return Err(GatewayError::RequestTooLarge {
                len: body.len(),
                cap: self.limits.max_request_body_bytes,
            });
        }
        let body = operation.check_body(body, &provider.model)?;

        let mut builder = self.client.request(method, url);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        if let Some(key) = &provider.api_key {
            let mut value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                GatewayError::BadHeader {
                    name: "authorization".to_string(),
                    reason: "the configured API key is not a legal header value",
                }
            })?;
            value.set_sensitive(true);
            builder = builder.header(AUTHORIZATION, value);
        }
        if !body.is_empty() {
            builder = builder.body(body);
        }

        let response = builder.send().await.map_err(|e| GatewayError::Upstream {
            id: provider.id.clone(),
            detail: e.to_string(),
        })?;

        ProviderStream::new(
            provider.id.clone(),
            response,
            self.limits.max_response_bytes,
        )
    }
}

/// The app's one live gateway, in a holder that can be swapped while the sidecar
/// is running.
///
/// A [`ProviderGateway`] is immutable once built — its registry is validated at
/// construction and never edited — so "the user changed their endpoint" is
/// expressed by building a NEW gateway and installing it here. The bridge reads
/// [`current`](Self::current) once per `provider.fetch`, which is what makes a
/// reconfiguration take effect on the next request instead of on the next app
/// start.
///
/// `None` means no provider is configured, and the bridge answers
/// `provider.fetch` with `unimplemented` — what a build the user has not given a
/// local model should say.
///
/// Nothing that arrives over the bridge can reach [`install`](Self::install):
/// the only caller is the trusted settings command (ADR-0017).
#[derive(Default)]
pub struct ProviderRegistry {
    current: Mutex<Option<Arc<ProviderGateway>>>,
}

impl ProviderRegistry {
    /// An empty holder: no provider configured.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The gateway to serve one request from, or `None` when none is configured.
    #[must_use]
    pub fn current(&self) -> Option<Arc<ProviderGateway>> {
        self.current
            .lock()
            .expect("assistant provider registry poisoned")
            .clone()
    }

    /// Installs a gateway, or clears the registry with `None`.
    ///
    /// An in-flight request keeps the `Arc` it already read, so a swap never
    /// pulls a provider out from under a completion that is already streaming.
    pub fn install(&self, gateway: Option<Arc<ProviderGateway>>) {
        *self
            .current
            .lock()
            .expect("assistant provider registry poisoned") = gateway;
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("current", &self.current())
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The response
// ─────────────────────────────────────────────────────────────────────────────

/// A provider response's head, in the shape `createGatewayFetch` reads back
/// (`HttpResponseHead` in `assistant-host/src/bridge/verbs.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    /// HTTP status code.
    pub status: u16,
    /// The status's canonical reason phrase.
    pub status_text: String,
    /// Response headers, lowercased by the HTTP stack.
    pub headers: BTreeMap<String, String>,
}

impl ResponseHead {
    /// The head as the OCAK1 `res` payload.
    #[must_use]
    pub fn to_payload(&self) -> Value {
        json!({
            "status": self.status,
            "statusText": self.status_text,
            "headers": self.headers,
        })
    }
}

/// A provider response: its head, plus a body that is read one chunk at a time
/// and counted against [`GatewayLimits::max_response_bytes`].
///
/// **Dropping this aborts the upstream call.** The `reqwest::Response` owns the
/// connection; dropping it mid-body closes it rather than leaving the provider
/// generating into a socket nobody is reading. That is how the bridge's
/// cancellation works — it drops the future that holds this.
#[derive(Debug)]
pub struct ProviderStream {
    provider_id: String,
    head: ResponseHead,
    response: reqwest::Response,
    remaining: u64,
    cap: u64,
}

impl ProviderStream {
    fn new(
        provider_id: String,
        response: reqwest::Response,
        cap: u64,
    ) -> Result<ProviderStream, GatewayError> {
        // Refused on the advertised length before a byte is read, when the
        // provider bothered to advertise one.
        if response.content_length().is_some_and(|len| len > cap) {
            return Err(GatewayError::ResponseTooLarge { cap });
        }
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_string(), text.to_string()))
            })
            .collect();
        Ok(ProviderStream {
            provider_id,
            head: ResponseHead {
                status: status.as_u16(),
                status_text: status.canonical_reason().unwrap_or_default().to_string(),
                headers,
            },
            response,
            remaining: cap,
            cap,
        })
    }

    /// The response head.
    #[must_use]
    pub fn head(&self) -> &ResponseHead {
        &self.head
    }

    /// The provider this response came from.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The next body chunk, or `None` at the end of the body.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, GatewayError> {
        let chunk = self
            .response
            .chunk()
            .await
            .map_err(|e| GatewayError::Upstream {
                id: self.provider_id.clone(),
                detail: e.to_string(),
            })?;
        let Some(bytes) = chunk else {
            return Ok(None);
        };
        let len = bytes.len() as u64;
        if len > self.remaining {
            return Err(GatewayError::ResponseTooLarge { cap: self.cap });
        }
        self.remaining -= len;
        Ok(Some(bytes.to_vec()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn local(id: &str, base_url: &str) -> ProviderConfig {
        ProviderConfig {
            id: id.to_string(),
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key: None,
        }
    }

    /// THE security test. Every accepted form, and every rejected form, with the
    /// reason each rejection must name.
    #[test]
    fn validate_provider_base_accepts_only_canonical_literal_loopback() {
        // (base, expected host, expected port, expected path prefix)
        let accepted: &[(&str, IpAddr, u16, &str)] = &[
            (
                "http://127.0.0.1:11434",
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                11434,
                "",
            ),
            (
                "http://127.0.0.1:11434/v1",
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                11434,
                "/v1",
            ),
            (
                "http://127.0.0.1:11434/v1/",
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                11434,
                "/v1",
            ),
            // `localhost` is normalised HERE, once, so the per-request check is
            // against a literal and never against a name (ADR-0017).
            (
                "http://localhost:1234/v1",
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                1234,
                "/v1",
            ),
            (
                "http://LOCALHOST:1234",
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                1234,
                "",
            ),
            // The whole of 127.0.0.0/8, not just .1.
            (
                "http://127.2.3.4:8080",
                IpAddr::V4(Ipv4Addr::new(127, 2, 3, 4)),
                8080,
                "",
            ),
            (
                "http://[::1]:8080/v1",
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                8080,
                "/v1",
            ),
            // No port: defaulted from the scheme.
            ("http://127.0.0.1", IpAddr::V4(Ipv4Addr::LOCALHOST), 80, ""),
            (
                "https://127.0.0.1",
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                443,
                "",
            ),
        ];
        for (base, host, port, prefix) in accepted {
            let validated = validate_provider_base(base)
                .unwrap_or_else(|e| panic!("{base} must be accepted, got: {e}"));
            assert_eq!(validated.host(), *host, "{base}: host");
            assert_eq!(validated.port(), *port, "{base}: port");
            assert_eq!(validated.path_prefix(), *prefix, "{base}: path prefix");
        }

        // (base, why it must be refused, the reason text the refusal must carry)
        let refused: &[(&str, &str, &str)] = &[
            (
                "http://2130706433/v1",
                "decimal-integer IPv4: the WHATWG URL parser reads this as 127.0.0.1",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://0177.0.0.1/v1",
                "octal IPv4: 0177 is 127 to a URL parser",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://0x7f.1/v1",
                "hex/short IPv4: also 127.0.0.1 to a URL parser",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://0x7f.0.0.1/v1",
                "hex-octet IPv4",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://127.1/v1",
                "short-form IPv4: two octets, read as 127.0.0.1",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://[::ffff:127.0.0.1]:8080/v1",
                "IPv4-mapped IPv6: a second spelling of the IPv4 loopback, refused deliberately",
                host_reason::IPV4_MAPPED,
            ),
            (
                "http://[0:0:0:0:0:0:0:1]:8080",
                "expanded IPv6: parses to ::1 but is not its canonical text",
                host_reason::NOT_CANONICAL,
            ),
            (
                "http://[::2]:8080",
                "a literal IPv6 that is not loopback",
                host_reason::NOT_LOOPBACK,
            ),
            (
                "http://10.0.0.5:8080",
                "a private but non-loopback IPv4 literal",
                host_reason::NOT_LOOPBACK,
            ),
            (
                "http://0.0.0.0:8080",
                "the unspecified address is not loopback",
                host_reason::NOT_LOOPBACK,
            ),
            (
                "http://example.com/v1",
                "a name that is not localhost",
                host_reason::NOT_A_LITERAL,
            ),
            (
                "http://localhost.evil.com/v1",
                "a name whose leftmost label is localhost",
                host_reason::NOT_A_LITERAL,
            ),
        ];
        for (base, why, reason) in refused {
            match validate_provider_base(base) {
                Err(GatewayError::Host { reason: got, .. }) => {
                    assert_eq!(got, *reason, "{base} refused for the wrong reason ({why})");
                }
                other => panic!("{base} must be refused — {why} — got {other:?}"),
            }
        }
    }

    #[test]
    fn validate_provider_base_refuses_userinfo() {
        // The reading that matters: the host here is `evil.com`.
        for base in [
            "http://127.0.0.1@evil.com/",
            "http://user:pass@127.0.0.1:8080/",
            "http://@evil.com/",
            "http://127.0.0.1@127.0.0.1:8080/",
        ] {
            match validate_provider_base(base) {
                Err(GatewayError::Userinfo { .. }) => {}
                other => panic!(
                    "{base} must be refused: userinfo hides the real host from a casual reader, \
                     got {other:?}"
                ),
            }
        }
    }

    #[test]
    fn validate_provider_base_refuses_a_non_http_scheme() {
        for base in [
            "file:///etc/passwd",
            "ftp://127.0.0.1/v1",
            "ws://127.0.0.1:8080",
            "data:text/plain,hi",
        ] {
            match validate_provider_base(base) {
                Err(GatewayError::Scheme { .. }) => {}
                other => panic!("{base} must be refused: only http and https, got {other:?}"),
            }
        }
    }

    #[test]
    fn validate_provider_base_refuses_a_query_or_fragment_in_the_base() {
        for base in [
            "http://127.0.0.1:11434/v1?key=secret",
            "http://127.0.0.1:11434/v1#frag",
        ] {
            match validate_provider_base(base) {
                Err(GatewayError::QueryOrFragment { .. }) => {}
                other => panic!(
                    "{base} must be refused: a base is a prefix, and a query or fragment in it \
                     cannot survive having a path appended, got {other:?}"
                ),
            }
        }
    }

    #[test]
    fn validate_provider_base_refuses_text_that_is_not_a_url() {
        for base in ["", "   ", "127.0.0.1:11434", "not a url"] {
            match validate_provider_base(base) {
                Err(GatewayError::NotAUrl { .. }) => {}
                other => panic!("{base:?} must be refused: not a URL, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_non_loopback_base_is_refused_at_registration() {
        let err = ProviderGateway::new(
            vec![local("remote", "http://api.example.com/v1")],
            GatewayLimits::default(),
        )
        .expect_err("a non-loopback base must never reach the registry");
        match err {
            GatewayError::Host { reason, .. } => assert_eq!(reason, host_reason::NOT_A_LITERAL),
            other => panic!("expected a host refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_https_base_is_refused_at_registration_naming_the_missing_tls_backend() {
        let err = ProviderGateway::new(
            vec![local("tls", "https://127.0.0.1:8443/v1")],
            GatewayLimits::default(),
        )
        .expect_err("an https base must be refused where it is configured, not at request time");
        assert!(
            matches!(err, GatewayError::TlsNotBuilt { .. }),
            "the refusal must name the reason: {err}"
        );
    }

    #[test]
    fn a_registry_entry_without_a_model_is_refused() {
        let mut config = local("local", "http://127.0.0.1:11434/v1");
        config.model = String::new();
        let err = ProviderGateway::new(vec![config], GatewayLimits::default())
            .expect_err("a provider with no model id is incomplete");
        assert!(
            matches!(err, GatewayError::IncompleteProvider { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_duplicate_provider_id_is_refused() {
        let err = ProviderGateway::new(
            vec![
                local("local", "http://127.0.0.1:11434/v1"),
                local("local", "http://127.0.0.1:8080/v1"),
            ],
            GatewayLimits::default(),
        )
        .expect_err("two entries cannot claim one id");
        assert!(
            matches!(err, GatewayError::DuplicateProvider { .. }),
            "{err}"
        );
    }

    #[test]
    fn the_registry_holds_the_model_and_the_validated_base() {
        let gateway = ProviderGateway::new(
            vec![ProviderConfig {
                id: "ollama".into(),
                base_url: "http://localhost:11434/v1".into(),
                model: "qwen3:8b".into(),
                api_key: Some("secret".into()),
            }],
            GatewayLimits::default(),
        )
        .expect("a loopback provider");
        assert_eq!(gateway.provider_ids(), vec!["ollama"]);
        assert_eq!(gateway.model("ollama"), Some("qwen3:8b"));
        let base = gateway.base("ollama").expect("a registered base");
        assert_eq!(base.host(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(base.origin(), "http://127.0.0.1:11434");

        // The credential must not be one `{:?}` away from a log line.
        let printed = format!("{gateway:?}");
        assert!(!printed.contains("secret"), "{printed}");
    }

    #[test]
    fn resolve_builds_the_url_from_the_base_and_the_operation_path() {
        let base = validate_provider_base("http://127.0.0.1:11434/v1").unwrap();
        assert_eq!(
            base.resolve("/chat/completions").unwrap().as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        // A query is part of the operation, not of the base.
        assert_eq!(
            base.resolve("/models?limit=10").unwrap().as_str(),
            "http://127.0.0.1:11434/v1/models?limit=10"
        );
        let v6 = validate_provider_base("http://[::1]:11434").unwrap();
        assert_eq!(
            v6.resolve("/chat/completions").unwrap().as_str(),
            "http://[::1]:11434/chat/completions"
        );
    }

    #[test]
    fn resolve_refuses_a_path_that_is_not_an_operation_path() {
        let base = validate_provider_base("http://127.0.0.1:11434/v1").unwrap();
        let cases: &[(&str, &str, &str)] = &[
            (
                "chat/completions",
                "not rooted at /",
                path_reason::NOT_ROOTED,
            ),
            (
                "//evil.com/v1/chat",
                "protocol-relative: a URL parser reads // as an authority",
                path_reason::PROTOCOL_RELATIVE,
            ),
            (
                "/x/http://evil.com/",
                "a URL smuggled into a path",
                path_reason::IS_A_URL,
            ),
            (
                "/../../admin",
                "climbs out of the registered path family",
                path_reason::DOT_DOT,
            ),
            ("/chat#frag", "carries a fragment", path_reason::FRAGMENT),
            (
                "/chat\r\nX-Injected: 1",
                "CRLF in a path is request splitting",
                path_reason::NOT_PRINTABLE_ASCII,
            ),
            (
                "/chat completions",
                "a raw space is not a legal path byte",
                path_reason::NOT_PRINTABLE_ASCII,
            ),
            (
                "/%2e%2e/v1-admin/reset",
                "F12: a percent-encoded .. that a URL parser decodes and acts on",
                path_reason::ENCODED_DOT,
            ),
            (
                "/%2E%2E/v1-admin/reset",
                "the same, cased the other way",
                path_reason::ENCODED_DOT,
            ),
            (
                "/chat/%2e%2e%2f%2e%2e/admin",
                "encoded dots and encoded separators together",
                path_reason::ENCODED_DOT,
            ),
            (
                "/chat\\completions",
                "a backslash, which a URL parser reads as a separator",
                path_reason::BACKSLASH,
            ),
            (
                "/..\\..\\admin",
                "backslash-spelled parent segments",
                path_reason::BACKSLASH,
            ),
            (
                "/a/./b",
                "a single-dot segment the parser collapses, so the built path is \
                 not the path that was written",
                path_reason::NOT_NORMALIZED,
            ),
            (
                "/chat`completions",
                "a byte the URL parser percent-encodes, so the built path is not \
                 the path that was written",
                path_reason::NOT_NORMALIZED,
            ),
        ];
        for (path, why, reason) in cases {
            match base.resolve(path) {
                Err(GatewayError::BadPath { reason: got, .. }) => {
                    assert_eq!(
                        got, *reason,
                        "{path:?} refused for the wrong reason ({why})"
                    )
                }
                other => panic!("{path:?} must be refused — {why} — got {other:?}"),
            }
        }

        let long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
        assert!(
            matches!(
                base.resolve(&long),
                Err(GatewayError::BadPath {
                    reason: path_reason::TOO_LONG,
                    ..
                })
            ),
            "an unbounded path is not a path"
        );
    }

    /// A path can only ever append. Even a `..` that slipped the text checks
    /// would have to move the built URL off the base to matter, and that is
    /// checked separately on the URL itself.
    #[test]
    fn resolve_keeps_the_built_url_on_the_registered_endpoint() {
        let base = validate_provider_base("http://127.0.0.1:11434/v1").unwrap();
        for path in ["/chat/completions", "/models", "/a/b/c?d=e&f=g"] {
            let url = base.resolve(path).unwrap();
            assert_eq!(url.host_str(), Some("127.0.0.1"), "{path}");
            assert_eq!(url.port(), Some(11434), "{path}");
            assert_eq!(url.scheme(), "http", "{path}");
            assert!(url.path().starts_with("/v1"), "{path}: {url}");
        }
    }

    /// Requirement: nothing arriving over the bridge may add, modify or select a
    /// provider outside the registry. `deny_unknown_fields` is what makes an
    /// attempt a named refusal instead of an ignored field.
    #[test]
    fn a_payload_carrying_a_url_or_base_override_is_refused() {
        for extra in ["url", "baseUrl", "base", "origin", "host", "endpoint"] {
            let payload = json!({
                "providerId": "local",
                "method": "POST",
                "path": "/chat/completions",
                "headers": {},
                extra: "http://evil.example.com/v1",
            });
            let parsed: Result<ProviderFetchPayload, _> = serde_json::from_value(payload);
            let err = parsed.expect_err(&format!(
                "a payload carrying {extra:?} must be refused: §5 says the sidecar names an id, \
                 never a URL"
            ));
            assert!(
                err.to_string().contains(extra),
                "the refusal must name the field it refused: {err}"
            );
        }
    }

    #[test]
    fn the_payload_shape_is_the_one_the_sidecar_sends() {
        // Pinned by `createGatewayFetch` in assistant-host/src/bridge/verbs.ts.
        let parsed: ProviderFetchPayload = serde_json::from_value(json!({
            "providerId": "local",
            "method": "POST",
            "path": "/chat/completions",
            "headers": {"content-type": "application/json"},
            "bodyBase64": "e30=",
        }))
        .expect("the sidecar's payload shape");
        assert_eq!(parsed.provider_id, "local");
        assert_eq!(parsed.path, "/chat/completions");
        assert_eq!(parsed.headers["content-type"], "application/json");
        assert_eq!(parsed.body_base64.as_deref(), Some("e30="));

        // `bodyBase64` and `headers` are optional; a GET sends neither.
        let get: ProviderFetchPayload = serde_json::from_value(json!({
            "providerId": "local",
            "method": "GET",
            "path": "/models",
        }))
        .expect("a bodiless payload");
        assert!(get.headers.is_empty());
        assert!(get.body_base64.is_none());
    }

    #[tokio::test]
    async fn an_unknown_provider_id_is_refused_at_request_time() {
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:11434/v1")],
            GatewayLimits::default(),
        )
        .unwrap();
        let err = gateway
            .fetch(json!({
                "providerId": "somewhere-else",
                "method": "POST",
                "path": "/chat/completions",
            }))
            .await
            .expect_err("an id outside the registry has no base to resolve against");
        match err {
            GatewayError::UnknownProvider { ref registered, .. } => {
                assert_eq!(registered, "local");
                assert_eq!(err.code(), "unknown_provider");
            }
            other => panic!("expected unknown_provider, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_oversized_request_body_is_refused_before_a_socket_is_opened() {
        let limits = GatewayLimits {
            max_request_body_bytes: 64,
            ..GatewayLimits::default()
        };
        // Port 1 has nothing listening: if the cap were checked after the connect
        // this would fail as `provider_unreachable` instead.
        let gateway =
            ProviderGateway::new(vec![local("local", "http://127.0.0.1:1/v1")], limits).unwrap();
        let body = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 65]);
        let err = gateway
            .fetch(json!({
                "providerId": "local",
                "method": "POST",
                "path": "/chat/completions",
                "bodyBase64": body,
            }))
            .await
            .expect_err("an oversized body must be refused");
        match err {
            GatewayError::RequestTooLarge { len, cap } => {
                assert_eq!(len, 65);
                assert_eq!(cap, 64);
                assert_eq!(err.code(), "request_too_large");
            }
            other => panic!("expected request_too_large, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_header_the_sidecar_may_not_set_is_refused() {
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:1/v1")],
            GatewayLimits::default(),
        )
        .unwrap();
        for (name, why) in [
            (
                "Authorization",
                "the gateway injects the credential; a forwarded one would shadow it",
            ),
            ("host", "the host is the registered provider's"),
            ("Content-Length", "set from the body the gateway sends"),
        ] {
            let err = gateway
                .fetch(json!({
                    "providerId": "local",
                    "method": "POST",
                    "path": "/chat/completions",
                    "headers": {name: "x"},
                }))
                .await
                .expect_err(why);
            assert!(
                matches!(err, GatewayError::BadHeader { .. }),
                "{name}: {why} — got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_method_that_is_not_an_http_token_is_refused() {
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:1/v1")],
            GatewayLimits::default(),
        )
        .unwrap();
        let err = gateway
            .fetch(json!({
                "providerId": "local",
                "method": "POST /evil HTTP/1.1\r\nX: y",
                "path": "/chat/completions",
            }))
            .await
            .expect_err("a method carrying CRLF is request splitting");
        assert!(matches!(err, GatewayError::BadMethod { .. }), "{err}");
    }

    #[test]
    fn every_wire_code_is_one_a_caller_can_branch_on() {
        assert_eq!(
            GatewayError::UnknownProvider {
                id: "x".into(),
                registered: String::new()
            }
            .code(),
            "unknown_provider"
        );
        assert_eq!(
            GatewayError::BadPayload {
                detail: String::new()
            }
            .code(),
            "bad_request"
        );
        assert_eq!(
            GatewayError::ResponseTooLarge { cap: 1 }.code(),
            "response_too_large"
        );
        assert_eq!(
            GatewayError::Upstream {
                id: "x".into(),
                detail: String::new()
            }
            .code(),
            "provider_unreachable"
        );
        assert_eq!(
            GatewayError::ClientBuild(String::new()).code(),
            "internal",
            "a registration-time failure is never the caller's fault to fix"
        );
    }

    #[test]
    fn the_registry_holder_swaps_and_clears_without_rebuilding_anything_else() {
        let registry = ProviderRegistry::new();
        assert!(
            registry.current().is_none(),
            "a fresh holder has no provider"
        );

        let first = Arc::new(
            ProviderGateway::new(
                vec![local("local", "http://127.0.0.1:11434/v1")],
                GatewayLimits::default(),
            )
            .unwrap(),
        );
        registry.install(Some(first));
        assert_eq!(
            registry.current().expect("installed").provider_ids(),
            vec!["local"]
        );

        // A swap: the same id, a different endpoint. What the next reader gets
        // is the new base, with no restart of anything.
        let second = Arc::new(
            ProviderGateway::new(
                vec![local("local", "http://127.0.0.1:8080")],
                GatewayLimits::default(),
            )
            .unwrap(),
        );
        registry.install(Some(second));
        assert_eq!(
            registry
                .current()
                .expect("installed")
                .base("local")
                .expect("a registered base")
                .origin(),
            "http://127.0.0.1:8080"
        );

        registry.install(None);
        assert!(registry.current().is_none(), "None clears the registry");
    }

    /// THE F12 regression, against the `url` crate rather than against WHATWG in
    /// the abstract.
    ///
    /// `/%2e%2e/v1-admin/reset` carries no raw `..` segment, so every text check
    /// the gateway used to run passed it. The parser then decodes the dots, pops
    /// `/v1`, and produces `/v1-admin/reset` — which `starts_with("/v1")` says is
    /// fine. It is not fine: `/v1-admin` is a SIBLING of the registered subtree.
    /// This test pins both halves of the fix, and pins them against the real
    /// parser so it keeps telling the truth if the parser's normalisation
    /// changes.
    #[test]
    fn a_sibling_prefix_reached_through_encoded_dots_is_refused() {
        let base = validate_provider_base("http://127.0.0.1:11434/v1").unwrap();

        // 1. What the parser actually does with the escape, stated as a fact
        //    rather than assumed: this is the behaviour being defended against.
        let escaped = Url::parse("http://127.0.0.1:11434/v1/%2e%2e/v1-admin/reset").unwrap();
        assert_eq!(
            escaped.path(),
            "/v1-admin/reset",
            "the url crate normalises encoded dot segments, which is why a text \
             scan for `..` is not enough"
        );
        assert!(
            escaped.path().starts_with("/v1"),
            "and the escaped path passes a STRING prefix test, which is why that \
             test is not enough either"
        );

        // 2. The gateway refuses it.
        match base.resolve("/%2e%2e/v1-admin/reset") {
            Err(GatewayError::BadPath { reason, .. }) => {
                assert_eq!(reason, path_reason::ENCODED_DOT)
            }
            other => panic!("the encoded-dot escape must be refused, got {other:?}"),
        }

        // 3. And the boundary test itself, which is what would catch an escape
        //    arriving by some encoding nobody has thought of yet.
        for (path, under) in [
            ("/v1", true),
            ("/v1/models", true),
            ("/v1/chat/completions", true),
            ("/v1-admin/reset", false),
            ("/v1-admin", false),
            ("/v10/models", false),
            ("/", false),
            ("/admin", false),
        ] {
            assert_eq!(
                path_is_under_prefix(path, "/v1"),
                under,
                "{path} under /v1 should be {under}"
            );
        }
        // An empty prefix is the whole origin, so everything on it is under it.
        assert!(path_is_under_prefix("/anything", ""));
    }

    #[tokio::test]
    async fn only_the_two_supported_operations_are_forwarded() {
        // Port 1 has nothing listening: every row below must be refused BEFORE a
        // socket is opened, so a `provider_unreachable` here would be a failure.
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:1/v1")],
            GatewayLimits::default(),
        )
        .unwrap();

        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "DELETE",
                "/chat/completions",
                "a supported path reached by a method that is not its own",
                "unsupported_operation",
            ),
            (
                "PUT",
                "/chat/completions",
                "likewise PUT",
                "unsupported_operation",
            ),
            (
                "DELETE",
                "/models",
                "deleting a model is model management, not a chat loop",
                "unsupported_operation",
            ),
            (
                "POST",
                "/models",
                "the model list is a GET",
                "unsupported_operation",
            ),
            (
                "GET",
                "/chat/completions",
                "a completion is a POST",
                "unsupported_operation",
            ),
            (
                "POST",
                "/api/pull",
                "Ollama's model pull lives at the same loopback origin",
                "unsupported_operation",
            ),
            (
                "DELETE",
                "/api/delete",
                "so does its model delete",
                "unsupported_operation",
            ),
            (
                "GET",
                "/admin/reset",
                "an administrative route",
                "unsupported_operation",
            ),
            (
                "GET",
                "/models?limit=10",
                "neither operation takes a query, so a query is not that operation",
                "unsupported_operation",
            ),
            (
                "POST",
                "/chat//completions",
                "an empty segment is a different path",
                "unsupported_operation",
            ),
            (
                "POST",
                "/chat/%2e%2e/chat/completions",
                "an encoded dot is refused as a path, before it is classified",
                "bad_request",
            ),
            (
                "POST",
                "/chat\\completions",
                "a backslash is refused as a path",
                "bad_request",
            ),
        ];
        for (method, path, why, code) in cases {
            let err = gateway
                .fetch(json!({
                    "providerId": "local",
                    "method": method,
                    "path": path,
                    "headers": {"content-type": "application/json"},
                    "bodyBase64": base64_chat_body("test-model"),
                }))
                .await
                .expect_err(&format!("{method} {path} must be refused: {why}"));
            assert_eq!(err.code(), *code, "{method} {path} ({why}): {err}");
        }

        // And the two that ARE supported get as far as the socket, which is the
        // proof that the rows above were refused by policy and not by the
        // connection failing first.
        for (method, path, body) in [
            ("GET", "/models", None),
            (
                "POST",
                "/chat/completions",
                Some(base64_chat_body("test-model")),
            ),
        ] {
            let mut payload = json!({
                "providerId": "local",
                "method": method,
                "path": path,
            });
            if let Some(body) = body {
                payload["bodyBase64"] = json!(body);
                payload["headers"] = json!({"content-type": "application/json"});
            }
            let err = gateway
                .fetch(payload)
                .await
                .expect_err("nothing is listening on port 1");
            assert_eq!(
                err.code(),
                "provider_unreachable",
                "{method} {path} must reach the connect attempt: {err}"
            );
        }
    }

    #[tokio::test]
    async fn a_credential_or_proxy_header_is_refused_by_name() {
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:1/v1")],
            GatewayLimits::default(),
        )
        .unwrap();
        for (name, why) in [
            ("Authorization", "the gateway injects the credential"),
            ("proxy-authorization", "a credential for a proxy"),
            ("Cookie", "a credential"),
            ("x-api-key", "a credential"),
            ("api-key", "a credential"),
            ("host", "the host is the registered provider's"),
            ("Content-Length", "set from the body the gateway sends"),
            ("transfer-encoding", "framing belongs to the client"),
            ("proxy-connection", "proxying is disabled"),
            ("forwarded", "the gateway forwards for nobody"),
            ("X-Forwarded-For", "the gateway forwards for nobody"),
            ("x-forwarded-host", "the gateway forwards for nobody"),
            ("x-forwarded-proto", "the gateway forwards for nobody"),
            ("via", "the gateway forwards for nobody"),
            ("x-anything-else", "not on the allowlist"),
            ("user-agent", "not on the allowlist"),
        ] {
            let err = gateway
                .fetch(json!({
                    "providerId": "local",
                    "method": "GET",
                    "path": "/models",
                    "headers": {name: "x"},
                }))
                .await
                .expect_err(why);
            assert!(
                matches!(err, GatewayError::BadHeader { .. }),
                "{name}: {why} — got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_content_type_a_provider_operation_accepts_is_the_one_it_defines() {
        let gateway = ProviderGateway::new(
            vec![local("local", "http://127.0.0.1:1/v1")],
            GatewayLimits::default(),
        )
        .unwrap();
        for (content_type, why) in [
            ("text/plain", "a chat body is JSON"),
            ("application/x-www-form-urlencoded", "a chat body is JSON"),
            ("multipart/form-data; boundary=x", "a chat body is JSON"),
        ] {
            let err = gateway
                .fetch(json!({
                    "providerId": "local",
                    "method": "POST",
                    "path": "/chat/completions",
                    "headers": {"content-type": content_type},
                    "bodyBase64": base64_chat_body("test-model"),
                }))
                .await
                .expect_err(why);
            assert!(
                matches!(err, GatewayError::BadContentType { .. }),
                "{content_type}: {why} — got {err:?}"
            );
        }

        // A model list carries no body, so it names no content type either.
        let err = gateway
            .fetch(json!({
                "providerId": "local",
                "method": "GET",
                "path": "/models",
                "headers": {"content-type": "application/json"},
            }))
            .await
            .expect_err("a bodiless operation does not carry a content type");
        assert!(matches!(err, GatewayError::BadContentType { .. }), "{err}");
    }

    /// The registered model is the model. A body naming another one is refused,
    /// and a body naming none has the registered one injected — the two halves of
    /// "stored but never enforced".
    #[test]
    fn the_registered_model_is_enforced_and_injected() {
        let chat = ProviderOperation::ChatCompletions;

        // Named correctly: forwarded byte-for-byte.
        let exact = br#"{"model":"qwen3:8b","messages":[{"role":"user","content":"hi"}]}"#;
        assert_eq!(
            chat.check_body(exact.to_vec(), "qwen3:8b").unwrap(),
            exact.to_vec(),
            "a body that already names the right model is not rewritten"
        );

        // Named wrongly: refused, with both names in the message.
        let wrong = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
        match chat.check_body(wrong.to_vec(), "qwen3:8b") {
            Err(err @ GatewayError::ModelMismatch { .. }) => {
                assert_eq!(err.code(), "model_mismatch");
                let text = err.to_string();
                assert!(
                    text.contains("gpt-4o") && text.contains("qwen3:8b"),
                    "{text}"
                );
            }
            other => panic!("a body naming another model must be refused, got {other:?}"),
        }

        // Named not at all: injected, so the request that goes out cannot
        // disagree with the configuration that built it.
        let absent = br#"{"messages":[{"role":"user","content":"hi"}]}"#;
        let sent: Value =
            serde_json::from_slice(&chat.check_body(absent.to_vec(), "qwen3:8b").unwrap()).unwrap();
        assert_eq!(sent["model"], "qwen3:8b");
        assert_eq!(sent["messages"][0]["content"], "hi");

        // Not a string at all.
        let typed = br#"{"model":{"id":"x"},"messages":[{"role":"user","content":"hi"}]}"#;
        assert!(matches!(
            chat.check_body(typed.to_vec(), "qwen3:8b"),
            Err(GatewayError::BadBodyShape { .. })
        ));
    }

    #[test]
    fn a_chat_body_that_is_not_the_shape_the_operation_defines_is_refused() {
        let chat = ProviderOperation::ChatCompletions;
        for (body, why) in [
            (&b""[..], "a chat completion needs a body"),
            (&b"not json"[..], "not JSON"),
            (&b"[1,2,3]"[..], "not a JSON object"),
            (&br#"{"model":"m"}"#[..], "no messages"),
            (&br#"{"model":"m","messages":[]}"#[..], "no messages"),
            (
                &br#"{"model":"m","messages":"hello"}"#[..],
                "messages is not an array",
            ),
        ] {
            assert!(
                matches!(
                    chat.check_body(body.to_vec(), "m"),
                    Err(GatewayError::BadBodyShape { .. })
                ),
                "{why}: {}",
                String::from_utf8_lossy(body)
            );
        }

        // A model list carries nothing at all.
        assert!(matches!(
            ProviderOperation::ListModels.check_body(b"{}".to_vec(), "m"),
            Err(GatewayError::BadBodyShape { .. })
        ));
        assert!(ProviderOperation::ListModels
            .check_body(Vec::new(), "m")
            .unwrap()
            .is_empty());
    }

    /// An `image_url` part naming a URL is a fetch someone else performs, at an
    /// address nothing here validated. Inline `data:` is the only accepted form
    /// until OneCAD has an attachment resolver of its own.
    #[test]
    fn a_non_inline_image_source_is_refused() {
        let chat = ProviderOperation::ChatCompletions;
        for (url, why) in [
            ("https://evil.example/pixel.png", "a remote https image"),
            ("http://169.254.169.254/latest/meta-data/", "link-local"),
            ("file:///etc/passwd", "a local file the provider would read"),
            ("//evil.example/x.png", "protocol-relative"),
        ] {
            let body = json!({
                "model": "m",
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "what is this"},
                        {"type": "image_url", "image_url": {"url": url}},
                    ],
                }],
            });
            let err = chat
                .check_body(serde_json::to_vec(&body).unwrap(), "m")
                .expect_err(why);
            assert_eq!(err.code(), "remote_image_source", "{why}: {err}");
            assert!(
                err.to_string().contains("evil") || err.to_string().contains(url),
                "{err}"
            );
        }

        // Inline data is what the sidecar actually sends for an attachment, and
        // it reaches nothing.
        let inline = json!({
            "model": "m",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="},
                }],
            }],
        });
        assert!(chat
            .check_body(serde_json::to_vec(&inline).unwrap(), "m")
            .is_ok());
    }

    /// The operation table, as a table. A row added here without a policy is a
    /// row this assertion notices.
    #[test]
    fn the_operation_allowlist_is_the_two_calls_a_chat_loop_makes() {
        let rows: Vec<(&str, &str, &str)> = ProviderOperation::ALL
            .iter()
            .map(|op| (op.as_str(), op.method(), op.path()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("models.list", "GET", "/models"),
                ("chat.completions", "POST", "/chat/completions"),
            ],
            "a third provider operation is an authorization decision, not a new \
             row in an enum"
        );
    }

    /// A minimal, valid chat body for the registered model, base64 as the wire
    /// carries it.
    fn base64_chat_body(model: &str) -> String {
        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
        });
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&body).unwrap())
    }

    #[test]
    fn the_head_payload_is_the_shape_the_sidecar_reads_back() {
        let head = ResponseHead {
            status: 200,
            status_text: "OK".into(),
            headers: BTreeMap::from([(
                "content-type".to_string(),
                "text/event-stream".to_string(),
            )]),
        };
        let payload = head.to_payload();
        assert_eq!(payload["status"], 200);
        assert_eq!(payload["statusText"], "OK");
        assert_eq!(payload["headers"]["content-type"], "text/event-stream");
    }
}

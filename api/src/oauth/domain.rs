//! Règles du serveur d'autorisation (ADR-0023) : scopes, PKCE, URI de retour, métadonnées
//! d'un client. Sans accès à la base ni au réseau.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::Uri;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::iam::domain::{Availability, Permission};

/// Durée d'un jeton d'accès.
pub const ACCESS_TTL: Duration = Duration::from_secs(60 * 60);
/// Durée d'un jeton de rafraîchissement, renouvelé à chaque usage.
pub const REFRESH_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Délai laissé à l'admin pour consentir.
pub const REQUEST_TTL: Duration = Duration::from_secs(10 * 60);
/// Durée du code d'autorisation, à usage unique.
pub const CODE_TTL: Duration = Duration::from_secs(60);
/// Âge au-delà duquel un document de métadonnées est relu.
pub const METADATA_TTL: Duration = Duration::from_secs(60 * 60);
/// Client enregistré dynamiquement et jamais accordé : oublié après ce délai.
pub const DYNAMIC_CLIENT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Taille maximale d'un document de métadonnées.
pub const MAX_METADATA_BYTES: usize = 64 * 1024;

const MAX_NAME_CHARS: usize = 100;
const MAX_URI_CHARS: usize = 2048;
const MAX_REDIRECT_URIS: usize = 10;
const MAX_STATE_CHARS: usize = 512;

/// Préfixes des secrets émis : reconnaissables dans un outil de détection de fuites.
pub const ACCESS_PREFIX: &str = "kuro_at_";
pub const REFRESH_PREFIX: &str = "kuro_rt_agent_";
pub const CODE_PREFIX: &str = "kuro_code_";

/// Scopes proposés aux clients : les permissions attribuables et accordables à un agent.
pub fn supported_scopes() -> Vec<Permission> {
    Permission::ALL
        .iter()
        .copied()
        .filter(|p| p.availability() == Availability::Available && p.agent_grantable())
        .collect()
}

/// Scopes d'une demande (`scope`, séparés par des espaces). Les inconnus et les non accordables
/// sont ignorés (RFC 6749 : le serveur peut accorder moins) ; sans scope, tous ceux proposés,
/// l'admin choisissant sur la page de consentement.
pub fn requested_scopes(scope: Option<&str>) -> Vec<Permission> {
    let supported = supported_scopes();
    let mut scopes: Vec<Permission> = match scope.map(str::trim).filter(|s| !s.is_empty()) {
        None => supported,
        Some(scope) => scope
            .split_ascii_whitespace()
            .filter_map(Permission::from_key)
            .filter(|p| supported.contains(p))
            .collect(),
    };
    scopes.sort();
    scopes.dedup();
    scopes
}

/// Forme `scope` d'une liste de permissions.
pub fn scope_string(scopes: &[Permission]) -> String {
    scopes.iter().map(|p| p.key()).collect::<Vec<_>>().join(" ")
}

/// Défi PKCE `S256` : 43 caractères base64url (SHA-256 encodé).
pub fn valid_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Vérificateur PKCE (RFC 7636) : 43 à 128 caractères non réservés, dont le SHA-256 est le défi.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
    well_formed && URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge
}

/// `state` renvoyé tel quel au client : borné, sans caractère de contrôle.
pub fn valid_state(state: &str) -> bool {
    state.len() <= MAX_STATE_CHARS && !state.chars().any(char::is_control)
}

/// URI de retour acceptable : https, ou http vers la boucle locale (client natif, port libre).
/// Ni fragment, ni identifiants dans l'autorité.
pub fn redirect_uri_allowed(uri: &str) -> bool {
    if uri.len() > MAX_URI_CHARS || uri.contains('#') {
        return false;
    }
    let Ok(parsed) = uri.parse::<Uri>() else {
        return false;
    };
    let Some(authority) = parsed.authority() else {
        return false;
    };
    if authority.as_str().contains('@') {
        return false;
    }
    match parsed.scheme_str() {
        Some("https") => true,
        Some("http") => matches!(authority.host(), "localhost" | "127.0.0.1" | "[::1]"),
        _ => false,
    }
}

/// L'URI de retour présentée correspond à une URI déclarée : à l'identique, ou, pour la boucle
/// locale en http, au port près (RFC 8252 §7.3 : un client natif écoute sur un port choisi au
/// lancement, et déclare souvent son URI sans port).
pub fn redirect_uri_matches(registered: &str, presented: &str) -> bool {
    if registered == presented {
        return true;
    }
    if !redirect_uri_allowed(presented) {
        return false;
    }
    let (Ok(registered), Ok(presented)) = (registered.parse::<Uri>(), presented.parse::<Uri>())
    else {
        return false;
    };
    let loopback = |uri: &Uri| {
        uri.scheme_str() == Some("http")
            && uri
                .host()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
    };
    loopback(&registered)
        && loopback(&presented)
        && registered.host() == presented.host()
        && registered.path_and_query() == presented.path_and_query()
}

/// `client_id` d'un document de métadonnées : URL https avec un chemin, sans fragment ni
/// identifiants (spécification MCP, Client ID Metadata Documents).
pub fn metadata_url_allowed(url: &str) -> bool {
    if url.len() > MAX_URI_CHARS || url.contains('#') {
        return false;
    }
    let Ok(parsed) = url.parse::<Uri>() else {
        return false;
    };
    parsed.scheme_str() == Some("https")
        && parsed
            .authority()
            .is_some_and(|a| !a.as_str().contains('@'))
        && parsed.path().len() > 1
}

/// Adresse joignable sur Internet (lecture d'un document de métadonnées, contre la falsification
/// de requête côté serveur) : la règle commune de [`crate::validation`].
pub use crate::validation::is_public;

/// Ajoute des paramètres à une URI de retour (`?` ou `&`), encodés.
pub fn with_query(uri: &str, params: &[(&str, &str)]) -> String {
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let separator = if uri.contains('?') { '&' } else { '?' };
    format!("{uri}{separator}{query}")
}

/// Encodage d'un composant d'URL (RFC 3986, caractères non réservés gardés).
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Métadonnées déclarées par un client (document à son URL, ou enregistrement dynamique).
#[derive(Debug, Clone, Deserialize)]
pub struct ClientMetadata {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub client_uri: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default)]
    pub response_types: Option<Vec<String>>,
}

/// Client accepté : ce qui sera stocké et affiché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidClient {
    pub name: String,
    pub uri: Option<String>,
    pub redirect_uris: Vec<String>,
}

impl ClientMetadata {
    /// Client public uniquement (PKCE, sans secret), code d'autorisation, URI de retour sûres.
    pub fn validate(self) -> Result<ValidClient, &'static str> {
        if self
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|m| m != "none")
        {
            return Err("only public clients are supported (token_endpoint_auth_method: none)");
        }
        if self
            .grant_types
            .as_ref()
            .is_some_and(|g| !g.iter().any(|g| g == "authorization_code"))
        {
            return Err("grant_types must include authorization_code");
        }
        if self
            .response_types
            .as_ref()
            .is_some_and(|r| !r.iter().any(|r| r == "code"))
        {
            return Err("response_types must include code");
        }
        if self.redirect_uris.is_empty() || self.redirect_uris.len() > MAX_REDIRECT_URIS {
            return Err("between 1 and 10 redirect_uris are required");
        }
        if !self.redirect_uris.iter().all(|u| redirect_uri_allowed(u)) {
            return Err("redirect_uris must be https, or http on the loopback, without fragment");
        }
        let uri = self.client_uri.filter(|u| {
            u.len() <= MAX_URI_CHARS
                && u.parse::<Uri>()
                    .is_ok_and(|u| u.scheme_str() == Some("https"))
        });
        let name: String = self
            .client_name
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME_CHARS)
            .collect();
        let name = name.trim();
        if name.is_empty() {
            return Err("client_name is required");
        }
        Ok(ValidClient {
            name: name.to_owned(),
            uri,
            redirect_uris: self.redirect_uris,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_and_administrative_permissions_are_never_granted() {
        let supported = supported_scopes();
        for permission in [
            Permission::IamRolesAssign,
            Permission::SystemSettingsUpdate,
            Permission::NodesDelete,
            Permission::ServersDelete,
            Permission::ServersBackupsRestore,
        ] {
            assert!(!supported.contains(&permission), "{permission:?}");
        }
        assert!(supported.contains(&Permission::ServersPower));
        assert!(supported.contains(&Permission::PrefabsManage));
        // Réservée : pas encore attribuable.
        assert!(!supported.contains(&Permission::ServersTransfer));
    }

    #[test]
    fn requested_scopes_ignore_unknown_and_forbidden_ones() {
        assert_eq!(
            requested_scopes(Some(
                "servers.power servers.delete nope servers.read servers.power"
            )),
            vec![Permission::ServersRead, Permission::ServersPower]
        );
        assert_eq!(requested_scopes(None), supported_scopes());
        assert_eq!(requested_scopes(Some("  ")), supported_scopes());
    }

    #[test]
    fn pkce_s256_follows_rfc_7636() {
        // Exemple de l'annexe B de la RFC 7636.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(valid_challenge(challenge));
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXl",
            challenge
        ));
        assert!(!pkce_matches("short", challenge));
    }

    #[test]
    fn redirect_uris_are_https_or_loopback() {
        assert!(redirect_uri_allowed(
            "https://claude.ai/api/mcp/auth_callback"
        ));
        assert!(redirect_uri_allowed("http://127.0.0.1:33418/callback"));
        assert!(redirect_uri_allowed("http://localhost:6274/oauth/callback"));
        assert!(!redirect_uri_allowed("http://evil.example/callback"));
        assert!(!redirect_uri_allowed("https://a.example/cb#frag"));
        assert!(!redirect_uri_allowed("https://user@a.example/cb"));
        assert!(!redirect_uri_allowed("javascript:alert(1)"));
        assert!(!redirect_uri_allowed("myapp:/callback"));
    }

    #[test]
    fn a_loopback_redirect_uri_matches_on_any_port() {
        // Claude Code déclare ses URI sans port et revient sur un port libre.
        assert!(redirect_uri_matches(
            "http://localhost/callback",
            "http://localhost:51234/callback"
        ));
        assert!(redirect_uri_matches(
            "http://127.0.0.1:33418/callback",
            "http://127.0.0.1:40000/callback"
        ));
        assert!(redirect_uri_matches(
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/api/mcp/auth_callback"
        ));
        // Ni autre hôte, ni autre chemin, ni https au port près.
        assert!(!redirect_uri_matches(
            "http://localhost/callback",
            "http://127.0.0.1:51234/callback"
        ));
        assert!(!redirect_uri_matches(
            "http://localhost/callback",
            "http://localhost:51234/other"
        ));
        assert!(!redirect_uri_matches(
            "https://claude.ai/cb",
            "https://claude.ai:8443/cb"
        ));
        assert!(!redirect_uri_matches(
            "http://localhost/callback",
            "http://localhost:1/callback#x"
        ));
    }

    #[test]
    fn metadata_urls_need_https_and_a_path() {
        assert!(metadata_url_allowed(
            "https://app.example/oauth/client.json"
        ));
        assert!(!metadata_url_allowed("https://app.example"));
        assert!(!metadata_url_allowed("https://app.example/"));
        assert!(!metadata_url_allowed("http://app.example/client.json"));
        assert!(!metadata_url_allowed("https://u:p@app.example/client.json"));
    }

    #[test]
    fn only_public_addresses_are_fetched() {
        for ip in ["93.184.215.14", "2606:4700::6810:85e5"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn redirect_parameters_are_encoded() {
        assert_eq!(
            with_query("https://a.example/cb", &[("code", "x y"), ("state", "a&b")]),
            "https://a.example/cb?code=x%20y&state=a%26b"
        );
        assert_eq!(
            with_query("https://a.example/cb?k=1", &[("error", "access_denied")]),
            "https://a.example/cb?k=1&error=access_denied"
        );
    }

    #[test]
    fn client_metadata_is_public_with_safe_redirects() {
        let metadata = |extra: serde_json::Value| -> ClientMetadata {
            let mut base = serde_json::json!({
                "client_name": "  Claude\u{7}  ",
                "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"],
            });
            base.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(base).unwrap()
        };
        let valid = metadata(serde_json::json!({})).validate().unwrap();
        assert_eq!(valid.name, "Claude");
        assert!(
            metadata(serde_json::json!({ "token_endpoint_auth_method": "client_secret_basic" }))
                .validate()
                .is_err()
        );
        assert!(
            metadata(serde_json::json!({ "redirect_uris": ["http://evil.example/cb"] }))
                .validate()
                .is_err()
        );
        assert!(
            metadata(serde_json::json!({ "client_name": "" }))
                .validate()
                .is_err()
        );
    }
}

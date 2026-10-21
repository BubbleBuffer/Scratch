//! The hub as a confidential OIDC client with PKCE: discovery (cached),
//! the authorize redirect, and the code exchange with ID token checks
//! (signature through JWKS, `iss`, `aud`, `exp`, `nonce`), all done by the
//! `openidconnect` crate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreRevocableToken, CoreRevocationErrorResponse,
    CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    reqwest, AdditionalClaims, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, StandardErrorResponse,
    StandardTokenResponse, TokenResponse,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::OidcConfig;

/// The extra claim the hub reads: Authentik's `groups`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupsClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
}

impl AdditionalClaims for GroupsClaims {}

type IdTokenFields = openidconnect::IdTokenFields<
    GroupsClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type HubTokenResponse = StandardTokenResponse<IdTokenFields, CoreTokenType>;
pub(crate) type HubClient = openidconnect::Client<
    GroupsClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    HubTokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// How long discovery metadata and the JWKS are reused.
pub(crate) const DISCOVERY_TTL: Duration = Duration::from_secs(600);

/// A started login, kept until the callback.
pub struct Started {
    pub authorize_url: String,
    pub state: String,
    pub nonce: Nonce,
    pub verifier: PkceCodeVerifier,
}

/// The signed-in user, from a verified ID token.
#[derive(Debug, Clone)]
pub struct User {
    pub sub: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

pub struct Oidc {
    cfg: OidcConfig,
    redirect: RedirectUrl,
    http: reqwest::Client,
    cached: Mutex<Option<(Instant, Arc<HubClient>)>>,
}

impl Oidc {
    pub fn new(cfg: OidcConfig, public_url: &str) -> Result<Oidc, String> {
        let redirect = RedirectUrl::new(format!(
            "{}/auth/callback",
            public_url.trim_end_matches('/')
        ))
        .map_err(|e| format!("public_url: {e}"))?;
        // No redirects: the openidconnect docs require it (SSRF).
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Oidc {
            cfg,
            redirect,
            http,
            cached: Mutex::new(None),
        })
    }

    async fn client(&self) -> Result<Arc<HubClient>, String> {
        let mut cached = self.cached.lock().await;
        if let Some((at, c)) = cached.as_ref() {
            if at.elapsed() < DISCOVERY_TTL {
                return Ok(c.clone());
            }
        }
        let issuer = IssuerUrl::new(self.cfg.issuer.clone()).map_err(|e| format!("issuer: {e}"))?;
        let meta = CoreProviderMetadata::discover_async(issuer, &self.http)
            .await
            .map_err(|e| format!("discovery: {}", error_chain(&e)))?;
        let client = HubClient::from_provider_metadata(
            meta,
            ClientId::new(self.cfg.client_id.clone()),
            Some(ClientSecret::new(self.cfg.client_secret.clone())),
        )
        .set_redirect_uri(self.redirect.clone());
        let client = Arc::new(client);
        *cached = Some((Instant::now(), client.clone()));
        Ok(client)
    }

    /// State, nonce, PKCE verifier and the issuer's authorize URL.
    pub async fn start(&self) -> Result<Started, String> {
        let client = self.client().await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut req = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );
        for s in self.cfg.scopes() {
            // openidconnect always adds `openid` itself.
            if s != "openid" {
                req = req.add_scope(Scope::new(s));
            }
        }
        let (url, state, nonce) = req.set_pkce_challenge(challenge).url();
        Ok(Started {
            authorize_url: url.to_string(),
            state: state.secret().clone(),
            nonce,
            verifier,
        })
    }

    /// Exchanges `code` (client secret plus PKCE verifier) and verifies
    /// the ID token.
    pub async fn finish(
        &self,
        code: String,
        verifier: PkceCodeVerifier,
        nonce: &Nonce,
    ) -> Result<User, String> {
        let client = self.client().await?;
        let resp = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| format!("token endpoint: {e}"))?
            .set_pkce_verifier(verifier)
            .request_async(&self.http)
            .await
            .map_err(|e| format!("code exchange: {}", error_chain(&e)))?;
        let id_token = resp
            .id_token()
            .ok_or_else(|| "the token response has no ID token".to_string())?;
        let claims = id_token
            .claims(&client.id_token_verifier(), nonce)
            .map_err(|e| format!("ID token: {e}"))?;
        Ok(User {
            sub: claims.subject().as_str().to_string(),
            email: claims.email().map(|e| e.as_str().to_string()),
            groups: claims
                .additional_claims()
                .groups
                .clone()
                .unwrap_or_default(),
        })
    }

    /// Whether `user` may sign in: listed by `sub` or email (ignoring
    /// case), or in a listed group. With both lists empty, nobody may.
    pub fn allowed(&self, user: &User) -> bool {
        allowed(&self.cfg.allowed_users, &self.cfg.allowed_groups, user)
    }
}

/// Shared login and MCP allow-list semantics: exact sub/group, case-insensitive email.
pub(crate) fn allowed(users: &[String], groups: &[String], user: &User) -> bool {
    users.iter().any(|a| {
        a == &user.sub
            || user
                .email
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case(a))
    }) || groups.iter().any(|g| user.groups.iter().any(|ug| ug == g))
}

pub(crate) fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(": ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_allow_list_matches_subject_email_or_group_and_defaults_to_nobody() {
        let user = User {
            sub: "subject".into(),
            email: Some("ann@example.com".into()),
            groups: vec!["notes-users".into()],
        };
        assert!(!allowed(&[], &[], &user));
        for name in ["subject", "ANN@example.com"] {
            assert!(allowed(&[name.into()], &[], &user));
        }
        assert!(allowed(&[], &["notes-users".into()], &user));
        assert!(!allowed(
            &["Subject".into()],
            &["Notes-users".into()],
            &user
        ));
        let without_email = User {
            email: None,
            ..user
        };
        assert!(allowed(&["subject".into()], &[], &without_email));
        assert!(!allowed(&["ann@example.com".into()], &[], &without_email));
    }
}

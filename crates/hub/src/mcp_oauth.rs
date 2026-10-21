//! Opt-in OAuth resource server for `/mcp`, independent of browser login.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::extract::State;
use axum::http::HeaderValue;
use axum::Json;
use openidconnect::core::{
    CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata,
};
use openidconnect::{
    reqwest, AccessToken, ClientId, IdToken, IssuerUrl, Nonce, UserInfoClaims, UserInfoError,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::config::{McpOAuthConfig, OidcConfig};
use crate::oidc::{allowed, error_chain, GroupsClaims, HubClient, User, DISCOVERY_TTL};
use crate::AppState;

type AccessJwt = IdToken<
    GroupsClaims,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;

const USERINFO_CACHE_CAPACITY: usize = 1024;

struct CachedUser {
    token_hash: String,
    expires_at: SystemTime,
    user: User,
}

#[derive(Debug)]
pub(crate) enum AuthError {
    InvalidToken,
    Forbidden,
    Unavailable,
}

pub(crate) struct McpOAuth {
    cfg: McpOAuthConfig,
    resource: String,
    challenge: HeaderValue,
    invalid_challenge: HeaderValue,
    agent_metadata_url: String,
    allowed_users: Vec<String>,
    allowed_groups: Vec<String>,
    http: reqwest::Client,
    cached: Mutex<Option<(Instant, Arc<HubClient>)>>,
    users: Mutex<VecDeque<CachedUser>>,
}

impl McpOAuth {
    pub(crate) fn new(
        cfg: McpOAuthConfig,
        public_url: &str,
        allow_list: &OidcConfig,
    ) -> Result<Self, String> {
        let base = public_url.trim_end_matches('/');
        let metadata_url = format!("{base}/.well-known/oauth-protected-resource/mcp");
        let challenge = format!(
            "Bearer resource_metadata=\"{}\"",
            metadata_url.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let invalid_challenge =
            HeaderValue::from_str(&format!("{challenge}, error=\"invalid_token\""))
                .map_err(|e| format!("public_url metadata header: {e}"))?;
        let challenge = HeaderValue::from_str(&challenge)
            .map_err(|e| format!("public_url metadata header: {e}"))?;
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            cfg,
            resource: format!("{base}/mcp"),
            challenge,
            invalid_challenge,
            agent_metadata_url: format!("{metadata_url}/agent"),
            allowed_users: allow_list.allowed_users.clone(),
            allowed_groups: allow_list.allowed_groups.clone(),
            http,
            cached: Mutex::new(None),
            users: Mutex::new(VecDeque::new()),
        })
    }

    async fn client(&self) -> Result<Arc<HubClient>, AuthError> {
        let mut cached = self.cached.lock().await;
        if let Some((at, client)) = cached.as_ref() {
            if at.elapsed() < DISCOVERY_TTL {
                return Ok(client.clone());
            }
        }
        let discover = async {
            let issuer =
                IssuerUrl::new(self.cfg.issuer.clone()).map_err(|e| format!("issuer: {e}"))?;
            let meta = CoreProviderMetadata::discover_async(issuer, &self.http)
                .await
                .map_err(|e| format!("discovery: {}", error_chain(&e)))?;
            Ok::<_, String>(HubClient::from_provider_metadata(
                meta,
                ClientId::new(self.cfg.client_id.clone()),
                None,
            ))
        };
        let client = discover.await.map_err(|error| {
            tracing::warn!(%error, "MCP OAuth discovery unavailable");
            AuthError::Unavailable
        })?;
        let client = Arc::new(client);
        *cached = Some((Instant::now(), client.clone()));
        Ok(client)
    }

    pub(crate) fn challenge(&self, rejected: bool) -> HeaderValue {
        if rejected {
            self.invalid_challenge.clone()
        } else {
            self.challenge.clone()
        }
    }

    pub(crate) async fn authenticate(&self, token: &str) -> Result<User, AuthError> {
        // Parsing happens before discovery, so garbage credentials never cause a fetch.
        let jwt: AccessJwt = token.parse().map_err(|_| AuthError::InvalidToken)?;
        let client = self.client().await?;
        let resource = &self.resource;
        let agent_resource = format!("{resource}/agent");
        let verifier = client
            .id_token_verifier()
            // Discovery may advertise more algorithms than the verifier's RS256 default.
            .set_allowed_algs([CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256])
            // RFC 8707 may add the connector URL alongside the required client
            // ID. Treat both connector resources as aliases on BOTH endpoints:
            // switching URLs never binds an otherwise valid token to a route.
            .set_other_audience_verifier_fn(move |aud| {
                aud.as_str() == resource
                    || aud.as_str() == agent_resource
                    || aud
                        .as_str()
                        .strip_prefix(&agent_resource)
                        .is_some_and(|suffix| suffix.starts_with('?') && !suffix.contains('#'))
            });
        let claims = jwt
            .claims(&verifier, |_: Option<&Nonce>| Ok(()))
            .map_err(|_| AuthError::InvalidToken)?;
        let expires_at: SystemTime = claims.expiration().into();
        let mut user = User {
            sub: claims.subject().as_str().to_owned(),
            email: claims.email().map(|e| e.as_str().to_owned()),
            groups: claims
                .additional_claims()
                .groups
                .clone()
                .unwrap_or_default(),
        };
        if allowed(&self.allowed_users, &[], &user) {
            return Ok(user);
        }
        let needs_userinfo = claims.additional_claims().groups.is_none()
            || (user.email.is_none() && !self.allowed_users.is_empty());
        if needs_userinfo {
            let hash = vault_core::sha256_hex(token.as_bytes());
            // Serialize misses so concurrent requests for one token fetch userinfo once.
            let mut users = self.users.lock().await;
            let now = SystemTime::now();
            if now >= expires_at {
                return Err(AuthError::InvalidToken);
            }
            if let Some(cached) = users
                .iter()
                .find(|u| u.token_hash == hash && u.expires_at > now)
            {
                user = cached.user.clone();
            } else {
                let request = client
                    .user_info(
                        AccessToken::new(token.to_owned()),
                        Some(claims.subject().clone()),
                    )
                    .map_err(|_| {
                        tracing::warn!("MCP OAuth userinfo endpoint unavailable");
                        AuthError::Unavailable
                    })?;
                let info: UserInfoClaims<GroupsClaims, CoreGenderClaim> = request
                    .request_async(&self.http)
                    .await
                    .map_err(|error| match error {
                        UserInfoError::ClaimsVerification(_) => AuthError::InvalidToken,
                        _ => {
                            // UserInfoError may contain the response body: never log it.
                            tracing::warn!("MCP OAuth userinfo unavailable");
                            AuthError::Unavailable
                        }
                    })?;
                user = User {
                    sub: info.subject().as_str().to_owned(),
                    email: info.email().map(|e| e.as_str().to_owned()),
                    groups: info.additional_claims().groups.clone().unwrap_or_default(),
                };
                insert_user(
                    &mut users,
                    hash,
                    expires_at,
                    user.clone(),
                    SystemTime::now(),
                );
            }
            // Userinfo may take long enough for the verified token to expire.
            if SystemTime::now() >= expires_at {
                return Err(AuthError::InvalidToken);
            }
        }
        if allowed(&self.allowed_users, &self.allowed_groups, &user) {
            Ok(user)
        } else {
            Err(AuthError::Forbidden)
        }
    }

    pub(crate) fn agent_challenge(&self, rejected: bool, query: Option<&str>) -> HeaderValue {
        let url = with_query(&self.agent_metadata_url, query);
        let value = format!(
            "Bearer resource_metadata=\"{}\"{}",
            url.replace('\\', "\\\\").replace('"', "\\\""),
            if rejected {
                ", error=\"invalid_token\""
            } else {
                ""
            }
        );
        // URI queries cannot contain raw controls; the base URL was checked at startup.
        HeaderValue::from_str(&value).expect("validated URI in OAuth metadata header")
    }

    pub(crate) fn agent_metadata(&self, query: Option<&str>) -> Value {
        json!({
            "resource": with_query(&format!("{}/agent", self.resource), query),
            "authorization_servers": [self.cfg.issuer],
            "scopes_supported": ["openid", "email", "profile", "offline_access"],
            "bearer_methods_supported": ["header"],
        })
    }
}

fn with_query(base: &str, query: Option<&str>) -> String {
    match query {
        Some(query) => format!("{base}?{query}"),
        None => base.to_owned(),
    }
}

fn insert_user(
    users: &mut VecDeque<CachedUser>,
    token_hash: String,
    expires_at: SystemTime,
    user: User,
    now: SystemTime,
) {
    users.retain(|u| u.expires_at > now);
    if expires_at <= now {
        return;
    }
    while users.len() >= USERINFO_CACHE_CAPACITY {
        users.pop_front();
    }
    users.push_back(CachedUser {
        token_hash,
        expires_at,
        user,
    });
}

pub(crate) async fn metadata(State(state): State<Arc<AppState>>) -> Json<Value> {
    let oauth = state
        .mcp_oauth
        .as_ref()
        .expect("OAuth metadata route enabled");
    Json(json!({
        "resource": oauth.resource,
        "authorization_servers": [oauth.cfg.issuer],
        "scopes_supported": ["openid", "email", "profile", "offline_access"],
        "bearer_methods_supported": ["header"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_cache_prunes_expired_and_evicts_oldest() {
        let now = SystemTime::now();
        let user = User {
            sub: "test".into(),
            email: None,
            groups: vec![],
        };
        let mut users = VecDeque::new();
        let expiry = now + Duration::from_secs(300);
        insert_user(&mut users, "expired".into(), now, user.clone(), now);
        assert!(users.is_empty());
        insert_user(
            &mut users,
            "soon".into(),
            now + Duration::from_secs(1),
            user.clone(),
            now,
        );
        for n in 0..USERINFO_CACHE_CAPACITY {
            insert_user(
                &mut users,
                n.to_string(),
                expiry,
                user.clone(),
                now + Duration::from_secs(2),
            );
        }
        assert_eq!(users.len(), USERINFO_CACHE_CAPACITY);
        assert_eq!(users.front().unwrap().token_hash, "0");
        insert_user(
            &mut users,
            "new".into(),
            expiry,
            user,
            now + Duration::from_secs(2),
        );
        assert_eq!(users.len(), USERINFO_CACHE_CAPACITY);
        assert_eq!(users.front().unwrap().token_hash, "1");
        assert_eq!(users.back().unwrap().token_hash, "new");
    }
}

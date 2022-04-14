use crate::{
    agent::{
        client::{expires, Client, LoginContext},
        InnerConfig, OAuth2Error,
    },
    config::openid,
    context::{Authentication, OAuth2Context},
};
use async_trait::async_trait;
use gloo_utils::window;
use oauth2::TokenResponse;
use openidconnect::{
    core::{
        CoreAuthDisplay, CoreAuthenticationFlow, CoreClaimName, CoreClaimType, CoreClient,
        CoreClientAuthMethod, CoreGenderClaim, CoreGrantType, CoreJsonWebKey, CoreJsonWebKeyType,
        CoreJsonWebKeyUse, CoreJweContentEncryptionAlgorithm, CoreJweKeyManagementAlgorithm,
        CoreJwsSigningAlgorithm, CoreResponseMode, CoreResponseType, CoreSubjectIdentifierType,
        CoreTokenResponse,
    },
    reqwest::async_http_client,
    AuthorizationCode, ClientId, CsrfToken, EmptyAdditionalClaims, EmptyAdditionalProviderMetadata,
    IdTokenClaims, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, ProviderMetadata,
    RedirectUrl, RefreshToken, Scope,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, rc::Rc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenIdLoginState {
    pub pkce_verifier: String,
    pub nonce: String,
}

#[derive(Clone, Debug)]
pub struct OpenIdClient {
    client: openidconnect::core::CoreClient,
    end_session_url: Option<Url>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdditionalProviderMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_session_endpoint: Option<Url>,
}

impl openidconnect::AdditionalProviderMetadata for AdditionalProviderMetadata {}

pub type ExtendedProviderMetadata = ProviderMetadata<
    AdditionalProviderMetadata,
    CoreAuthDisplay,
    CoreClientAuthMethod,
    CoreClaimName,
    CoreClaimType,
    CoreGrantType,
    CoreJweContentEncryptionAlgorithm,
    CoreJweKeyManagementAlgorithm,
    CoreJwsSigningAlgorithm,
    CoreJsonWebKeyType,
    CoreJsonWebKeyUse,
    CoreJsonWebKey,
    CoreResponseMode,
    CoreResponseType,
    CoreSubjectIdentifierType,
>;

#[async_trait(? Send)]
impl Client for OpenIdClient {
    type TokenResponse = CoreTokenResponse;
    type Configuration = openid::Config;
    type LoginState = OpenIdLoginState;
    type SessionState = Rc<IdTokenClaims<EmptyAdditionalClaims, CoreGenderClaim>>;

    async fn from_config(config: Self::Configuration) -> Result<Self, OAuth2Error> {
        let issuer = IssuerUrl::new(config.issuer_url)
            .map_err(|err| OAuth2Error::Configuration(format!("invalid issuer URL: {err}")))?;

        let metadata = ExtendedProviderMetadata::discover_async(issuer, async_http_client)
            .await
            .map_err(|err| {
                OAuth2Error::Configuration(format!("Failed to discover client: {err}"))
            })?;

        let end_session_url = config
            .end_session_url
            .map(|url| Url::parse(&url))
            .transpose()
            .map_err(|err| {
                OAuth2Error::Configuration(format!("Unable to parse end_session_url: {err}"))
            })?
            .or_else(|| metadata.additional_metadata().end_session_endpoint.clone());

        let client =
            CoreClient::from_provider_metadata(metadata, ClientId::new(config.client_id), None);

        Ok(Self {
            client,
            end_session_url,
        })
    }

    fn set_redirect_uri(mut self, url: Url) -> Self {
        self.client = self.client.set_redirect_uri(RedirectUrl::from_url(url));
        self
    }

    fn make_login_context(
        &self,
        config: &InnerConfig,
        redirect_url: Url,
    ) -> Result<LoginContext<Self::LoginState>, OAuth2Error> {
        let client = self
            .client
            .clone()
            .set_redirect_uri(RedirectUrl::from_url(redirect_url));

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        let mut req = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );

        for scope in &config.scopes {
            req = req.add_scope(Scope::new(scope.clone()));
        }

        let (url, state, nonce) = req.set_pkce_challenge(pkce_challenge).url();

        Ok(LoginContext {
            url,
            csrf_token: state.secret().clone(),
            state: OpenIdLoginState {
                pkce_verifier: pkce_verifier.secret().clone(),
                nonce: nonce.secret().clone(),
            },
        })
    }

    async fn exchange_code(
        &self,
        code: String,
        state: Self::LoginState,
    ) -> Result<(OAuth2Context, Self::SessionState), OAuth2Error> {
        let pkce_verifier = PkceCodeVerifier::new(state.pkce_verifier);

        let result = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(pkce_verifier)
            .request_async(async_http_client)
            .await
            .map_err(|err| OAuth2Error::LoginResult(format!("failed to exchange code: {err}")))?;

        log::debug!("Exchange code result: {:?}", result);

        let id_token = result.extra_fields().id_token().ok_or_else(|| {
            OAuth2Error::LoginResult("Server did not return an ID token".to_string())
        })?;

        let claims = Rc::new(
            id_token
                .clone()
                .into_claims(&self.client.id_token_verifier(), &Nonce::new(state.nonce))
                .map_err(|err| {
                    OAuth2Error::LoginResult(format!("failed to verify ID token: {err}"))
                })?,
        );

        Ok((
            OAuth2Context::Authenticated(Authentication {
                access_token: result.access_token().secret().to_string(),
                refresh_token: result.refresh_token().map(|t| t.secret().to_string()),
                expires: expires(result.expires_in()),
                claims: Some(claims.clone()),
            }),
            claims,
        ))
    }

    async fn exchange_refresh_token(
        &self,
        refresh_token: String,
        session_state: Self::SessionState,
    ) -> Result<(OAuth2Context, Self::SessionState), OAuth2Error> {
        let result = self
            .client
            .exchange_refresh_token(&RefreshToken::new(refresh_token))
            .request_async(async_http_client)
            .await
            .map_err(|err| {
                OAuth2Error::LoginResult(format!("failed to exchange refresh token: {err}"))
            })?;

        Ok((
            OAuth2Context::Authenticated(Authentication {
                access_token: result.access_token().secret().to_string(),
                refresh_token: result.refresh_token().map(|t| t.secret().to_string()),
                expires: expires(result.expires_in()),
                claims: Some(session_state.clone()),
            }),
            session_state,
        ))
    }

    fn logout(&self) {
        if let Some(url) = &self.end_session_url {
            let mut url = url.clone();
            if let Ok(current) = window().location().href() {
                url.query_pairs_mut().append_pair("redirect_uri", &current);
            }
            window().location().set_href(url.as_str()).ok();
        }
    }
}

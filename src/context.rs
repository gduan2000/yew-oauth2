use yew::{context::ContextHandle, html::Scope, prelude::*};

#[cfg(feature = "openid")]
pub type Claims = openidconnect::IdTokenClaims<
    openidconnect::EmptyAdditionalClaims,
    openidconnect::core::CoreGenderClaim,
>;

#[derive(Clone, Debug, PartialEq)]
pub enum OAuth2Context {
    /// The agent is not initialized yet.
    NotInitialized,
    /// Not authenticated.
    NotAuthenticated {
        /// Reason why it is not authenticated.
        reason: Reason,
    },
    /// Session is authenticated.
    Authenticated {
        /// The access token
        access_token: String,
        /// An optional refresh token
        refresh_token: Option<String>,
        /// OpenID claims
        #[cfg(feature = "openid")]
        claims: Option<std::rc::Rc<Claims>>,
        expires: Option<u64>,
    },
    /// Something failed.
    Failed(String),
}

impl OAuth2Context {
    /// Get the access token, if the context is [`OAuth2Context::Authenticated`]
    pub fn access_token(&self) -> Option<String> {
        match self {
            Self::Authenticated { access_token, .. } => Some(access_token.clone()),
            _ => None,
        }
    }

    /// Get the claims, if the context is [`OAuth2Context::Authenticated`]
    #[cfg(feature = "openid")]
    pub fn claims(&self) -> Option<&Claims> {
        match self {
            Self::Authenticated {
                claims: Some(claims),
                ..
            } => Some(claims),
            _ => None,
        }
    }
}

/// The reason why the context is un-authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NewSession,
    Expired,
    Logout,
}

/// Helper to get an unzipped version of the context.
pub trait UnzippedWith {
    fn unzipped_with(
        &self,
        callback: Callback<OAuth2Context>,
    ) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>);
}

/// Helper to get an unzipped version of the context.
pub trait Unzipped {
    type Message;

    fn unzipped<F>(&self, f: F) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>)
    where
        F: Fn(OAuth2Context) -> Self::Message + 'static;
}

impl<C> UnzippedWith for Context<C>
where
    C: Component,
{
    fn unzipped_with(
        &self,
        callback: Callback<OAuth2Context>,
    ) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>) {
        self.link().unzipped_with(callback)
    }
}

impl<C> UnzippedWith for Scope<C>
where
    C: Component,
{
    fn unzipped_with(
        &self,
        callback: Callback<OAuth2Context>,
    ) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>) {
        match self.context(callback) {
            Some((auth, handle)) => (Some(auth), Some(handle)),
            None => (None, None),
        }
    }
}

impl<C> Unzipped for Context<C>
where
    C: Component,
{
    type Message = C::Message;

    fn unzipped<F>(&self, f: F) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>)
    where
        F: Fn(OAuth2Context) -> Self::Message + 'static,
    {
        self.link().unzipped(f)
    }
}

impl<C> Unzipped for Scope<C>
where
    C: Component,
{
    type Message = C::Message;

    fn unzipped<F>(&self, f: F) -> (Option<OAuth2Context>, Option<ContextHandle<OAuth2Context>>)
    where
        F: Fn(OAuth2Context) -> Self::Message + 'static,
    {
        self.unzipped_with(self.callback(f))
    }
}

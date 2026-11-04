use crate::StoreResult;
use crate::crypt::decrypt;
use crate::domain::oauth::{OauthVerifier, OauthVerifierData, SsoSignInData};
use crate::domain::org_security_audit_log::{SecurityEventType, SecurityRequestContext};
use crate::domain::sso::SsoConnection;
use crate::domain::users::LoginUserResponse;
use crate::errors::StoreError;
use crate::repositories::OrganizationsInterface;
use crate::repositories::oauth::get_verifier;
use crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs;
use crate::repositories::users::{finalize_login, validate_domain};
use crate::store::Store;
use async_trait::async_trait;
use common_domain::ids::{BaseId, OrganizationId, OrganizationInviteId, SsoConnectionId, UserId};
use common_eventbus::Event;
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::query::organizations::verified_domain_owner;
use diesel_models::sso::OrganizationSsoConnectionRow;
use diesel_models::users::{UserRow, UserRowNew};
use error_stack::{Report, ResultExt, bail};
use meteroid_oauth::model::OidcSsoConfig;
use meteroid_oauth::service::build_oidc_sso_service;
use secrecy::{ExposeSecret, SecretString};
use std::str::FromStr;

#[async_trait]
pub trait SsoInterface {
    /// Enabled connection owning `email`'s domain, if any (login discovery).
    /// Only a domain verified in `organization_domain` resolves.
    async fn find_sso_connection_for_email(
        &self,
        email: &str,
    ) -> StoreResult<Option<SsoConnection>>;

    /// Build the IdP authorization URL for a connection, persisting the PKCE /
    /// CSRF verifier the callback needs. `callback_url` is the SSO redirect URI
    /// registered with the IdP (owned by the REST layer, which knows the
    /// externally-reachable base URL).
    async fn sso_authorize_url(
        &self,
        connection_id: SsoConnectionId,
        invite_key: Option<String>,
        callback_url: String,
    ) -> StoreResult<SecretString>;

    /// Complete an SSO callback: exchange the code, resolve (or JIT-provision)
    /// the member, and issue a session (or MFA challenge).
    async fn sso_signin(
        &self,
        code: SecretString,
        state: SecretString,
        callback_url: String,
        request_context: SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse>;
}

#[async_trait]
impl SsoInterface for Store {
    async fn find_sso_connection_for_email(
        &self,
        email: &str,
    ) -> StoreResult<Option<SsoConnection>> {
        let Some(domain) = email_domain(email) else {
            return Ok(None);
        };
        let mut conn = self.get_conn().await?;

        // The domain must be verified (globally unique owner) and that owner's
        // enabled connection must list it. An unverified claim never routes.
        let Some(owner) = verified_domain_owner(&mut conn, &domain)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };
        let Some(sso) = OrganizationSsoConnectionRow::find_enabled_by_org(&mut conn, owner)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };
        if sso.domains.iter().flatten().any(|d| d == &domain) {
            Ok(Some(sso.into()))
        } else {
            Ok(None)
        }
    }

    async fn sso_authorize_url(
        &self,
        connection_id: SsoConnectionId,
        invite_key: Option<String>,
        callback_url: String,
    ) -> StoreResult<SecretString> {
        let mut conn = self.get_conn().await?;
        let row = self
            .load_enabled_connection(&mut conn, connection_id)
            .await?;
        let service =
            build_oidc_sso_service(self.oauth.egress(), self.oidc_config(&row, callback_url)?)
                .change_context(StoreError::OauthError("invalid SSO configuration".into()))?;

        let auth_url = service.authorize_url();
        let verifier = OauthVerifier {
            csrf_token: auth_url.csrf_token,
            pkce_verifier: auth_url.pkce_verifier,
            data: OauthVerifierData::SsoSignIn(SsoSignInData {
                connection_id,
                invite_key,
            }),
        };
        verifier
            .to_row(&self.settings.crypt_key)?
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(auth_url.url)
    }

    async fn sso_signin(
        &self,
        code: SecretString,
        state: SecretString,
        callback_url: String,
        request_context: SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse> {
        let verifier = get_verifier(self, state).await?;
        let OauthVerifierData::SsoSignIn(SsoSignInData {
            connection_id,
            invite_key,
        }) = verifier.data
        else {
            bail!(StoreError::OauthError("invalid SSO verifier data".into()));
        };

        let row = {
            let mut conn = self.get_conn().await?;
            self.load_enabled_connection(&mut conn, connection_id)
                .await?
        };
        let organization_id = row.organization_id;
        let jit = row.jit_provisioning;
        // Domains this IdP is configured to serve.
        let domains: Vec<String> = row.domains.iter().flatten().cloned().collect();

        let service =
            build_oidc_sso_service(self.oauth.egress(), self.oidc_config(&row, callback_url)?)
                .change_context(StoreError::OauthError("invalid SSO configuration".into()))?;

        let tokens = service
            .exchange_code(code, verifier.pkce_verifier)
            .await
            .change_context(StoreError::OauthError("failed to exchange SSO code".into()))?;
        let oauth_user = service
            .get_user_info(tokens.access_token)
            .await
            .change_context(StoreError::OauthError("failed to fetch SSO user".into()))?;

        let email = oauth_user.email.to_lowercase();
        validate_domain(&email, &self.settings.domains_whitelist)?;

        // The IdP is only authoritative for domains this org has both configured
        // on the connection AND proven ownership of. This rejects an asserted
        // email outside them, so a misconfigured or hostile IdP can't mint
        // sessions for arbitrary addresses.
        let email_domain = email_domain(&email).ok_or_else(|| {
            Report::new(StoreError::LoginError(
                "SSO returned an invalid email".into(),
            ))
        })?;
        if !domains.iter().any(|d| d == &email_domain) {
            bail!(StoreError::LoginError(
                "this email domain is not configured for the organization's SSO".into()
            ));
        }
        if !is_domain_verified_for_org(self, organization_id, &email_domain).await? {
            bail!(StoreError::LoginError(
                "this email domain is not verified for the organization".into()
            ));
        }

        let mut conn = self.get_conn().await?;
        let existing = UserRow::find_by_email(&mut conn, email.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let (user_id, response) = match existing {
            Some(user) => {
                let uid = user.id;
                let is_member = OrganizationMemberRow::find_by_user_and_org(
                    &mut conn,
                    uid.as_uuid(),
                    organization_id,
                )
                .await
                .is_ok();
                if !is_member {
                    // Prefer the invite (keeps its role); else JIT if enabled.
                    let joined = match &invite_key {
                        Some(key) => {
                            drop(conn);
                            let joined = self
                                .accept_sso_invite(uid, key, request_context.clone())
                                .await;
                            conn = self.get_conn().await?;
                            joined
                        }
                        None => false,
                    };
                    if !joined {
                        if !jit {
                            bail!(StoreError::LoginError(
                                "your account is not a member of this organization".into()
                            ));
                        }
                        add_membership(&mut conn, uid, organization_id).await?;
                    }
                }
                let response = finalize_login(self, &mut conn, user).await?;
                drop(conn);
                (uid, response)
            }
            None if invite_key.is_some() => {
                // New account joining via an invite link — create + consume the
                // invite so the member lands with the invited role.
                drop(conn);
                let req = crate::domain::users::RegisterUserRequestInternal {
                    email: email.clone(),
                    password: None,
                    invite_key: invite_key.map(SecretString::from),
                    signup_source: None,
                };
                let res = crate::repositories::users::register_user_internal(
                    self,
                    req,
                    request_context.clone(),
                )
                .await?;
                let uid = res.user.id;
                (
                    uid,
                    LoginUserResponse::Authenticated {
                        token: res.token,
                        user: res.user,
                    },
                )
            }
            None => {
                if !jit {
                    bail!(StoreError::LoginError(
                        "no account exists for this email and SSO provisioning is disabled".into()
                    ));
                }
                let new_user = UserRowNew {
                    id: UserId::new(),
                    email: email.clone(),
                    password_hash: None,
                };
                new_user
                    .insert(&mut conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                add_membership(&mut conn, new_user.id, organization_id).await?;
                let _ = self
                    .eventbus
                    .publish(Event::user_created(None, new_user.id, None))
                    .await;

                let user = UserRow::find_by_id(&mut conn, new_user.id.as_uuid())
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let response = finalize_login(self, &mut conn, user).await?;
                drop(conn);
                (new_user.id, response)
            }
        };

        if matches!(response, LoginUserResponse::Authenticated { .. }) {
            record_auth_event_for_user_orgs(
                self,
                user_id,
                &email,
                SecurityEventType::AuthSsoLoginSucceeded,
                Some(serde_json::json!({
                    "provider": "oidc",
                    "connection_id": connection_id.to_string(),
                })),
                request_context,
            )
            .await;
        }

        Ok(response)
    }
}

impl Store {
    fn oidc_config(
        &self,
        row: &OrganizationSsoConnectionRow,
        callback_url: String,
    ) -> StoreResult<OidcSsoConfig> {
        let client_secret = decrypt(&self.settings.crypt_key, &row.client_secret).change_context(
            StoreError::CryptError("failed to decrypt client secret".into()),
        )?;
        Ok(OidcSsoConfig {
            client_id: row.client_id.clone(),
            client_secret: client_secret.expose_secret().to_string(),
            authorize_url: row.authorize_url.clone(),
            token_url: row.token_url.clone(),
            userinfo_url: row.userinfo_url.clone(),
            callback_url,
            scopes: row.scopes.split_whitespace().map(str::to_string).collect(),
        })
    }

    /// Consume an invite as part of an SSO sign-in, returning whether the user
    /// joined through it. A malformed key or an invite that doesn't match this
    /// user (wrong email, already used, expired) yields `false`, letting the
    /// caller fall back to JIT provisioning.
    async fn accept_sso_invite(
        &self,
        user_id: UserId,
        invite_key: &str,
        request_context: SecurityRequestContext,
    ) -> bool {
        let Ok(invite_id) = OrganizationInviteId::from_str(invite_key) else {
            return false;
        };
        self.accept_invite(user_id, invite_id, request_context)
            .await
            .is_ok()
    }

    async fn load_enabled_connection(
        &self,
        conn: &mut crate::store::PgConn,
        connection_id: SsoConnectionId,
    ) -> StoreResult<OrganizationSsoConnectionRow> {
        let row = OrganizationSsoConnectionRow::find_by_id(conn, connection_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::LoginError("SSO connection not found".into()))
            })?;
        if !row.enabled {
            bail!(StoreError::LoginError(
                "SSO is not enabled for this organization".into()
            ));
        }
        Ok(row)
    }
}

async fn add_membership(
    conn: &mut crate::store::PgConn,
    user_id: UserId,
    organization_id: OrganizationId,
) -> StoreResult<()> {
    OrganizationMemberRow {
        user_id,
        organization_id,
        role: OrganizationUserRole::Member,
        role_id: None,
    }
    .insert(conn)
    .await
    .map(|_| ())
    .map_err(Into::<Report<StoreError>>::into)
}

fn email_domain(email: &str) -> Option<String> {
    email
        .rsplit('@')
        .next()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty() && d.len() < email.len())
}

/// Whether `domain` is verified *and owned by* `organization_id`: SSO is only
/// authoritative for domains the organization has proven it owns.
async fn is_domain_verified_for_org(
    store: &Store,
    organization_id: OrganizationId,
    domain: &str,
) -> StoreResult<bool> {
    let Some(domain) = normalize_domain(domain) else {
        return Ok(false);
    };
    let mut conn = store.get_conn().await?;
    let owner = verified_domain_owner(&mut conn, &domain)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(owner == Some(organization_id))
}

pub fn normalize_domain(domain: &str) -> Option<String> {
    let d = domain.trim().trim_start_matches('@').to_lowercase();
    (!d.is_empty() && d.contains('.') && !d.contains(' ')).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::{email_domain, normalize_domain};

    #[test]
    fn normalize_domain_cleans_input() {
        assert_eq!(
            normalize_domain("  @Acme.COM ").as_deref(),
            Some("acme.com")
        );
        assert_eq!(normalize_domain("no-dot"), None);
        assert_eq!(normalize_domain("has space.com"), None);
        assert_eq!(normalize_domain(""), None);
    }

    #[test]
    fn email_domain_is_extracted_lowercased() {
        assert_eq!(email_domain("Alice@Acme.COM").as_deref(), Some("acme.com"));
        assert_eq!(email_domain("no-at-sign"), None);
        assert_eq!(email_domain("").as_deref(), None);
    }
}

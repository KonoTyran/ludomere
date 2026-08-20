use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::capabilities::{GogCapability, GogCapabilityRegistry};

#[derive(Debug, Serialize)]
struct CapabilityAudit {
    checks: Vec<CapabilityCheck>,
}

#[derive(Debug, Serialize)]
struct CapabilityCheck {
    capability: GogCapability,
    ok: bool,
    item_count: Option<usize>,
    error: Option<String>,
}

pub fn run() -> Result<()> {
    let token =
        crate::auth::load_saved_token()?.context("sign in to GOG before running the audit")?;
    let registry = GogCapabilityRegistry::load()?;
    let client = crate::gog::client()?;
    let mut checks = Vec::new();

    let friends = check(&mut checks, GogCapability::FriendsRead, || {
        super::friends::list(&client, &token)
    });
    check(&mut checks, GogCapability::FriendRequests, || {
        super::friends::invitation_count(&client, &token)
    });
    if registry.permits_read(GogCapability::PresenceRead) {
        let friend_ids = friends
            .as_ref()
            .map(|friends| {
                friends
                    .iter()
                    .map(|friend| friend.user_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        check(&mut checks, GogCapability::PresenceRead, || {
            super::presence::statuses(&client, &token, &friend_ids)
        });
    }

    let product_id = crate::auth::fetch_owned_product_ids(&token)?
        .into_iter()
        .next();
    if let Some(product_id) = product_id {
        check(&mut checks, GogCapability::AchievementsRead, || {
            super::gameplay::achievements(&client, &token, product_id, &token.user_id)
        });
        check(&mut checks, GogCapability::SessionsRead, || {
            super::gameplay::session_count(&client, &token, product_id, &token.user_id)
        });
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&CapabilityAudit { checks })?
    );
    Ok(())
}

pub fn validate_write(arguments: &[String]) -> Result<()> {
    let value = argument_after(arguments, "--validate-gog-write")
        .context("--validate-gog-write requires a capability name")?;
    let capability = value.parse::<GogCapability>()?;
    let expected_account =
        argument_after(arguments, "--account").context("write validation requires --account")?;
    if !arguments
        .iter()
        .any(|argument| argument == "--confirm-live-write")
    {
        bail!("write validation requires --confirm-live-write");
    }
    let token = crate::auth::load_saved_token()?
        .context("sign in to the designated GOG test account before validating writes")?;
    anyhow::ensure!(
        token.user_id == expected_account,
        "the authenticated GOG account does not match --account"
    );
    let registry = GogCapabilityRegistry::load()?;
    anyhow::ensure!(
        registry.permits_write(capability),
        "{value} has not been enabled for live write validation"
    );
    bail!("{value} has no registered live write validator")
}

fn argument_after<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
}

fn check<T>(
    checks: &mut Vec<CapabilityCheck>,
    capability: GogCapability,
    operation: impl FnOnce() -> Result<T>,
) -> Option<T>
where
    T: AuditCount,
{
    match operation() {
        Ok(value) => {
            checks.push(CapabilityCheck {
                capability,
                ok: true,
                item_count: Some(value.audit_count()),
                error: None,
            });
            Some(value)
        }
        Err(error) => {
            checks.push(CapabilityCheck {
                capability,
                ok: false,
                item_count: None,
                error: Some(sanitize_error(&error)),
            });
            None
        }
    }
}

trait AuditCount {
    fn audit_count(&self) -> usize;
}

impl<T> AuditCount for Vec<T> {
    fn audit_count(&self) -> usize {
        self.len()
    }
}

impl AuditCount for usize {
    fn audit_count(&self) -> usize {
        *self
    }
}

fn sanitize_error(error: &anyhow::Error) -> String {
    let message = format!("{error:#}");
    if message.contains("http") {
        "GOG endpoint request failed".into()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_errors_do_not_expose_urls() {
        let error = anyhow::anyhow!("request failed for https://example.invalid/?token=secret");
        assert_eq!(sanitize_error(&error), "GOG endpoint request failed");
    }

    #[test]
    fn arguments_are_read_without_accepting_a_missing_value() {
        let arguments = vec!["ludomere".into(), "--account".into(), "42".into()];
        assert_eq!(argument_after(&arguments, "--account"), Some("42"));
        assert_eq!(argument_after(&arguments, "--validate-gog-write"), None);
    }
}

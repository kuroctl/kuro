use std::fmt;

/// Adresse e-mail normalisée (minuscules, sans espaces autour). Validation volontairement
/// simple : la seule preuve qu'une adresse existe est d'y recevoir un message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid email address")]
pub struct InvalidEmail;

impl Email {
    pub fn parse(value: &str) -> Result<Self, InvalidEmail> {
        let email = value.trim().to_lowercase();
        let valid = email.len() <= 254
            && !email.chars().any(|c| c.is_whitespace() || c.is_control())
            && email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty() && domain.contains('.') && !domain.contains('@')
            });
        if valid {
            Ok(Self(email))
        } else {
            Err(InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

use crate::client::{LdapClient, LdapEntry, LdapOptions, SearchScope};
use crate::{LdapError, filter};

#[derive(Debug, Clone, Default)]
pub struct LdapUser {
    pub dn: String,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

impl LdapUser {
    pub fn claims(&self) -> Vec<(String, String)> {
        let mut claims = vec![("sub".to_owned(), self.dn.clone())];

        if !self.username.is_empty() {
            claims.push(("name".to_owned(), self.username.clone()));
        }
        if let Some(email) = &self.email {
            claims.push(("email".to_owned(), email.clone()));
        }
        for group in &self.groups {
            claims.push(("role".to_owned(), group.clone()));
        }

        claims
    }
}

#[derive(Clone)]
pub struct LdapAuthenticator {
    options: LdapOptions,
}

impl std::fmt::Debug for LdapAuthenticator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LdapAuthenticator")
            .field("options", &self.options)
            .finish()
    }
}

impl LdapAuthenticator {
    pub fn new(options: LdapOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &LdapOptions {
        &self.options
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<LdapUser, LdapError> {
        if username.is_empty() {
            return Err(LdapError::NoSuchUser);
        }
        if password.is_empty() {
            return Err(LdapError::InvalidCredentials);
        }
        if username.contains('\0') || password.contains('\0') {
            return Err(LdapError::InvalidCredentials);
        }

        let (dn, entry) = self.find_user(username)?;

        let mut user_client = LdapClient::connect(&self.options)?;
        user_client.bind(&dn, password)?;
        let _ = user_client.unbind();

        let mut groups = entry
            .values(&self.options.group_attribute)
            .iter()
            .map(|group_dn| common_name(group_dn).unwrap_or_else(|| group_dn.clone()))
            .collect::<Vec<_>>();

        if let Some(group_base) = &self.options.group_base_dn {
            groups.extend(self.search_groups(group_base, &dn)?);
        }
        groups.sort();
        groups.dedup();

        Ok(LdapUser {
            dn,
            username: username.to_owned(),
            display_name: entry
                .get("displayName")
                .or_else(|| entry.get("cn"))
                .map(str::to_owned),
            email: entry
                .get("mail")
                .or_else(|| entry.get("userPrincipalName"))
                .map(str::to_owned),
            groups,
        })
    }

    fn find_user(&self, username: &str) -> Result<(String, LdapEntry), LdapError> {
        // The pattern branch defers the real credential check to the
        // user bind in `authenticate`; binding here with an empty password
        // would be an unauthenticated bind that most servers reject.
        if let Some(pattern) = &self.options.user_dn_pattern {
            let dn = pattern.replace("{username}", &escape_dn(username));
            return Ok((dn, LdapEntry::default()));
        }

        let mut client = LdapClient::connect(&self.options)?;
        client.bind(&self.options.bind_dn, &self.options.bind_password)?;

        // The assertion value is BER-encoded, which is binary-safe and
        // immune to filter injection, so the raw username must be used;
        // applying the string-filter escaping here would break lookups
        // for usernames that contain `*`, `(`, `)` or `\`.
        let filter = filter::equality(&self.options.user_attribute, username);
        let entries = client.search(
            &self.options.base_dn,
            SearchScope::Subtree,
            &filter,
            &["cn", "displayName", "mail", "userPrincipalName", "memberOf"],
        )?;
        let _ = client.unbind();

        let mut entries = entries.into_iter();
        let entry = entries.next().ok_or(LdapError::NoSuchUser)?;
        if entries.next().is_some() {
            return Err(LdapError::Search(format!(
                "more than one entry matches `{username}`"
            )));
        }

        let dn = entry.dn.clone();
        Ok((dn, entry))
    }

    fn search_groups(&self, group_base: &str, user_dn: &str) -> Result<Vec<String>, LdapError> {
        let mut client = LdapClient::connect(&self.options)?;
        client.bind(&self.options.bind_dn, &self.options.bind_password)?;

        let filter = filter::and(&[
            filter::equality("objectClass", &self.options.group_object_class),
            filter::equality("member", user_dn),
        ]);
        let entries = client.search(group_base, SearchScope::Subtree, &filter, &["cn"])?;
        let _ = client.unbind();

        Ok(entries
            .into_iter()
            .filter_map(|entry| entry.get("cn").map(str::to_owned))
            .collect())
    }
}

fn common_name(dn: &str) -> Option<String> {
    dn.split(',')
        .find_map(|part| {
            part.trim()
                .strip_prefix("CN=")
                .or_else(|| part.trim().strip_prefix("cn="))
        })
        .map(str::to_owned)
}

fn escape_dn(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for (index, character) in value.char_indices() {
        let is_leading = index == 0;
        let is_trailing = index + character.len_utf8() == value.len();

        match character {
            '\0' => output.push_str("\\00"),
            ',' | '+' | '"' | '\\' | '<' | '>' | ';' | '=' | '#' => {
                output.push('\\');
                output.push(character);
            }
            ' ' if is_leading || is_trailing => output.push_str("\\ "),
            character => output.push(character),
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_common_name() {
        assert_eq!(
            common_name("CN=Admins,OU=Groups,DC=example,DC=com").as_deref(),
            Some("Admins")
        );
    }

    #[test]
    fn builds_claims() {
        let user = LdapUser {
            dn: "uid=alice,dc=example,dc=com".to_owned(),
            username: "alice".to_owned(),
            email: Some("alice@example.com".to_owned()),
            groups: vec!["Admins".to_owned()],
            ..LdapUser::default()
        };

        assert!(
            user.claims()
                .iter()
                .any(|(key, value)| key == "role" && value == "Admins")
        );
    }
}

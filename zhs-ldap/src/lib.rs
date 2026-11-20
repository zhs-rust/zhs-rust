mod auth;
mod ber;
mod client;
mod error;
mod filter;

pub use auth::{LdapAuthenticator, LdapUser};
pub use client::{LdapClient, LdapEntry, LdapOptions, SearchScope, user_filter};
pub use error::LdapError;
pub use filter::{and, equality, escape_filter, or, present};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_helpers_are_public() {
        assert_eq!(escape_filter("a*b"), "a\\2ab");
        assert!(!equality("uid", "alice").is_empty());
        assert!(!present("objectClass").is_empty());
        assert!(!and(&[equality("a", "b"), equality("c", "d")]).is_empty());
        assert!(!or(&[equality("a", "b")]).is_empty());
    }
}

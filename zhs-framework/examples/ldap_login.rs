use std::env;

use zhs_framework::ldap::{LdapAuthenticator, LdapOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = env::var("LDAP_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let base_dn = env::var("LDAP_BASE_DN").unwrap_or_else(|_| "dc=example,dc=com".to_owned());
    let bind_dn = env::var("LDAP_BIND_DN").unwrap_or_default();
    let bind_password = env::var("LDAP_BIND_PASSWORD").unwrap_or_default();
    let username = env::var("LDAP_USERNAME").unwrap_or_else(|_| "alice".to_owned());
    let password = env::var("LDAP_PASSWORD").unwrap_or_default();

    let mut options = LdapOptions::new(host, 389, base_dn).user_attribute("uid");
    if !bind_dn.is_empty() {
        options = options.service_account(bind_dn, bind_password);
    }

    let user = LdapAuthenticator::new(options).authenticate(&username, &password)?;

    println!("dn: {}", user.dn);
    println!("display name: {:?}", user.display_name);
    println!("email: {:?}", user.email);
    println!("groups: {:?}", user.groups);

    for (claim_type, value) in user.claims() {
        println!("claim {claim_type} = {value}");
    }

    Ok(())
}

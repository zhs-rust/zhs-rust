mod jwt;

pub use jwt::{Claims, JwtCodec, JwtError, VerifiedClaims};

pub struct ClaimTypes;

impl ClaimTypes {
    pub const NAME_IDENTIFIER: &'static str = "sub";
    pub const NAME: &'static str = "name";
    pub const ROLE: &'static str = "role";
    pub const EMAIL: &'static str = "email";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub claim_type: String,
    pub value: String,
}

impl Claim {
    pub fn new(claim_type: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            claim_type: claim_type.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaimsIdentity {
    authentication_type: Option<String>,
    claims: Vec<Claim>,
}

impl ClaimsIdentity {
    pub fn authenticated(authentication_type: impl Into<String>) -> Self {
        Self {
            authentication_type: Some(authentication_type.into()),
            claims: Vec::new(),
        }
    }

    pub fn add_claim(&mut self, claim: Claim) -> &mut Self {
        self.claims.push(claim);
        self
    }

    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    pub fn is_authenticated(&self) -> bool {
        self.authentication_type.is_some()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaimsPrincipal {
    identities: Vec<ClaimsIdentity>,
}

impl ClaimsPrincipal {
    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn new(identity: ClaimsIdentity) -> Self {
        Self {
            identities: vec![identity],
        }
    }

    pub fn identities(&self) -> &[ClaimsIdentity] {
        &self.identities
    }

    pub fn is_authenticated(&self) -> bool {
        self.identities.iter().any(ClaimsIdentity::is_authenticated)
    }

    pub fn find_first(&self, claim_type: &str) -> Option<&Claim> {
        self.identities
            .iter()
            .flat_map(ClaimsIdentity::claims)
            .find(|claim| claim.claim_type == claim_type)
    }

    pub fn has_claim(&self, claim_type: &str, value: &str) -> bool {
        self.identities
            .iter()
            .flat_map(ClaimsIdentity::claims)
            .any(|claim| claim.claim_type == claim_type && claim.value == value)
    }

    pub fn is_in_role(&self, role: &str) -> bool {
        self.has_claim(ClaimTypes::ROLE, role) || self.has_claim("roles", role)
    }

    pub fn name(&self) -> Option<&str> {
        self.find_first(ClaimTypes::NAME_IDENTIFIER)
            .or_else(|| self.find_first(ClaimTypes::NAME))
            .map(|claim| claim.value.as_str())
    }
}

impl From<VerifiedClaims> for ClaimsPrincipal {
    fn from(claims: VerifiedClaims) -> Self {
        let mut identity = ClaimsIdentity::authenticated("Bearer");
        identity.add_claim(Claim::new(ClaimTypes::NAME_IDENTIFIER, claims.subject));

        if let Some(name) = claims.extra.get("name").and_then(zhs_json::Value::as_str) {
            identity.add_claim(Claim::new(ClaimTypes::NAME, name));
        }

        for role in claims.roles {
            identity.add_claim(Claim::new(ClaimTypes::ROLE, role));
        }

        for (key, value) in claims.extra.iter() {
            let value = match value {
                zhs_json::Value::String(value) => value.clone(),
                value => zhs_json::to_string(value),
            };
            identity.add_claim(Claim::new(key, value));
        }

        Self::new(identity)
    }
}

pub struct JwtBearer {
    codec: JwtCodec,
}

impl JwtBearer {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, JwtError> {
        Ok(Self {
            codec: JwtCodec::new(secret)?,
        })
    }

    pub fn from_codec(codec: JwtCodec) -> Self {
        Self { codec }
    }

    pub fn codec(&self) -> &JwtCodec {
        &self.codec
    }

    pub fn authenticate(&self, authorization: Option<&str>) -> Option<ClaimsPrincipal> {
        let authorization = authorization?;
        let token = authorization.strip_prefix("Bearer ")?;
        let claims = self.codec.verify(token).ok()?;

        Some(ClaimsPrincipal::from(claims))
    }
}

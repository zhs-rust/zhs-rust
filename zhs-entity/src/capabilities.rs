#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub transactions: bool,

    pub identity: bool,

    pub raw_sql: bool,

    pub raw_command: bool,

    pub pool: bool,
}

impl Capabilities {
    pub const NONE: Self = Self {
        transactions: false,
        identity: false,
        raw_sql: false,
        raw_command: false,
        pool: false,
    };

    pub const fn new() -> Self {
        Self::NONE
    }

    pub const fn with_transactions(mut self) -> Self {
        self.transactions = true;
        self
    }

    pub const fn with_identity(mut self) -> Self {
        self.identity = true;
        self
    }

    pub const fn with_raw_sql(mut self) -> Self {
        self.raw_sql = true;
        self
    }

    pub const fn with_raw_command(mut self) -> Self {
        self.raw_command = true;
        self
    }

    pub const fn with_pool(mut self) -> Self {
        self.pool = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::Capabilities;

    #[test]
    fn defaults_to_nothing() {
        assert_eq!(Capabilities::default(), Capabilities::NONE);
        assert_eq!(Capabilities::new(), Capabilities::NONE);
    }

    #[test]
    fn builds_a_snapshot_without_touching_other_flags() {
        let capabilities = Capabilities::new().with_transactions().with_pool();

        assert!(capabilities.transactions);
        assert!(capabilities.pool);
        assert!(!capabilities.identity);
        assert!(!capabilities.raw_sql);
        assert!(!capabilities.raw_command);
    }
}

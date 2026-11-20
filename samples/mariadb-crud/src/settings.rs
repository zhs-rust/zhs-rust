//! appsettings.json 配置段绑定。

use zhs_entity_mariadb::{ConnectOptions, MariaDbDatabase};
use zhs_framework::config::Configuration;
use zhs_framework::entity::Database;
use zhs_framework::http::auth::JwtCodec;
use zhs_framework::prelude::FromJson;

/// 全部应用配置。
#[derive(Debug)]
pub struct Settings {
    pub mariadb: MariaDbSettings,
    pub jwt: JwtSettings,
}

impl Settings {
    /// 从框架已加载的配置(appsettings.json + 环境变量 + 命令行)绑定。
    pub fn from(configuration: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            mariadb: configuration
                .get::<MariaDbSettings>("MariaDb")
                .ok_or("appsettings.json 缺少 MariaDb 配置段(或字段不完整)")?,
            jwt: configuration
                .get::<JwtSettings>("Jwt")
                .ok_or("appsettings.json 缺少 Jwt 配置段(或字段不完整)")?,
        })
    }
}

/// `MariaDb` 段。FromJson 精确匹配优先、未命中时大小写不敏感回退,
/// 所以 PascalCase 键(`Host`/`Port`/...)直接映射到 snake_case 字段。
#[derive(Debug, FromJson)]
pub struct MariaDbSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl MariaDbSettings {
    /// 建立 MariaDB 连接池并包装为 `Database`。
    pub fn connect(&self) -> Result<Database, zhs_framework::entity::Error> {
        let options = ConnectOptions::new(
            self.host.clone(),
            self.port,
            self.user.clone(),
            self.password.clone(),
        )
        .database(self.database.clone());

        Ok(Database::new(MariaDbDatabase::connect(&options)?))
    }
}

/// `Jwt` 段(`Secret` 必填,其余可选)。
#[derive(Debug, FromJson)]
pub struct JwtSettings {
    pub secret: String,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub lifetime_minutes: Option<u64>,
}

impl JwtSettings {
    pub fn lifetime_seconds(&self) -> u64 {
        self.lifetime_minutes.unwrap_or(120) * 60
    }

    /// 构建签发/校验共用的 codec。codec 配置的 iss/aud 在签发时
    /// 自动补进令牌,与校验端天然一致。
    pub fn codec(&self) -> Result<JwtCodec, Box<dyn std::error::Error>> {
        if self.secret.len() < 32 {
            return Err("Jwt:Secret 至少需要 32 个字符".into());
        }

        let mut codec = JwtCodec::new(&self.secret)?;
        if let Some(ref issuer) = self.issuer {
            codec = codec.with_issuer(issuer.clone());
        }
        if let Some(ref audience) = self.audience {
            codec = codec.with_audience(audience.clone());
        }

        Ok(codec.with_lifetime(self.lifetime_seconds()))
    }
}

/// 迁移工具 standalone 加载配置:文件 + `ZHS_` 环境变量 + `--key=value` 参数,
/// 优先级与 WebApplication 一致。
pub fn standalone_configuration(arguments: &[String]) -> Configuration {
    let mut configuration = Configuration::new();

    let environment = arguments
        .windows(2)
        .find(|pair| pair[0] == "--environment")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("ZHS_ENVIRONMENT").ok())
        .unwrap_or_else(|| "Development".to_owned());

    for path in [
        "appsettings.json".to_owned(),
        format!("appsettings.{environment}.json"),
    ] {
        if let Ok(loaded) = Configuration::load_file(&path) {
            configuration.merge(&loaded);
        }
    }

    configuration.add_environment("ZHS_");
    configuration.add_command_line(arguments.iter().cloned());
    configuration
}

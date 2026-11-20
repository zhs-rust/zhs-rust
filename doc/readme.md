# zhs 框架

> 自研 Rust Web 框架，对标 ASP.NET Core，核心零第三方依赖。

## 定位

- **拒绝供应链投毒**：HTTP、JSON、加密、数据库协议、异步运行时全部手写，不引入外部 crate。
- **给 C# 程序员一个友好的 Rust 入口**：保留 Controller / DI / Middleware / JWT / Configuration / HealthCheck / OpenAPI 等心智模型，同时使用 Rust 的所有权与类型系统。
- **薄 crate + 门面聚合**：每个能力独立成小 crate，`zhs-framework` 统一重导出，应用只依赖门面。

## 快速开始

```bash
cargo run -p zhs-cli -- new hello
cd hello
cargo run
```

访问：

- `http://127.0.0.1:5000/api/hello`
- `http://127.0.0.1:5000/swagger`
- `http://127.0.0.1:5000/health`

控制器示例：

```rust
use std::sync::Arc;
use zhs_framework::prelude::*;

struct AppState { greeting: String }

#[injectable]
impl AppState {
    #[inject]
    fn new() -> Self {
        Self { greeting: "Hello".to_owned() }
    }
}

struct HelloController { state: Arc<AppState> }

#[controller("/api/[controller]")]
impl HelloController {
    #[inject]
    fn new(state: Arc<AppState>) -> Self { Self { state } }

    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        Ok(format!("{} from zhs", self.state.greeting))
    }
}
```

## Crate 清单

| Crate | 说明 |
| --- | --- |
| `zhs-framework` | 门面 crate（`zhs_framework`），聚合所有子 crate |
| `zhs-cli` | `zhs new <name>` 项目生成器 |
| `zhs-types` | 通用值类型：`Guid`、`DateTimeOffset`、`Decimal` |
| `zhs-macros` | 过程宏：`#[controller]`、`#[injectable]`、`#[derive(ToJson/FromJson/FromRow)]` |
| `zhs-json` | JSON 解析与序列化 |
| `zhs-log` | 日志（含 runtime logger） |
| `zhs-http` | HTTP/1.1 服务器（内置 reactor）、路由、`HttpContext`、响应 |
| `zhs-config` | 配置系统（文件 / 环境变量 / 命令行） |
| `zhs-crypto` | 密码学原语（SHA/HMAC/PBKDF2/Base64/CSPRNG） |
| `zhs-ldap` | LDAP v3 客户端与认证器 |
| `zhs-mn` | M:N 异步运行时：调度器、线程池、定时器、看门狗 |
| `zhs-scheduler` | Quartz 风格定时任务 |
| `zhs-tui` | 终端 UI 工具箱 |
| `zhs-entity` | SQL 抽象：参数化 SQL、`FromRow`、`#[table]`/`#[column]` |
| `zhs-entity-sqlite` | SQLite 后端 |
| `zhs-entity-mariadb` | MariaDB / MySQL 后端 |
| `zhs-entity-sqlserver` | SQL Server 后端（TDS） |
| `zhs-entity-oracle` | Oracle 后端（OCI，运行时加载客户端库） |
| `zhs-entity-mongo` | MongoDB 后端（BSON / OP_MSG） |
| `zhs-entity-influx` | InfluxDB 2.x 后端（Line Protocol + Flux） |
| `zhs-entity-pi` | PI Web API 后端（只读） |

## 构建

根 `Cargo.toml` 声明了完整 workspace，一条命令构建全部：

```bash
cargo build --workspace
```

单独构建或测试某个 crate：

```bash
cargo build -p zhs-json
cargo test -p zhs-json
```

## 安全须知

部署前请务必阅读：

- **无内置 TLS**。HTTP、MariaDB、SQL Server、MongoDB、LDAP、InfluxDB、PI 全部走明文协议。SQL Server 的 TDS 登录包只是混淆，不是加密。请部署在 TLS 终止代理之后，或限制所有后端连接在可信网络内。
- **授权按路由开启**。Controller action 默认匿名，必须显式标注 `#[authorize]`。没有“全局强制认证”开关，请自行审计路由。
- **限流键基于 socket peer 地址**。`zhs-framework` 的限流中间件使用连接对端 IP，不使用 `X-Forwarded-For`。若在代理后运行，请使用 `zhs-http` 的可信代理中间件，使 peer 地址反映真实客户端。
- **CORS**：`allow_any_origin()` 永远不会与 `Access-Control-Allow-Credentials: true` 组合生效（浏览器会忽略该组合）。如需跨域携带凭证，请配置显式的 origin 白名单。
- **密码学**（SHA-1/SHA-256、HMAC、PBKDF2、密码哈希、CSPRNG）在 `zhs-crypto` 中从零实现，并通过 RFC/FIPS 测试向量验证，但未经独立审计，也未针对侧信道做额外加固（仅保证常量时间比较）。请根据威胁模型评估是否适用。

## 开发方式

AI 生成 + 人工审码。

## License

暂未选择开源许可证。

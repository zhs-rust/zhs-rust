# mariadb-crud

基于 zhs 框架的 MariaDB CRUD 示例:JWT 登录页 + 商品表格/表单,完整增删改查、
关键字搜索、分页、基于角色的授权。

## 运行

```bash
cd samples/mariadb-crud
cargo run --bin migrate   # ① 建表 + 种子账号(一次性,可重复执行)
cargo run                 # ② 启动服务
```

- 登录页: http://127.0.0.1:5000/
- 商品管理: http://127.0.0.1:5000/app.html
- Swagger UI: http://127.0.0.1:5000/swagger
- OpenAPI: http://127.0.0.1:5000/openapi/v1.json
- Health: http://127.0.0.1:5000/health

> 迁移与服务进程完全分离(对应 `dotnet ef database update` 与应用启动的关系):
> `migrate` 独立读取同一份 `appsettings.json`,服务启动不做任何 DDL。
> 登录偏慢请用 `cargo run --release`(debug 构建下 PBKDF2 校验约需数秒)。

## 演示账号

`cargo run --bin migrate` 时写入(已存在则跳过):

| 用户名 | 密码 | 角色 | 权限 |
| --- | --- | --- | --- |
| `admin` | `admin123` | admin | 增删改查 |
| `demo` | `demo123` | user | 增改查,删除返回 403 |

## 准备数据库

示例默认连接本机 MariaDB(配置在 `appsettings.json` 的 `MariaDb` 段):

```sql
CREATE DATABASE zhs_crud CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER 'zhs_crud_app'@'127.0.0.1' IDENTIFIED BY 'zhs_crud_app';
GRANT ALL PRIVILEGES ON zhs_crud.* TO 'zhs_crud_app'@'127.0.0.1';
```

按自己的环境改 `appsettings.json`,也可以用环境变量覆盖:

```powershell
$env:ZHS_MARIADB__HOST = "192.168.1.10"
$env:ZHS_MARIADB__PASSWORD = "secret"
```

> 注意:zhs 的 MariaDB 驱动只支持 `mysql_native_password` 认证(默认即此插件)。

## 端点

| 方法 | 路径 | 说明 | 授权 |
| --- | --- | --- | --- |
| POST | `/api/auth/login` | 用户名密码换 JWT | 匿名 |
| GET | `/api/auth/me` | 当前用户信息 | 登录 |
| GET | `/api/products` | 列表,`keyword` / `page` / `page_size` | 登录 |
| GET | `/api/products/{id}` | 单个商品 | 登录 |
| POST | `/api/products` | 新增,201 + location | 登录 |
| PUT | `/api/products/{id}` | 编辑 | 登录 |
| DELETE | `/api/products/{id}` | 删除,204 | **admin** |

密码使用 `zhs-crypto` 的 PBKDF2-SHA256 哈希存储;JWT 为 HS256,
密钥与有效期在 `appsettings.json` 的 `Jwt` 段(`Secret` 至少 32 字符)。

## Layout

标准分层(Controller → Service → Model),迁移是独立工具:

```text
src/
├── main.rs                 服务入口(等价 Program.cs):装配 DI 与中间件
├── lib.rs                  库根:服务进程与迁移工具共用
├── settings.rs             appsettings.json 配置段绑定(MariaDb / Jwt)
├── migrations.rs           建表脚本执行 + 种子账号(仅供 migrate 工具调用)
├── bin/
│   └── migrate.rs          迁移工具:cargo run --bin migrate
├── controllers/            HTTP 端点:只做绑定与响应映射
│   ├── auth.rs             POST /api/auth/login、GET /api/auth/me
│   └── products.rs         商品 CRUD 五个端点
├── services/               业务逻辑:校验 + SQL(Dapper 风格)
│   ├── auth.rs             认证、令牌签发
│   └── products.rs         商品 CRUD、分页、搜索
└── models/
    ├── entities.rs         FromRow 实体(products / users 行)
    └── dtos.rs             请求/响应 DTO(FromJson / ToJson)

appsettings.json            MariaDb / Jwt / Logging 配置
migrations/0001_init.sql    建表脚本(幂等,由 migrate 工具执行)
wwwroot/                    前端:登录页 + 商品管理页(表格 + 表单 + 分页)
```

## 代码里值得看的点

- **迁移与应用分离**:`src/bin/migrate.rs` 独立加载配置、执行
  `migrations/0001_init.sql`(按分号拆分逐句执行,驱动走 prepared 语句
  不支持多语句,脚本须幂等)并用 `PasswordHasher` 现算哈希写入种子账号;
  `main.rs` 启动时不碰任何 DDL。
- **三层职责**:`controllers/` 只做 HTTP 绑定与 DTO 映射,
  `services/` 持有全部校验与 SQL,`models/` 只有数据形状——
  服务层通过 `#[injectable]` 注册进 DI,控制器构造注入 `Arc<Service>`。
- **INSERT ... RETURNING**:`services/products.rs` 的 `create` 用
  MariaDB 10.5+ 的 `RETURNING` 一次拿回含自增 id 的整行,避免
  `LAST_INSERT_ID()` 在连接池下拿到别的连接的值。
- **DECIMAL 是文本**:zhs 的 MariaDB 二进制协议把 `DECIMAL` 解码成字符串,
  所以 `models/entities.rs` 里 `Product.price` 是 `String`,转 DTO 时再 parse 成 `f64`。
- **参数一律走 `params!`**:模糊搜索的 `%` 由 SQL 端 `CONCAT` 拼接,
  用户输入永不进 SQL 文本。
- **错误自动转换**:`From<zhs_entity::Error> for ApiError` 让服务层
  数据调用直接 `?` 收尾——唯一键冲突自动 409,其余自动 500
  problem+json(细节只进日志),相当于 ASP.NET Core 的异常过滤器兜底。
- **配置大小写不敏感**:`#[derive(FromJson)]` 精确匹配优先、未命中时
  大小写不敏感回退,所以 `appsettings.json` 的 PascalCase 键
  (`Host`/`Port`/...)直接映射到 snake_case 字段,无需 rename。
- **JWT 签发自洽**:`JwtCodec` 上配置的 `with_issuer`/`with_audience`
  在签发时自动补进令牌,校验端与签发端天然一致,控制器无需镜像传递。
- **角色授权**:DELETE 标注 `#[authorize(roles = "admin")]`,
  `demo` 账号调用会得到 403,前端会弹提示。

## 常见问题

- **启动报"缺少 MariaDb 配置段"**:请在 `samples/mariadb-crud` 目录下运行
  (`appsettings.json` 按相对路径读取,服务与 migrate 工具同理)。
- **请求 500 且日志提示表不存在**:先执行 `cargo run --bin migrate`。
- **中文乱码**:确保客户端以 UTF-8 发送 JSON
  (`content-type: application/json; charset=utf-8`);
  浏览器 fetch 自动满足。
- **401 反复跳登录**:JWT 过期(`Jwt:LifetimeMinutes`,默认 120 分钟)。
  codec 的 iss/aud 签发时自动补齐,不会出现两端不一致。

# zhs

> 自研的 Rust 框架，采用「瘦 crate + 门面 crate」的架构，核心代码零依赖。

## 架构

- **瘦 crate**：每个能力（JSON、日志、HTTP、调度……）都是独立、最小化的 crate。
- **门面 crate**：`zhs-framework` 聚合各瘦 crate，向上提供统一入口。
- **零依赖**：核心部分不引入第三方依赖。

## crates

| crate | 说明 |
| --- | --- |
| `zhs-framework` | 门面 crate，聚合各子模块 |
| `zhs-json` | JSON 解析与序列化 |
| `zhs-log` | 日志 |
| `zhs-http` | HTTP |
| `zhs-config` | 配置 |
| `zhs-crypto` | 加解密 |
| `zhs-ldap` | LDAP |
| `zhs-mn` | 模型/微服务基础组件 |
| `zhs-scheduler` | 调度 |
| `zhs-tui` | 终端界面 |
| `zhs-entity` | 实体抽象 |
| `zhs-entity-sqlite` / `-mariadb` / `-mongo` / `-oracle` / `-sqlserver` / `-influx` / `-pi` | 各数据库的实体实现 |

## 构建

```bash
cargo build
```

各 crate 目前为独立 crate，可进入各自目录单独构建：

```bash
cd zhs-json
cargo build
```

## 文档

见 [doc/readme.md](doc/readme.md)。

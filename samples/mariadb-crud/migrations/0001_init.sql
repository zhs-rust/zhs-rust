-- zhs mariadb-crud 示例初始化脚本
-- 程序启动时会自动执行本文件(按分号拆分逐句执行),脚本要求可重复执行。

CREATE TABLE IF NOT EXISTS users (
    id            BIGINT       NOT NULL AUTO_INCREMENT,
    username      VARCHAR(50)  NOT NULL,
    password_hash VARCHAR(200) NOT NULL,
    display_name  VARCHAR(50)  NOT NULL,
    role          VARCHAR(20)  NOT NULL DEFAULT 'user',
    created_at    DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id),
    UNIQUE KEY uq_users_username (username)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE IF NOT EXISTS products (
    id         BIGINT        NOT NULL AUTO_INCREMENT,
    name       VARCHAR(100)  NOT NULL,
    category   VARCHAR(50)   NOT NULL,
    price      DECIMAL(10,2) NOT NULL DEFAULT 0,
    quantity   INT           NOT NULL DEFAULT 0,
    created_at DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (id),
    KEY ix_products_name (name),
    KEY ix_products_category (category)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

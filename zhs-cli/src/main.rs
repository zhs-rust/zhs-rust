use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();

    match arguments.first().map(String::as_str) {
        Some("new") => new_project(&arguments[1..]),
        _ => {
            print_help();
            ExitCode::SUCCESS
        }
    }
}

fn print_help() {
    println!("zhs new <name> [--path <zhs-framework path>]");
    println!("  Creates a controller-based Web API project.");
}

fn new_project(arguments: &[String]) -> ExitCode {
    let Some(name) = arguments.first() else {
        eprintln!("missing project name");
        print_help();
        return ExitCode::FAILURE;
    };

    if name.is_empty() {
        eprintln!("project name must not be empty");
        print_help();
        return ExitCode::FAILURE;
    }

    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        eprintln!("project name may only contain letters, digits, '-' and '_'");
        return ExitCode::FAILURE;
    }

    let zhs_path = arguments
        .windows(2)
        .find(|pair| pair[0] == "--path")
        .map(|pair| PathBuf::from(&pair[1]))
        .filter(|path| !path.as_os_str().is_empty() && !path.to_string_lossy().starts_with('-'))
        .unwrap_or_else(default_zhs_path);

    let target = PathBuf::from(name);

    // The validated name has no path separators, so `target` has no parents to
    // create. `create_dir` (non-recursive) claims the directory atomically: a
    // pre-existing directory or file surfaces as `AlreadyExists` instead of
    // racing a separate `exists()` check.
    if let Err(error) = fs::create_dir(&target) {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            eprintln!("`{}` already exists", target.display());
        } else {
            eprintln!("failed to create `{}`: {error}", target.display());
        }

        return ExitCode::FAILURE;
    }

    if let Err(error) = write_project(&target, name, &zhs_path) {
        eprintln!("failed to create project: {error}");
        return ExitCode::FAILURE;
    }

    println!("created `{}`", target.display());
    println!();
    println!("  cd {name}");
    println!("  cargo run");
    println!();
    println!("  http://127.0.0.1:5000/api/hello");
    println!("  http://127.0.0.1:5000/swagger");

    ExitCode::SUCCESS
}

fn default_zhs_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zhs-cli should live inside the workspace")
        .join("zhs-framework")
}

// Escapes a value that is spliced into a generated TOML basic string, so a path
// containing a quote, a backslash or a control character cannot break out of it.
fn toml_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\u{8}' => output.push_str("\\b"),
            '\n' => output.push_str("\\n"),
            '\u{c}' => output.push_str("\\f"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < ' ' || character == '\u{7f}' => {
                output.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => output.push(character),
        }
    }

    output
}

fn write_project(target: &Path, name: &str, zhs_path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target.join("src"))?;
    fs::create_dir_all(target.join("migrations"))?;

    let zhs_path = toml_escape(&zhs_path.to_string_lossy().replace('\\', "/"));

    fs::write(
        target.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
zhs-framework = {{ path = "{zhs_path}" }}

[workspace]
"#
        ),
    )?;

    fs::write(
        target.join("src/main.rs"),
        r#"use std::sync::Arc;

use zhs_framework::prelude::*;

struct AppState {
    greeting: String,
}

#[injectable]
impl AppState {
    #[inject]
    fn new() -> Self {
        Self {
            greeting: "Hello".to_owned(),
        }
    }
}

struct HelloController {
    state: Arc<AppState>,
}

#[controller("/api/[controller]")]
impl HelloController {
    #[inject]
    fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        Ok(format!("{} from zhs", self.state.greeting))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = WebApplication::builder(std::env::args());

    builder.services.add_singleton::<AppState>();
    builder.services.add_controllers::<HelloController>();

    let mut app = builder.build()?;
    app.use_request_id();
    app.use_request_logging();
    app.map_controllers();
    app.map_health_checks("/health");
    app.map_openapi("/openapi/v1.json", "My API", "v1");
    app.use_swagger_ui("/swagger", "/openapi/v1.json");
    app.run()?;

    Ok(())
}
"#,
    )?;

    fs::write(
        target.join("appsettings.json"),
        r#"{
  "Logging": {
    "LogLevel": {
      "Default": "Information"
    }
  }
}
"#,
    )?;

    fs::write(
        target.join("appsettings.Development.json"),
        r#"{
  "Logging": {
    "LogLevel": {
      "Default": "Debug"
    }
  }
}
"#,
    )?;

    fs::write(
        target.join("migrations/0001_init.sql"),
        "-- Add your schema here. zhs does not run migrations automatically.\n",
    )?;

    fs::write(
        target.join("README.md"),
        format!(
            r#"# {name}

Generated by `zhs new`.

```bash
cargo run
```

- API: http://127.0.0.1:5000/api/hello
- Swagger UI: http://127.0.0.1:5000/swagger
- OpenAPI: http://127.0.0.1:5000/openapi/v1.json
- Health: http://127.0.0.1:5000/health

## Layout

```text
src/main.rs          Program + controllers
appsettings.json     Configuration
migrations/          SQL schema scripts (run manually)
```
"#
        ),
    )?;

    Ok(())
}

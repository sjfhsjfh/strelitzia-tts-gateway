use std::io;
use std::path::Path;

use serde_json::Value;
use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, Schema, SchemaFormat, SchemaType, Type};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityRequirement, SecurityScheme};
use utoipa::openapi::{Components, LicenseBuilder, OpenApi, RefOr};

pub const OPENAPI_FILE: &str = "openapi.json";

/// Schema-only: describes a string/binary body for the spec; runtime responses stream raw bytes.
pub struct BinaryAudio;

impl utoipa::PartialSchema for BinaryAudio {
    fn schema() -> RefOr<Schema> {
        RefOr::T(Schema::Object(
            ObjectBuilder::new()
                .schema_type(SchemaType::new(Type::String))
                .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
                .build(),
        ))
    }
}

impl utoipa::ToSchema for BinaryAudio {}

pub fn openapi() -> OpenApi {
    let (_, mut document) = crate::api::openapi_router().split_for_parts();

    document.info.title = "Strelitzia TTS Gateway".to_owned();
    document.info.version = env!("CARGO_PKG_VERSION").to_owned();
    document.info.description = Some(
        "OpenAI-compatible text-to-speech aggregation gateway with a Legado endpoint.".to_owned(),
    );
    document.info.contact = None;
    document.info.license = Some(
        LicenseBuilder::new()
            .name("MIT")
            .identifier(Some("MIT"))
            .build(),
    );

    let components = document.components.get_or_insert_with(Components::new);
    components.add_security_scheme(
        "bearer_auth",
        SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
    );

    document.security = Some(vec![SecurityRequirement::new(
        "bearer_auth",
        Vec::<String>::new(),
    )]);

    document
}

pub fn write_or_check(check: bool) -> Result<(), io::Error> {
    let document = openapi();
    let rendered = serde_json::to_string_pretty(&document)
        .map_err(|error| io::Error::other(format!("cannot serialize OpenAPI: {error}")))?;
    let path = Path::new(OPENAPI_FILE);

    if !check {
        std::fs::write(path, format!("{rendered}\n"))?;
        println!("wrote {OPENAPI_FILE}");
        return Ok(());
    }

    let existing = std::fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot read {OPENAPI_FILE}: {error}; run `strelitzia openapi` first"),
        )
    })?;
    let existing: Value = serde_json::from_str(&existing).map_err(|error| {
        io::Error::other(format!(
            "{OPENAPI_FILE} is not valid JSON: {error}; run `strelitzia openapi` to regenerate it"
        ))
    })?;
    let generated: Value = serde_json::from_str(&rendered)
        .map_err(|error| io::Error::other(format!("cannot parse generated OpenAPI: {error}")))?;

    if existing == generated {
        return Ok(());
    }

    let mut differences = Vec::new();
    let generated_paths = generated
        .get("paths")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let existing_paths = existing
        .get("paths")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for path in generated_paths.keys() {
        if !existing_paths.contains_key(path) {
            differences.push(format!("missing path `{path}`"));
        }
    }
    for path in existing_paths.keys() {
        if !generated_paths.contains_key(path) {
            differences.push(format!("unexpected path `{path}`"));
        }
    }
    if differences.is_empty() {
        differences.push("specification content differs".to_owned());
    }

    Err(io::Error::other(format!(
        "{OPENAPI_FILE} is out of date ({}); run `strelitzia openapi` to regenerate it",
        differences.join(", ")
    )))
}

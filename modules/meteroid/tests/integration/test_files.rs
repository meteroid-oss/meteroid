use crate::data::ids::TENANT_ID;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use bytes::Bytes;
use image::{DynamicImage, ImageFormat};
use meteroid::services::storage::{self, ObjectKind};
use meteroid_store::domain::enums::StoredObjectKindEnum;

/// Full write→read cycle for a business object: `put` stores the bytes and records the registry
/// row; the public `/files/v1/logo/{id}` endpoint reads them back, and its `read_of_kind` guard
/// rejects an id of any other kind. Also asserts the raw byte round-trip through the `read` facade.
#[tokio::test]
async fn test_logo_put_and_public_read() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // A tiny but valid PNG — the endpoint decodes, resizes and re-encodes it.
    let mut png = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgba8(4, 4)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let png = Bytes::from(png.into_inner());

    // Write path: store the bytes and record the registry row in one call.
    let logo = storage::put(
        setup.object_store.as_ref(),
        &setup.store,
        ObjectKind::ImageLogo,
        TENANT_ID,
        png,
    )
    .await
    .expect("put logo");

    let client = http_client_tracing::client();
    let base = &setup.config.rest_api_external_url;

    // Read path: the public logo endpoint resolves the id via the registry, reads the bytes and
    // returns a PNG.
    let resp = client
        .get(format!("{base}/files/v1/logo/{}", logo.id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(
        &body[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "logo endpoint returns a PNG"
    );

    // A non-logo object of the same id space, to exercise the kind guard.
    let batch_input = storage::put(
        setup.object_store.as_ref(),
        &setup.store,
        ObjectKind::BatchJobInput,
        TENANT_ID,
        Bytes::from_static(b"not an image"),
    )
    .await
    .expect("put batch input");

    // Kind guard: the public endpoint must not serve a non-logo id (would otherwise be usable to
    // fetch arbitrary document kinds by id).
    let resp = client
        .get(format!("{base}/files/v1/logo/{}", batch_input.id))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "logo endpoint must reject a non-logo id"
    );

    // Exact byte round-trip through the read facade (no image processing).
    let raw = storage::read(setup.object_store.as_ref(), &setup.store, batch_input.id)
        .await
        .expect("read batch input");
    assert_eq!(raw, Bytes::from_static(b"not an image"));

    // `read_of_kind` enforces the kind at the facade level too.
    assert!(
        storage::read_of_kind(
            setup.object_store.as_ref(),
            &setup.store,
            batch_input.id,
            StoredObjectKindEnum::ImageLogo,
        )
        .await
        .is_err(),
        "read_of_kind must reject the wrong kind"
    );
}

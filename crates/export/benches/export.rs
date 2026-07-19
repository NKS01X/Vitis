use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vitis_export::{
    build_config, build_manifest, compact_layer, package_image, sha256_hex, sha256_hex_bytes,
    OciLayer,
};

fn bench_compact_layer_small(c: &mut Criterion) {
    let src = tempfile::tempdir().unwrap();
    for i in 0..5 {
        std::fs::write(
            src.path().join(format!("file_{}.txt", i)),
            format!("content of file {} with some padding data here", i),
        )
        .unwrap();
    }
    c.bench_function("compact_layer_5_files", |b| {
        b.iter(|| {
            let out = tempfile::tempdir().unwrap();
            compact_layer(black_box(src.path()), out.path()).unwrap();
        })
    });
}

fn bench_compact_layer_medium(c: &mut Criterion) {
    let src = tempfile::tempdir().unwrap();
    let data = vec![0x42u8; 102_400]; // 100 KB per file
    for i in 0..50 {
        std::fs::write(src.path().join(format!("file_{}.bin", i)), &data).unwrap();
    }
    c.bench_function("compact_layer_50_files_5mb", |b| {
        b.iter(|| {
            let out = tempfile::tempdir().unwrap();
            compact_layer(black_box(src.path()), out.path()).unwrap();
        })
    });
}

fn bench_build_manifest(c: &mut Criterion) {
    c.bench_function("build_manifest_10_layers", |b| {
        b.iter(|| {
            let layers: Vec<OciLayer> = (0..10)
                .map(|i| OciLayer {
                    media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                    digest: format!("sha256:{:064x}", i),
                    size: 1024 * 1024 * (i + 1) as u64,
                })
                .collect();
            build_manifest(
                black_box(layers),
                black_box("sha256:config_digest"),
                black_box(2048),
            )
        })
    });
}

fn bench_sha256_1mb(c: &mut Criterion) {
    let data = "a".repeat(1_048_576);
    c.bench_function("sha256_1mb", |b| {
        b.iter(|| sha256_hex(black_box(&data)))
    });
}

fn bench_sha256_bytes_1mb(c: &mut Criterion) {
    let data = vec![0x55u8; 1_048_576];
    c.bench_function("sha256_bytes_1mb", |b| {
        b.iter(|| sha256_hex_bytes(black_box(&data)))
    });
}

fn bench_package_image_single_layer(c: &mut Criterion) {
    let config = build_config(
        vec!["PATH=/usr/bin".into()],
        Some("/app".into()),
        Some(vec!["bash".into()]),
        None,
    );
    let config_json = serde_json::to_vec_pretty(&config).unwrap();
    let config_digest = sha256_hex_bytes(&config_json);
    let layer_data = vec![0x42u8; 4096];
    let layer_digest = sha256_hex_bytes(&layer_data);
    let layers = vec![(layer_digest.clone(), layer_data)];
    let manifest = build_manifest(
        vec![OciLayer {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            digest: layers[0].0.clone(),
            size: layers[0].1.len() as u64,
        }],
        &config_digest,
        config_json.len() as u64,
    );
    c.bench_function("package_image_single_layer", |b| {
        b.iter(|| {
            let out = tempfile::tempdir().unwrap();
            let tar_path = out.path().join("output.tar");
            package_image(
                black_box(&config),
                black_box(&manifest),
                black_box(&layers),
                &tar_path,
            )
            .unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_compact_layer_small,
    bench_compact_layer_medium,
    bench_build_manifest,
    bench_sha256_1mb,
    bench_sha256_bytes_1mb,
    bench_package_image_single_layer,
);
criterion_main!(benches);

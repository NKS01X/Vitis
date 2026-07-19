use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vitis_cache::{CacheKey, ContentCache};

fn bench_cache_key_generation(c: &mut Criterion) {
    c.bench_function("cache_key_generation", |b| {
        b.iter(|| {
            CacheKey::new(
                black_box("RUN apt-get update && apt-get install -y gcc make cmake"),
                black_box(&["abc123".into(), "def456".into()]),
                black_box(&[b"file_contents_here".to_vec()]),
            )
        })
    });
}

fn bench_cache_store_1kb(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let data = vec![0xABu8; 1024];
    let mut i = 0u64;
    c.bench_function("cache_store_1kb", |b| {
        b.iter(|| {
            let key = CacheKey::new(
                &format!("RUN step_{}", i),
                &[],
                &[],
            );
            cache.store(&key, black_box(&data)).unwrap();
            i += 1;
        })
    });
}

fn bench_cache_hit_lookup(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let key = CacheKey::new("RUN echo cached", &[], &[]);
    cache.store(&key, b"artifact_data").unwrap();
    c.bench_function("cache_hit_lookup", |b| {
        b.iter(|| {
            assert!(cache.hit(black_box(&key)));
        })
    });
}

fn bench_cache_diff_4kb(c: &mut Criterion) {
    let a: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
    let mut b = a.clone();
    // Modify ~10% of bytes scattered throughout
    for i in (0..4096).step_by(10) {
        b[i] = b[i].wrapping_add(1);
    }
    c.bench_function("cache_diff_4kb", |bench| {
        bench.iter(|| ContentCache::diff(black_box(&a), black_box(&b)))
    });
}

fn bench_cache_diff_64kb(c: &mut Criterion) {
    let a: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let mut b = a.clone();
    for i in (0..65536).step_by(10) {
        b[i] = b[i].wrapping_add(1);
    }
    c.bench_function("cache_diff_64kb", |bench| {
        bench.iter(|| ContentCache::diff(black_box(&a), black_box(&b)))
    });
}

fn bench_cache_key_with_copy_content(c: &mut Criterion) {
    let copy_data: Vec<Vec<u8>> = (0..5)
        .map(|i| format!("file_content_{}", i).into_bytes())
        .collect();
    let copy_refs: Vec<Vec<u8>> = copy_data.clone();
    c.bench_function("cache_key_5_copy_targets", |b| {
        b.iter(|| {
            CacheKey::new(
                black_box("COPY . ."),
                black_box(&["parent_hash".into()]),
                black_box(&copy_refs),
            )
        })
    });
}

criterion_group!(
    benches,
    bench_cache_key_generation,
    bench_cache_store_1kb,
    bench_cache_hit_lookup,
    bench_cache_diff_4kb,
    bench_cache_diff_64kb,
    bench_cache_key_with_copy_content,
);
criterion_main!(benches);

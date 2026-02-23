// Neural Kernel — Scheduler Benchmarks
// Author: Mounesh Kodi — CruxLabx
// AGPL-3.0

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_scheduler_creation(c: &mut Criterion) {
    c.bench_function("scheduler_create", |b| {
        b.iter(|| {
            let config = neural_kernel::SchedulerConfig::default();
            black_box(neural_kernel::Scheduler::new(config));
        });
    });
}

criterion_group!(benches, bench_scheduler_creation);
criterion_main!(benches);

#![allow(missing_docs)]
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ini_edit::ast::{AstNode, File};
use ini_edit::editor::Editor;

const GITEA: &str = include_str!("../tests/fixtures/gitea-app.example.ini");

const SMALL: &str = "\
[server]
host = 0.0.0.0
port = 8080

[database]
url = postgres://localhost/db
timeout = 30
";

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");

    group.throughput(Throughput::Bytes(SMALL.len() as u64));
    group.bench_with_input(BenchmarkId::new("small", SMALL.len()), &SMALL, |b, src| {
        b.iter(|| ini_edit::parse(black_box(src)));
    });

    group.throughput(Throughput::Bytes(GITEA.len() as u64));
    group.bench_with_input(BenchmarkId::new("gitea", GITEA.len()), &GITEA, |b, src| {
        b.iter(|| ini_edit::parse(black_box(src)));
    });

    group.finish();
}

fn bench_round_trip(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_trip");

    group.throughput(Throughput::Bytes(GITEA.len() as u64));
    group.bench_function("gitea", |b| {
        b.iter(|| {
            let p = ini_edit::parse(black_box(GITEA));
            black_box(p.syntax().text().to_string());
        });
    });

    group.finish();
}

fn bench_ast_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_traversal");

    let parse = ini_edit::parse(GITEA);
    group.bench_function("gitea_sections_and_entries", |b| {
        b.iter(|| {
            let file = File::cast(parse.syntax()).unwrap();
            let mut count = 0u64;
            for section in file.sections() {
                for entry in section.entries() {
                    count += entry.key().map_or(0, |k| k.len() as u64);
                }
            }
            black_box(count);
        });
    });

    group.finish();
}

fn bench_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit");

    group.bench_function("set_value", |b| {
        b.iter(|| {
            let ed = Editor::new(black_box(SMALL));
            ed.section("server").set("port", "9090");
            black_box(ed.finish());
        });
    });

    group.bench_function("append_entry", |b| {
        b.iter(|| {
            let ed = Editor::new(black_box(SMALL));
            ed.section("server").append_entry("timeout", "30");
            black_box(ed.finish());
        });
    });

    group.bench_function("create_section", |b| {
        b.iter(|| {
            let ed = Editor::new(black_box(SMALL));
            ed.section("new_section").set("key", "value");
            black_box(ed.finish());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse,
    bench_round_trip,
    bench_ast_traversal,
    bench_edit,
    bench_compare_parse,
    bench_compare_edit,
);
criterion_main!(benches);

fn bench_compare_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare_parse");

    // Small input
    group.throughput(Throughput::Bytes(SMALL.len() as u64));
    group.bench_with_input(BenchmarkId::new("ini_edit", "small"), &SMALL, |b, src| {
        b.iter(|| ini_edit::parse(black_box(src)));
    });
    group.bench_with_input(BenchmarkId::new("rust_ini", "small"), &SMALL, |b, src| {
        b.iter(|| ini::Ini::load_from_str(black_box(src)));
    });
    group.bench_with_input(
        BenchmarkId::new("configparser", "small"),
        &SMALL,
        |b, src| {
            b.iter(|| {
                let mut c = configparser::ini::Ini::new();
                c.read(black_box(src.to_string()))
            });
        },
    );
    group.bench_with_input(BenchmarkId::new("ini_core", "small"), &SMALL, |b, src| {
        b.iter(|| {
            for item in ini_core::Parser::new(black_box(src)) {
                black_box(item);
            }
        });
    });

    // Large input (Gitea 129KB)
    group.throughput(Throughput::Bytes(GITEA.len() as u64));
    group.bench_with_input(BenchmarkId::new("ini_edit", "gitea"), &GITEA, |b, src| {
        b.iter(|| ini_edit::parse(black_box(src)));
    });
    group.bench_with_input(BenchmarkId::new("rust_ini", "gitea"), &GITEA, |b, src| {
        b.iter(|| ini::Ini::load_from_str(black_box(src)));
    });
    group.bench_with_input(
        BenchmarkId::new("configparser", "gitea"),
        &GITEA,
        |b, src| {
            b.iter(|| {
                let mut c = configparser::ini::Ini::new();
                c.read(black_box(src.to_string()))
            });
        },
    );
    group.bench_with_input(BenchmarkId::new("ini_core", "gitea"), &GITEA, |b, src| {
        b.iter(|| {
            for item in ini_core::Parser::new(black_box(src)) {
                black_box(item);
            }
        });
    });

    group.finish();
}

fn bench_compare_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare_edit");

    // ini_edit: parse + set + serialize
    group.bench_function("ini_edit/set_value", |b| {
        b.iter(|| {
            let ed = Editor::new(black_box(SMALL));
            ed.section("server").set("port", "9090");
            black_box(ed.finish());
        });
    });

    // rust-ini: parse + set + serialize
    group.bench_function("rust_ini/set_value", |b| {
        b.iter(|| {
            let mut ini = ini::Ini::load_from_str(black_box(SMALL)).unwrap();
            ini.set_to(Some("server"), "port".into(), "9090".into());
            let mut buf = Vec::new();
            ini.write_to(&mut buf).unwrap();
            black_box(buf);
        });
    });

    // configparser: parse + set + serialize
    group.bench_function("configparser/set_value", |b| {
        b.iter(|| {
            let mut c = configparser::ini::Ini::new();
            c.read(black_box(SMALL.to_string())).unwrap();
            c.set("server", "port", Some("9090".into()));
            black_box(c.writes());
        });
    });

    group.finish();
}

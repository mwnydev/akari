//! 数据库账号授权矩阵（`deploy/ansible/roles/postgresql/templates/grants.sql.j2`）与代码对照。
//!
//! 每个程序只用自己的数据库账号（plan 第 2、8 节）。这里从源码推出每个程序会执行哪些 SQL：程序源码里引用的
//! `forum_db` 函数（含 `use` 导入与别名），加上这些函数在 forum-db 内部再调用的函数，取其中 `sqlx` 宏的 SQL 文本
//! （`query_file*!` 读对应的 `queries/*.sql`）。然后在迁移完成的临时库上套用授权脚本，以各程序的账号对每条 SQL 做
//! `PREPARE` + `EXPLAIN EXECUTE`（不执行，但 PostgreSQL 照常检查表与列的权限）：
//!
//! - 任何一条报权限不足即失败——新查询用到没授权的表或列，CI 就挡住；
//! - 授权给某账号的每张表都必须出现在该程序的某条 SQL 里——多给的权限也挡住（最小权限）；
//! - 序列的 USAGE 只给显式调用 `nextval` 的程序（identity 列插入不需要序列权限）。
//!
//! 推导是按名字的静态分析：注释里提到的函数也算引用（只会多算，不会漏算）。需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forum_db::PgPool;
use sqlx::Executor;

/// `(程序目录, 数据库账号)`
const SERVICES: &[(&str, &str)] = &[
    ("api", "forum_api"),
    ("worker", "forum_worker"),
    ("indexer", "forum_indexer"),
    ("sponsor", "forum_sponsor"),
];

const MACROS: &[&str] = &[
    "sqlx::query!(",
    "sqlx::query_as!(",
    "sqlx::query_scalar!(",
    "sqlx::query_file!(",
    "sqlx::query_file_as!(",
    "sqlx::query_file_scalar!(",
];

fn backend() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `s` 里 `at` 位置开始的标识符。
fn ident_at(s: &str, at: usize) -> &str {
    let end = s[at..]
        .find(|c: char| !is_ident(c))
        .map_or(s.len(), |i| at + i);
    &s[at..end]
}

/// 去掉 `//` 注释（字符串里的 `//` 也会被截掉，只影响找引用，不影响 SQL：SQL 从 forum-db 源码原样取）。
fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|l| l.find("//").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 `at` 开始的第一个 Rust 字符串字面量（普通或 raw），返回其内容。
fn string_literal(s: &str, at: usize) -> String {
    let rest = &s[at..];
    let start = rest.find('"').unwrap();
    let hashes = rest[..start]
        .rfind('r')
        .filter(|&r| rest[r + 1..start].chars().all(|c| c == '#'))
        .map(|r| start - r - 1);
    let body = &rest[start + 1..];
    if let Some(h) = hashes {
        let close = format!("\"{}", "#".repeat(h));
        return body[..body.find(&close).unwrap()].to_owned();
    }
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                // 行尾续行：跳过换行与下一行开头的空白
                Some('\n') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

/// forum-db 的一个函数：它的 SQL 与它调用的其他 forum-db 函数。
#[derive(Debug, Default)]
struct DbFn {
    queries: Vec<String>,
    calls: BTreeSet<(String, String)>,
}

/// forum-db 全部模块的函数（`模块 → 函数名 → DbFn`）。函数的范围取到下一个 `fn` 为止（forum-db 没有嵌套函数）。
fn db_functions() -> BTreeMap<String, BTreeMap<String, DbFn>> {
    let root = backend().join("crates/db");
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut modules: BTreeMap<String, BTreeMap<String, DbFn>> = BTreeMap::new();
    for f in &files {
        let module = f.file_stem().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(f).unwrap();
        let src = match src.find("#[cfg(test)]") {
            Some(i) => &src[..i],
            None => &src[..],
        };
        let starts: Vec<(usize, &str)> = src
            .match_indices("fn ")
            .filter(|(i, _)| *i == 0 || !is_ident(src[..*i].chars().last().unwrap()))
            .map(|(i, _)| (i, ident_at(src, i + 3)))
            .filter(|(_, name)| !name.is_empty())
            .collect();
        let fns = modules.entry(module.clone()).or_default();
        for (k, &(start, name)) in starts.iter().enumerate() {
            let end = starts.get(k + 1).map_or(src.len(), |n| n.0);
            let body = &src[start..end];
            let entry = fns.entry(name.to_owned()).or_default();
            for m in MACROS {
                for (i, _) in body.match_indices(m) {
                    let lit = string_literal(body, i + m.len());
                    let sql = if m.contains("query_file") {
                        std::fs::read_to_string(root.join(&lit)).unwrap()
                    } else {
                        lit
                    };
                    entry.queries.push(sql);
                }
            }
            let code = strip_comments(body);
            for (i, _) in code.match_indices("crate::") {
                let m = ident_at(&code, i + 7);
                let rest = i + 7 + m.len();
                if code[rest..].starts_with("::") {
                    let f = ident_at(&code, rest + 2);
                    entry.calls.insert((m.to_owned(), f.to_owned()));
                }
            }
            // 同一模块里的直接调用 `name(`（方法调用 `.name(` 与路径 `x::name(` 除外）
            for &(_, callee) in &starts {
                for (i, _) in code.match_indices(callee) {
                    let before = code[..i].chars().last();
                    let after = &code[i + callee.len()..];
                    if after.starts_with('(')
                        && !before.is_some_and(|c| is_ident(c) || c == '.' || c == ':')
                        && !code[..i].ends_with("fn ")
                    {
                        entry.calls.insert((module.clone(), callee.to_owned()));
                    }
                }
            }
        }
    }
    modules
}

/// `use forum_db::<tree>;` 的导入：`(路径, 本地名)`。
fn use_tree(tree: &str, prefix: &[String], out: &mut Vec<(Vec<String>, String)>) {
    let tree = tree.trim();
    if let Some(inner) = tree.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let mut depth = 0;
        let mut start = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    use_tree(&inner[start..i], prefix, out);
                    start = i + 1;
                }
                _ => {}
            }
        }
        use_tree(&inner[start..], prefix, out);
        return;
    }
    if tree.is_empty() {
        return;
    }
    if let Some((head, rest)) = tree.split_once("::") {
        let mut p = prefix.to_vec();
        p.push(head.trim().to_owned());
        use_tree(rest, &p, out);
        return;
    }
    let (name, alias) = match tree.split_once(" as ") {
        Some((n, a)) => (n.trim(), a.trim()),
        None => (tree, tree),
    };
    if name == "self" {
        if let Some(last) = prefix.last() {
            let alias = if alias == "self" {
                last.as_str()
            } else {
                alias
            };
            out.push((prefix.to_vec(), alias.to_owned()));
        }
    } else {
        let mut p = prefix.to_vec();
        p.push(name.to_owned());
        out.push((p, alias.to_owned()));
    }
}

/// 一个程序源码里引用的 forum-db 函数。
fn service_calls(
    dir: &str,
    fns: &BTreeMap<String, BTreeMap<String, DbFn>>,
) -> BTreeSet<(String, String)> {
    let mut files = Vec::new();
    rust_files(&backend().join("bins").join(dir).join("src"), &mut files);
    let mut calls = BTreeSet::new();
    let known = |m: &str, f: &str| fns.get(m).is_some_and(|x| x.contains_key(f));
    for f in files {
        let code = strip_comments(&std::fs::read_to_string(&f).unwrap());
        let mut imports = Vec::new();
        for (i, _) in code.match_indices("use forum_db::") {
            let end = i + code[i..].find(';').unwrap();
            use_tree(&code[i + 14..end], &[], &mut imports);
        }
        let mut paths: Vec<(Vec<String>, String)> = imports;
        paths.push((Vec::new(), "forum_db".to_owned()));
        for (path, local) in paths {
            for (i, _) in code.match_indices(local.as_str()) {
                if code[..i]
                    .chars()
                    .last()
                    .is_some_and(|c| is_ident(c) || c == ':' || c == '.')
                {
                    continue;
                }
                let mut segs = path.clone();
                let mut pos = i + local.len();
                if pos < code.len() && is_ident(code[pos..].chars().next().unwrap()) {
                    continue;
                }
                while code[pos..].starts_with("::") {
                    let seg = ident_at(&code, pos + 2);
                    if seg.is_empty() {
                        break;
                    }
                    segs.push(seg.to_owned());
                    pos += 2 + seg.len();
                }
                if let [m, f, ..] = segs.as_slice()
                    && known(m, f)
                {
                    calls.insert((m.clone(), f.clone()));
                }
            }
        }
    }
    calls
}

/// 一个程序会执行的全部 SQL（去重）。
fn service_queries(dir: &str, fns: &BTreeMap<String, BTreeMap<String, DbFn>>) -> BTreeSet<String> {
    let mut todo: Vec<(String, String)> = service_calls(dir, fns).into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut queries = BTreeSet::new();
    while let Some(call) = todo.pop() {
        if !seen.insert(call.clone()) {
            continue;
        }
        let Some(f) = fns.get(&call.0).and_then(|m| m.get(&call.1)) else {
            continue;
        };
        queries.extend(f.queries.iter().cloned());
        todo.extend(f.calls.iter().cloned());
    }
    queries
}

/// 授权脚本（去掉 Jinja 的 `{{ ansible_managed }}` 行，脚本里不能有别的模板语法）。
fn grants_sql() -> String {
    let path = backend().join("../deploy/ansible/roles/postgresql/templates/grants.sql.j2");
    let raw = std::fs::read_to_string(path).unwrap();
    let sql: String = raw
        .lines()
        .filter(|l| !l.contains("{{"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !sql.contains("{%") && !sql.contains("{{"),
        "grants.sql.j2 must stay plain SQL"
    );
    sql
}

/// 去掉 SQL 注释后按分号切成语句。
fn statements(sql: &str) -> Vec<String> {
    sql.lines()
        .map(|l| l.find("--").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join("\n")
        .split(';')
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect()
}

/// 账号 → 被授予的 `(表, 权限)`。
type Grants = BTreeMap<String, BTreeSet<(String, String)>>;

/// 授权脚本里每个账号被 GRANT 的 `(表, 权限)`（`GRANT p1, p2 (列) ON t1, t2 TO r1, r2`；列级算作该权限），
/// 以及谁有序列 USAGE。
fn granted(sql: &str) -> (Grants, BTreeSet<String>) {
    let mut grants = Grants::new();
    let mut sequences = BTreeSet::new();
    for s in statements(sql) {
        let Some(rest) = s.strip_prefix("GRANT ") else {
            continue;
        };
        let (on, to) = rest.rsplit_once(" TO ").unwrap();
        let roles: Vec<String> = to.split(',').map(|r| r.trim().to_owned()).collect();
        let (privs, objects) = on.rsplit_once(" ON ").unwrap();
        if objects.starts_with("ALL SEQUENCES") {
            sequences.extend(roles);
            continue;
        }
        // 去掉列清单后按逗号分出权限
        let mut privs_only = String::new();
        let mut depth = 0;
        for c in privs.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                c if depth == 0 => privs_only.push(c),
                _ => {}
            }
        }
        for r in &roles {
            let set = grants.entry(r.clone()).or_default();
            for t in objects.split(',') {
                for p in privs_only.split(',') {
                    set.insert((t.trim().to_owned(), p.trim().to_owned()));
                }
            }
        }
    }
    (grants, sequences)
}

/// 空白压成一个空格、转小写（按语句关键字找表）。
fn normalized(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 某条 SQL 是否需要 `table` 上的 `privilege`：SELECT 只要提到这张表；写权限要有对应的语句
/// （`INSERT INTO t`、`UPDATE t` 或 `INSERT INTO t ... DO UPDATE`、`DELETE FROM t`）。
fn uses(sql: &str, table: &str, privilege: &str) -> bool {
    let q = normalized(sql);
    let after = |keyword: &str| mentions(&q, &format!("{keyword} {table}"));
    match privilege {
        "SELECT" => mentions(&q, table),
        "INSERT" => after("insert into"),
        "UPDATE" => after("update") || (after("insert into") && q.contains("do update")),
        "DELETE" => after("delete from"),
        other => panic!("unexpected privilege {other}"),
    }
}

/// SQL 里是否把 `table` 当作一个词出现。
fn mentions(sql: &str, table: &str) -> bool {
    sql.match_indices(table).any(|(i, _)| {
        let before = sql[..i].chars().last();
        let after = sql[i + table.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

fn param_count(sql: &str) -> usize {
    sql.match_indices('$')
        .filter_map(|(i, _)| ident_at(sql, i + 1).parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

#[test]
fn static_analysis_finds_known_calls() {
    let fns = db_functions();
    // 内部调用：take_like_digest 调 posts::by_id
    let digest = &fns["floor"]["take_like_digest"];
    assert!(
        digest
            .calls
            .contains(&("posts".to_owned(), "by_id".to_owned()))
    );
    assert!(fns["reports"]["create"].queries.len() == 2);
    let worker = service_calls("worker", &fns);
    assert!(worker.contains(&("floor".to_owned(), "take_like_digest".to_owned())));
    assert!(worker.contains(&("hot".to_owned(), "refresh".to_owned())));
    // 别名导入：forum-api 里 `use forum_db::{arbitration as arb, ...}` 之后的 `arb::owner_queue`
    let api = service_calls("api", &fns);
    assert!(api.contains(&("arbitration".to_owned(), "owner_queue".to_owned())));
    assert!(!service_calls("sponsor", &fns).contains(&("posts".to_owned(), "reply".to_owned())));

    let mut imports = Vec::new();
    use_tree("{a as b, c::{self, d}, e}", &[], &mut imports);
    let names: Vec<&str> = imports.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(names, ["b", "c", "d", "e"]);
    assert_eq!(string_literal(r##"x!(T, r#"a "b""#)"##, 0), r#"a "b""#);
    assert_eq!(string_literal("x!(\"a\\\n   b\")", 0), "ab");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn service_queries_are_granted_and_grants_are_used(db: PgPool) {
    let fns = db_functions();
    let sql = grants_sql();
    // 账号是整个服务器共用的：已存在就不再建（测试库里由超级用户切换过去，不需要 LOGIN）
    for (_, role) in SERVICES {
        db.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DO $$ BEGIN CREATE ROLE {role} NOLOGIN; EXCEPTION WHEN duplicate_object THEN NULL; END $$"
        ))))
        .await
        .unwrap();
    }
    db.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str())))
        .await
        .unwrap();
    let (granted, sequences) = granted(&sql);

    let mut failures = Vec::new();
    for (dir, role) in SERVICES {
        let queries = service_queries(dir, &fns);
        assert!(!queries.is_empty(), "{dir}: no queries found");
        let mut conn = db.acquire().await.unwrap();
        for (n, q) in queries.iter().enumerate() {
            let args = vec!["NULL"; param_count(q)].join(", ");
            let execute = if args.is_empty() {
                format!("EXPLAIN EXECUTE q{n}")
            } else {
                format!("EXPLAIN EXECUTE q{n}({args})")
            };
            let script = format!("SET ROLE {role}; PREPARE q{n} AS {q}; {execute}");
            if let Err(e) = conn
                .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(script)))
                .await
            {
                failures.push(format!("{role}: {e}\n    {}", q.trim()));
            }
            conn.execute(sqlx::raw_sql("RESET ROLE; DEALLOCATE ALL"))
                .await
                .unwrap();
        }
        // 最小权限：授权的每张表的每种权限都要用到
        for (table, privilege) in granted.get(*role).into_iter().flatten() {
            if !queries.iter().any(|q| uses(q, table, privilege)) {
                failures.push(format!(
                    "{role}: granted {privilege} on {table} but no query needs it"
                ));
            }
        }
        let nextval = queries.iter().any(|q| q.contains("nextval("));
        if nextval != sequences.contains(*role) {
            failures.push(format!(
                "{role}: sequence USAGE granted = {}, queries call nextval = {nextval}",
                sequences.contains(*role)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

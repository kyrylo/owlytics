mod common;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Fixture {
    directory: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = owlytics::auth::random_token().unwrap();
        let directory =
            std::env::temp_dir().join(format!("owlytics-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        Self {
            directory,
            child: None,
        }
    }

    fn boot(&mut self) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_owlytics"))
            .env("OWLYTICS_BIND", "127.0.0.1:0")
            .env(
                "OWLYTICS_DATABASE",
                self.directory.join("nested/analytics.sqlite"),
            )
            .env("OWLYTICS_URL", "http://localhost:3000")
            .env("OWLYTICS_PASSWORD_HASH", common::config().password_hash)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        self.child = Some(child);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line);
            let _ = sender.send(result.map(|_| line));
        });
        let line = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("startup deadline exceeded")
            .unwrap();
        line.trim()
            .strip_prefix("Owlytics listening on http://")
            .expect("missing startup address")
            .to_owned()
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn healthcheck(address: &str) {
    let mut stream =
        TcpStream::connect_timeout(&address.parse().unwrap(), Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET /up HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.ends_with("\r\n\r\nok"), "{response}");
}

#[test]
fn binary_creates_persistent_database_and_recovers_after_process_stop() {
    let mut fixture = Fixture::new();
    let first_address = fixture.boot();
    healthcheck(&first_address);
    fixture.stop();
    let database = fixture.directory.join("nested/analytics.sqlite");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(database.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TABLE persistence_probe(value TEXT); INSERT INTO persistence_probe VALUES ('survives restart');").unwrap();
    drop(connection);
    let second_address = fixture.boot();
    healthcheck(&second_address);
    fixture.stop();
    let connection = rusqlite::Connection::open(database).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM persistence_probe", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "survives restart");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn password_cli_uses_stdin_and_rejects_short_passwords() {
    fn hash(input: &str) -> std::process::Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_owlytics"))
            .arg("hash-password")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    let output = hash("a long password from stdin\n");
    assert!(output.status.success());
    let value = String::from_utf8(output.stdout).unwrap();
    assert!(owlytics::auth::validate_hash(value.trim()));
    assert!(owlytics::auth::verify_password(
        "a long password from stdin",
        value.trim()
    ));
    let output = hash("short\n");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn startup_rejects_a_newer_database_schema() {
    let fixture = Fixture::new();
    let database = fixture.directory.join("future.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 999")
        .unwrap();
    drop(connection);
    assert!(matches!(
        owlytics::db::Database::open(&database),
        Err(owlytics::db::Error::NewerSchema)
    ));
    let connection = rusqlite::Connection::open(database).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);
}

#[tokio::test]
async fn plausible_cli_previews_then_imports_and_rejects_duplicate_imports() {
    let fixture = Fixture::new();
    let path = fixture.directory.join("analytics.sqlite");
    let db = owlytics::db::Database::open(&path).unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let directory = fixture.directory.join("plausible export");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("imported_visitors_20200101_20200101.csv"),
        "date,visitors,pageviews\n2020-01-01,3,8\n",
    )
    .unwrap();
    fs::write(
        directory.join("imported_custom_props_20200101_20200101.csv"),
        "outside scope\n",
    )
    .unwrap();
    let run = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_owlytics"))
            .env("OWLYTICS_DATABASE", &path)
            .args(["import-plausible", "example.com"])
            .arg(&directory)
            .args(options)
            .output()
            .unwrap()
    };
    let preview = run(&["--dry-run"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let output = String::from_utf8(preview.stdout).unwrap();
    assert!(output.contains("Would import 3 visitors and 8 pageviews"));
    assert!(output.contains("Dry run: no imported data saved."));
    assert!(output.contains("Outside v1 scope: custom_props"));
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        0
    );
    assert!(!run(&["--unknown"]).status.success());
    let imported = run(&[]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(
        String::from_utf8(imported.stdout)
            .unwrap()
            .contains("Imported 3 visitors and 8 pageviews")
    );
    let repeated = run(&[]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already been imported"));
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT SUM(pageviews) FROM import_daily", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        8
    );
}
